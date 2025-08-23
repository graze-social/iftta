use anyhow::Result;
use atproto_oauth::pkce::generate;
use axum::{
    extract::{Form, Query, State},
    response::{IntoResponse, Redirect},
};
use axum_extra::extract::{
    PrivateCookieJar,
    cookie::{Cookie, SameSite},
};
use axum_template::RenderHtml;
use chrono::Utc;
use minijinja::context;
use rand::{Rng, distributions::Alphanumeric};
use serde::Deserialize;
use ulid::Ulid;
use url::form_urlencoded;

use crate::{
    errors::HttpError,
    http::{context::WebContext, errors::WebError},
    storage::{OAuthRequest, Session},
};

pub const AUTH_COOKIE_NAME: &str = "ifthisthenat_session";

/// Form data structure for the OAuth login request
#[derive(Deserialize)]
pub(super) struct LoginForm {
    /// The ATProto handle to authenticate (e.g., "user.bsky.social")
    pub handle: String,
}

/// OAuth callback query parameters
#[derive(Deserialize)]
pub(super) struct OAuthCallbackQuery {
    /// Authorization code from the authorization server
    code: Option<String>,
    /// State parameter for CSRF protection
    state: Option<String>,
    /// Error parameter (if authorization failed)
    error: Option<String>,
    /// Error description (if authorization failed)
    error_description: Option<String>,
}

/// Handle GET requests to show the login form
pub(super) async fn handle_login_get(
    State(web_context): State<WebContext>,
) -> Result<impl IntoResponse, WebError> {
    Ok(RenderHtml(
        "login.html",
        web_context.engine.clone(),
        context! {
            canonical_url => format!("{}/login", web_context.config.external_base),
        },
    )
    .into_response())
}

/// Handle POST requests to process login form
pub(super) async fn handle_login_post(
    State(web_context): State<WebContext>,
    Form(form_data): Form<LoginForm>,
) -> Result<impl IntoResponse, WebError> {
    let redirect = handle_aip_login(web_context, form_data).await?;
    Ok(redirect)
}

/// Handle login for AIP OAuth backend
async fn handle_aip_login(
    web_context: WebContext,
    form_data: LoginForm,
) -> Result<Redirect, WebError> {
    // Resolve the handle to a DID document
    let _did_document = web_context
        .identity_resolver
        .resolve(&form_data.handle)
        .await
        .map_err(|e| {
            WebError::Http(HttpError::Unhandled {
                details: format!("Failed to resolve handle '{}': {}", form_data.handle, e),
            })
        })?;

    // Generate OAuth parameters
    let state: String = rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(30)
        .map(char::from)
        .collect();
    let nonce: String = rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(30)
        .map(char::from)
        .collect();
    let (pkce_verifier, code_challenge) = generate();

    // Get AIP server configuration
    let aip_hostname = format!("https://{}", web_context.config.oauth.hostname);
    let aip_client_id = &web_context.config.oauth.client_id;
    let aip_client_secret = &web_context.config.oauth.client_secret;
    let aip_oauth_scope = &web_context.config.oauth.scope;

    // Create AIP-specific OAuth request state with scope
    let aip_oauth_request_state = atproto_oauth::workflow::OAuthRequestState {
        state: state.clone(),
        nonce: nonce.clone(),
        code_challenge,
        scope: aip_oauth_scope.clone(),
    };

    // Get AIP authorization server metadata
    let authorization_server = atproto_oauth_aip::resources::oauth_authorization_server(
        &web_context.http_client,
        &aip_hostname,
    )
    .await
    .map_err(|e| {
        WebError::Http(HttpError::Unhandled {
            details: format!("Failed to get AIP authorization server: {}", e),
        })
    })?;

    // Create AIP OAuth client
    let oauth_client = atproto_oauth_aip::workflow::OAuthClient {
        redirect_uri: format!(
            "https://{}/login/callback",
            web_context.config.external_base
        ),
        client_id: aip_client_id.clone(),
        client_secret: aip_client_secret.clone(),
    };

    // Initialize AIP OAuth flow
    let par_response = atproto_oauth_aip::workflow::oauth_init(
        &web_context.http_client,
        &oauth_client,
        Some(&form_data.handle),
        &authorization_server.pushed_authorization_request_endpoint,
        &aip_oauth_request_state,
    )
    .await
    .map_err(|e| {
        WebError::Http(HttpError::Unhandled {
            details: format!("AIP OAuth initialization failed: {}", e),
        })
    })?;

    // Store OAuth request for callback
    let created_at = chrono::Utc::now();
    let expires_at = created_at + chrono::Duration::seconds(par_response.expires_in as i64);

    let oauth_request = OAuthRequest {
        oauth_state: state.clone(),
        nonce: nonce.clone(),
        pkce_verifier,
        created_at,
        expires_at,
    };

    web_context
        .oauth_request_storage
        .insert_oauth_request(&oauth_request)
        .await
        .map_err(|e| {
            WebError::Http(HttpError::Unhandled {
                details: format!("Failed to store OAuth request: {}", e),
            })
        })?;

    // Build authorization URL with proper URL encoding
    let encoded_params: String = form_urlencoded::Serializer::new(String::new())
        .append_pair("request_uri", &par_response.request_uri)
        .append_pair("client_id", aip_client_id)
        .finish();

    let authorization_url = format!(
        "{}?{}",
        authorization_server.authorization_endpoint,
        encoded_params
    );

    Ok(Redirect::to(&authorization_url))
}

/// Handle the OAuth callback
pub(super) async fn handle_login_callback(
    State(web_context): State<WebContext>,
    jar: PrivateCookieJar,
    Query(query): Query<OAuthCallbackQuery>,
) -> Result<impl IntoResponse, WebError> {
    let response = handle_aip_callback(web_context, jar, query).await?;
    Ok(response)
}

async fn handle_aip_callback(
    web_context: WebContext,
    jar: PrivateCookieJar,
    query: OAuthCallbackQuery,
) -> Result<(PrivateCookieJar, Redirect), WebError> {
    // Check for OAuth errors first
    if let Some(error) = query.error {
        let error_desc = query
            .error_description
            .unwrap_or_else(|| "Unknown error".to_string());
        tracing::warn!(error = %error, description = %error_desc, "OAuth authorization failed");

        return Err(WebError::Http(HttpError::RequestValidation {
            details: format!("OAuth authorization failed: {} - {}", error, error_desc),
        }));
    }

    // Extract required parameters
    let (callback_code, callback_state) = match (query.code, query.state) {
        (Some(code), Some(state)) => (code, state),
        _ => {
            return Err(WebError::Http(HttpError::RequestValidation {
                details: "Missing required OAuth callback parameters (code or state)".to_string(),
            }));
        }
    };

    // Retrieve the OAuth request from storage
    let oauth_request = web_context
        .oauth_request_storage
        .get_oauth_request_by_state(&callback_state)
        .await
        .map_err(|e| {
            WebError::Http(HttpError::Unhandled {
                details: format!("Failed to retrieve OAuth request: {}", e),
            })
        })?;

    let oauth_request = oauth_request.ok_or_else(|| {
        WebError::Http(HttpError::RequestValidation {
            details: "OAuth request not found or expired".to_string(),
        })
    })?;

    // Get AIP server configuration
    let aip_hostname = format!("https://{}", web_context.config.oauth.hostname);
    let aip_client_id = &web_context.config.oauth.client_id;
    let aip_client_secret = &web_context.config.oauth.client_secret;
    let _aip_oauth_scope = &web_context.config.oauth.scope;

    // Get AIP authorization server
    let authorization_server = atproto_oauth_aip::resources::oauth_authorization_server(
        &web_context.http_client,
        &aip_hostname,
    )
    .await
    .map_err(|e| {
        WebError::Http(HttpError::Unhandled {
            details: format!("Failed to get AIP authorization server: {}", e),
        })
    })?;

    // Create AIP OAuth client
    let oauth_client = atproto_oauth_aip::workflow::OAuthClient {
        redirect_uri: format!(
            "https://{}/login/callback",
            web_context.config.external_base
        ),
        client_id: aip_client_id.clone(),
        client_secret: aip_client_secret.clone(),
    };

    // Perform the token exchange using simplified AIP OAuth request
    let simplified_oauth_request = atproto_oauth::workflow::OAuthRequest {
        oauth_state: oauth_request.oauth_state.clone(),
        issuer: authorization_server.issuer.clone(),
        authorization_server: authorization_server.issuer.clone(),
        nonce: oauth_request.nonce.clone(),
        pkce_verifier: oauth_request.pkce_verifier.clone(),
        signing_public_key: "".to_string(), // Not used for AIP
        dpop_private_key: "".to_string(),   // Not used for AIP
        created_at: oauth_request.created_at,
        expires_at: oauth_request.expires_at,
    };

    let token_response = atproto_oauth_aip::workflow::oauth_complete(
        &web_context.http_client,
        &oauth_client,
        &authorization_server.token_endpoint,
        &callback_code,
        &simplified_oauth_request,
    )
    .await
    .map_err(|e| {
        WebError::Http(HttpError::Unhandled {
            details: format!("AIP token exchange failed: {}", e),
        })
    })?;

    // Get user info from AIP server
    let (did, _maybe_email) = get_user_info(
        &web_context.http_client,
        &aip_hostname,
        &token_response.access_token,
    )
    .await
    .map_err(|e| WebError::Http(HttpError::Unhandled { details: e }))?;

    // Resolve the DID document (CachingIdentityResolver handles storage automatically)
    let _did_document = web_context
        .identity_resolver
        .resolve(&did)
        .await
        .map_err(|e| {
            WebError::Http(HttpError::Unhandled {
                details: format!("Failed to resolve DID '{}': {}", did, e),
            })
        })?;

    // Store the session
    let session_id = Ulid::new().to_string();
    let access_token_expires_at =
        Utc::now() + chrono::Duration::seconds(token_response.expires_in as i64);

    let session = Session {
        session_id: session_id.clone(),
        did: did.clone(),
        access_token: token_response.access_token.clone(),
        refresh_token: token_response.refresh_token.clone(),
        access_token_expires_at,
        session_type: crate::storage::session::SessionType::OAuth,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };

    web_context
        .session_storage
        .upsert_session(&session)
        .await
        .map_err(|e| {
            WebError::Http(HttpError::Unhandled {
                details: format!("Failed to store session: {}", e),
            })
        })?;

    // Create session cookie
    let mut cookie = Cookie::new(AUTH_COOKIE_NAME, session_id);
    cookie.set_domain(web_context.config.external_base.clone());
    cookie.set_path("/");
    cookie.set_http_only(true);
    cookie.set_secure(true);
    cookie.set_max_age(Some(cookie::time::Duration::seconds(
        (token_response.expires_in as i64) - 60,
    )));
    cookie.set_same_site(Some(SameSite::Lax));

    let updated_jar = jar.add(cookie);

    // Clean up the OAuth request from storage
    if let Err(e) = web_context
        .oauth_request_storage
        .delete_oauth_request_by_state(&oauth_request.oauth_state)
        .await
    {
        tracing::warn!(error = ?e, "Failed to delete OAuth request");
        // Don't fail the whole flow for cleanup errors
    }

    // Return success response
    Ok((updated_jar, Redirect::to("/dashboard")))
}

#[derive(Clone, Deserialize)]
struct OpenIDClaims {
    pub did: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,

    // Allow for other fields that might be present but aren't used
    #[serde(flatten)]
    pub _extra: std::collections::HashMap<String, serde_json::Value>,
}

#[derive(Clone, Deserialize)]
#[serde(untagged)]
enum OpenIDClaimsResponse {
    OpenIDClaims(OpenIDClaims),
    SimpleError(atproto_client::errors::SimpleError),
}

async fn get_user_info(
    http_client: &reqwest::Client,
    aip_server: &str,
    aip_access_token: &str,
) -> Result<(String, Option<String>), String> {
    let userinfo_endpoint = format!("{}/oauth/userinfo", aip_server);

    let response: OpenIDClaimsResponse = http_client
        .get(userinfo_endpoint)
        .bearer_auth(aip_access_token)
        .send()
        .await
        .map_err(|e| format!("AIP userinfo request failed: {}", e))?
        .json()
        .await
        .map_err(|e| format!("AIP userinfo parsing failed: {}", e))?;

    match response {
        OpenIDClaimsResponse::OpenIDClaims(claims) => Ok((claims.did, claims.email)),
        OpenIDClaimsResponse::SimpleError(simple_error) => Err(format!(
            "AIP server error: {}",
            simple_error.error_message()
        )),
    }
}
