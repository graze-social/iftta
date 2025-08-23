//! AIP (AT Protocol Identity Provider) utility functions

use crate::errors::AuthError;
use reqwest::StatusCode;
use tracing::{debug, instrument};

/// Check if an app-password is set for the current AIP OAuth client
///
/// Returns:
/// - `Ok(true)` if app-password is set (HTTP 204)
/// - `Ok(false)` if app-password is not set (HTTP 404)
/// - `Err` for other errors
#[instrument(skip_all, fields(operation = "check_app_password_exists"))]
pub async fn check_app_password_exists(
    http_client: &reqwest::Client,
    aip_base_url: &str,
    aip_access_token: &str,
) -> Result<bool, AuthError> {
    // Handle both hostname-only and full URL formats
    let url = if aip_base_url.starts_with("http://") || aip_base_url.starts_with("https://") {
        format!("{}/api/atprotocol/app-password", aip_base_url)
    } else {
        format!("https://{}/api/atprotocol/app-password", aip_base_url)
    };

    debug!(?url, "Checking app-password existence at AIP");

    let response = http_client
        .get(&url)
        .bearer_auth(aip_access_token)
        .send()
        .await
        .map_err(|e| AuthError::OAuthOperationFailed {
            operation: "check_app_password".to_string(),
            details: e.to_string(),
        })?;

    match response.status() {
        StatusCode::NO_CONTENT => {
            debug!("App-password exists (HTTP 204)");
            Ok(true)
        }
        StatusCode::NOT_FOUND => {
            debug!("App-password not found (HTTP 404)");
            Ok(false)
        }
        status => {
            let error_body = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            debug!(status = ?status, error = %error_body, "Unexpected response from AIP");
            Err(AuthError::OAuthOperationFailed {
                operation: "check_app_password".to_string(),
                details: format!("Unexpected response from AIP: {} - {}", status, error_body),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{bearer_token, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn test_check_app_password_exists_returns_true_on_204() {
        let mock_server = MockServer::start().await;
        let http_client = reqwest::Client::new();

        Mock::given(method("GET"))
            .and(path("/api/atprotocol/app-password"))
            .and(bearer_token("test-token"))
            .respond_with(ResponseTemplate::new(204))
            .mount(&mock_server)
            .await;

        let result = check_app_password_exists(&http_client, &mock_server.uri(), "test-token")
            .await
            .unwrap();

        assert!(result);
    }

    #[tokio::test]
    async fn test_check_app_password_exists_returns_false_on_404() {
        let mock_server = MockServer::start().await;
        let http_client = reqwest::Client::new();

        Mock::given(method("GET"))
            .and(path("/api/atprotocol/app-password"))
            .and(bearer_token("test-token"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&mock_server)
            .await;

        let result = check_app_password_exists(&http_client, &mock_server.uri(), "test-token")
            .await
            .unwrap();

        assert!(!result);
    }

    #[tokio::test]
    async fn test_check_app_password_exists_returns_error_on_other_status() {
        let mock_server = MockServer::start().await;
        let http_client = reqwest::Client::new();

        Mock::given(method("GET"))
            .and(path("/api/atprotocol/app-password"))
            .and(bearer_token("test-token"))
            .respond_with(ResponseTemplate::new(500).set_body_string("Internal server error"))
            .mount(&mock_server)
            .await;

        let result =
            check_app_password_exists(&http_client, &mock_server.uri(), "test-token").await;

        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Unexpected response")
        );
    }
}
