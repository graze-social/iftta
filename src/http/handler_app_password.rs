//! Handles app-password management for identities via AIP integration

use axum::{
    Form,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Json, Redirect},
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tracing::{Instrument, debug, error, instrument};

/// Error type for app-password operations
#[derive(Debug)]
pub enum Error {
    Configuration(String),
    ExternalService(String),
    #[allow(dead_code)]
    Storage(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Configuration(msg) => write!(f, "Configuration error: {}", msg),
            Error::ExternalService(msg) => write!(f, "External service error: {}", msg),
            Error::Storage(msg) => write!(f, "Storage error: {}", msg),
        }
    }
}

impl std::error::Error for Error {}

type Result<T> = std::result::Result<T, Error>;

/// Form data for setting an app-password
#[derive(Debug, Deserialize)]
pub struct SetAppPasswordForm {
    pub app_password: String,
}

/// Response for app-password operations
#[derive(Debug, Serialize)]
pub struct AppPasswordResponse {
    pub success: bool,
    pub message: String,
    pub session_id: Option<String>,
}

/// AIP app-password response
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct AipAppPasswordResponse {
    client_id: String,
    did: String,
    message: String,
    timestamp: String,
}

/// Handler to set an app-password for an identity
#[instrument(skip_all, fields(handler = "set_app_password"))]
pub async fn set_app_password(
    State(context): State<crate::http::context::WebContext>,
    jar: axum_extra::extract::PrivateCookieJar,
    Form(form): Form<SetAppPasswordForm>,
) -> std::result::Result<impl IntoResponse, (StatusCode, Json<Value>)> {
    // Get session from cookie
    let session = crate::http::auth::get_session_from_cookie(&context, &jar)
        .await
        .map_err(|_| {
            (
                StatusCode::UNAUTHORIZED,
                Json(json!({
                    "error": "unauthorized",
                    "error_description": "No valid session found"
                })),
            )
        })?;

    let identity_id = session.did.clone();

    // Check if the user is allowed to set app passwords
    if !crate::http::auth::is_allowed_identity(&context.config.allowed_identities, &identity_id) {
        return Err((
            StatusCode::FORBIDDEN,
            Json(json!({
                "error": "forbidden",
                "error_description": "Features are limited to a waitlist at this time"
            })),
        ));
    }
    debug!(did = %identity_id, "Setting app-password for identity");

    // Validate app-password is not empty
    if form.app_password.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": "invalid_request",
                "error_description": "App password cannot be empty"
            })),
        ));
    }

    // Get AIP client credentials token
    // let aip_token = get_aip_client_credentials(&context).await.map_err(|e| {
    //     (
    //         StatusCode::INTERNAL_SERVER_ERROR,
    //         Json(json!({
    //             "error": "aip_auth_failed",
    //             "error_description": format!("Failed to authenticate with AIP: {}", e)
    //         })),
    //     )
    // })?;

    // Set the app-password via AIP API
    set_app_password_via_aip(&context, &session.access_token, &form.app_password)
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_GATEWAY,
                Json(json!({
                    "error": "aip_request_failed",
                    "error_description": format!("Failed to set app-password via AIP: {}", e)
                })),
            )
        })?;

    // Redirect back to dashboard page with success
    Ok(Redirect::to("/dashboard"))
}

/// Set app-password via AIP API
#[instrument(skip_all, fields(operation = "set_app_password_via_aip"))]
async fn set_app_password_via_aip(
    context: &crate::http::context::WebContext,
    aip_token: &str,
    app_password: &str,
) -> Result<()> {
    let aip_base_url = context
        .config
        .aip_base_url
        .as_ref()
        .ok_or_else(|| Error::Configuration("AIP base URL not configured".to_string()))?;

    let url = format!("https://{}/api/atprotocol/app-password", aip_base_url);

    let response = {
        let span = tracing::info_span!("aip_set_password_request",
            url = %url
        );
        context
            .http_client
            .post(&url)
            .bearer_auth(aip_token)
            .form(&[("app-password", app_password)])
            .send()
            .instrument(span)
            .await
            .map_err(|e| {
                Error::ExternalService(format!("Failed to set app-password: {}", e))
            })?
    };

    let status = response.status();
    if !status.is_success() {
        let error_body = response
            .text()
            .await
            .unwrap_or_else(|_| "Unknown error".to_string());
        error!(status = ?status, error = %error_body, "Failed to set app-password");
        return Err(Error::ExternalService(format!(
            "Failed to set app-password: {}",
            error_body
        )));
    }

    debug!("App-password set successfully via AIP");

    Ok(())
}

/// Handler to get app-password session status
#[instrument(skip_all, fields(handler = "get_app_password_status"))]
pub async fn get_app_password_status(
    State(context): State<crate::http::context::WebContext>,
    jar: axum_extra::extract::PrivateCookieJar,
) -> std::result::Result<Json<AppPasswordResponse>, (StatusCode, Json<Value>)> {
    // Get session from cookie
    let session = crate::http::auth::get_session_from_cookie(&context, &jar)
        .await
        .map_err(|_| {
            (
                StatusCode::UNAUTHORIZED,
                Json(json!({
                    "error": "unauthorized",
                    "error_description": "No valid session found"
                })),
            )
        })?;

    let identity_id = session.did;

    // Look for existing app-password session for this identity
    let session = context
        .session_storage()
        .get_session_by_did(&identity_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "error": "storage_failed",
                    "error_description": format!("Failed to query sessions: {}", e)
                })),
            )
        })?;

    match session {
        Some(app_pwd_session) if app_pwd_session.is_app_password() => {
            let now = Utc::now();
            if app_pwd_session.access_token_expires_at > now {
                Ok(Json(AppPasswordResponse {
                    success: true,
                    message: "App password session is active".to_string(),
                    session_id: Some(app_pwd_session.session_id),
                }))
            } else {
                Ok(Json(AppPasswordResponse {
                    success: false,
                    message: "App password session has expired".to_string(),
                    session_id: None,
                }))
            }
        }
        Some(_) => {
            // Found a session but it's not an app-password session
            Ok(Json(AppPasswordResponse {
                success: false,
                message: "No app password session found".to_string(),
                session_id: None,
            }))
        }
        None => Ok(Json(AppPasswordResponse {
            success: false,
            message: "No app password session found".to_string(),
            session_id: None,
        })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_app_password_form_validation() {
        let form = SetAppPasswordForm {
            app_password: "test-password-123".to_string(),
        };
        assert!(!form.app_password.is_empty());

        let empty_form = SetAppPasswordForm {
            app_password: "".to_string(),
        };
        assert!(empty_form.app_password.trim().is_empty());
    }

    #[test]
    fn test_app_password_response() {
        let response = AppPasswordResponse {
            success: true,
            message: "Test message".to_string(),
            session_id: Some("session-123".to_string()),
        };

        assert!(response.success);
        assert_eq!(response.message, "Test message");
        assert_eq!(response.session_id, Some("session-123".to_string()));
    }
}
