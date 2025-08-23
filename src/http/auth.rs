//! Shared authentication utilities for HTTP handlers

use axum::{
    http::{HeaderMap, StatusCode},
    response::Json,
};
use axum_extra::extract::PrivateCookieJar;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use tokio::sync::RwLock;

use crate::{
    errors::HttpError,
    http::{WebContext, errors::WebError, handle_auth::AUTH_COOKIE_NAME},
    storage::Session,
};

/// Represents the authentication method and associated data
#[derive(Debug, Clone)]
pub(super) enum AuthenticationMethod {
    /// Cookie authentication with DID and session access token
    Cookie(String, #[allow(dead_code)] String),
    /// Bearer token (ATProtocol inter-service JWT) with DID
    Bearer(String),
    /// AIP access token with DID and token
    Aip(String, #[allow(dead_code)] String),
}

impl AuthenticationMethod {
    /// Extract the DID from any authentication method
    pub fn did(&self) -> &str {
        match self {
            AuthenticationMethod::Cookie(did, _) => did,
            AuthenticationMethod::Bearer(did) => did,
            AuthenticationMethod::Aip(did, _) => did,
        }
    }
}

/// Get session from cookie - used by web handlers
pub(super) async fn get_session_from_cookie(
    web_context: &WebContext,
    jar: &PrivateCookieJar,
) -> Result<Session, WebError> {
    // Get session ID from cookie
    let cookie = jar.get(AUTH_COOKIE_NAME).ok_or_else(|| {
        WebError::Http(HttpError::Unauthorized {
            details: "No session cookie found".to_string(),
        })
    })?;

    let session_id = cookie.value();

    // Get session from storage
    let session = web_context
        .session_storage
        .get_session(session_id)
        .await
        .map_err(|e| {
            WebError::Http(HttpError::Unhandled {
                details: format!("Failed to retrieve session: {}", e),
            })
        })?;

    session.ok_or_else(|| {
        WebError::Http(HttpError::Unauthorized {
            details: "Session not found or expired".to_string(),
        })
    })
}

/// Get authenticated DID from session cookie or Authorization header - used by XRPC handlers
pub(super) async fn get_authenticated_did(
    web_context: &WebContext,
    jar: &PrivateCookieJar,
    headers: &HeaderMap,
) -> Result<AuthenticationMethod, (StatusCode, Json<serde_json::Value>)> {
    // First try to get DID from session cookie (web authentication)
    if let Some(cookie) = jar.get(AUTH_COOKIE_NAME) {
        let session_id = cookie.value();
        if let Ok(Some(session)) = web_context.session_storage.get_session(session_id).await {
            return Ok(AuthenticationMethod::Cookie(
                session.did.clone(),
                session.access_token.clone(),
            ));
        }
    }

    // Check Authorization header for Bearer token or AIP token
    if let Some(auth_header) = headers.get("authorization")
        && let Ok(auth_str) = auth_header.to_str()
    {
        // Handle Bearer token (ATProtocol inter-service JWT)
        if auth_str.starts_with("Bearer ") {
            let token = auth_str.strip_prefix("Bearer ").unwrap(); // Remove "Bearer " prefix

            // Validate ATProtocol JWT and extract DID
            match validate_atprotocol_jwt(token, web_context).await {
                Ok(did) => return Ok(AuthenticationMethod::Bearer(did)),
                Err(e) => {
                    // Log the error but continue to check other auth methods
                    tracing::debug!("Bearer token validation failed: {}", e);
                }
            }
        }

        // Handle AIP access token
        if auth_str.starts_with("AIP ") {
            let token = auth_str.strip_prefix("AIP ").unwrap(); // Remove "AIP " prefix

            // Validate AIP access token and extract DID
            match validate_aip_access_token(token, web_context).await {
                Ok(did) => {
                    return Ok(AuthenticationMethod::Aip(did, token.to_string()));
                }
                Err(e) => {
                    // Log the error but continue
                    tracing::debug!("AIP token validation failed: {}", e);
                }
            }
        }
    }

    Err((
        StatusCode::UNAUTHORIZED,
        Json(json!({
            "error": "AuthenticationRequired",
            "message": "Authentication required for this endpoint"
        })),
    ))
}

/// Validate an ATProtocol inter-service JWT and extract the DID
async fn validate_atprotocol_jwt(
    _token: &str,
    _web_context: &WebContext,
) -> Result<String, String> {
    // TODO: Implement proper JWT validation using atproto_oauth crate
    // This should:
    // 1. Decode the JWT
    // 2. Verify the signature using the issuer's public key
    // 3. Check expiration and other standard claims
    // 4. Extract the 'sub' claim which should be the DID

    // For now, return an error indicating this needs implementation
    Err("ATProtocol JWT validation not yet implemented".to_string())
}

/// Cache entry for AIP access tokens
#[derive(Clone)]
struct AipTokenCacheEntry {
    did: String,
    expires_at: DateTime<Utc>,
}

/// Global in-memory cache for AIP access tokens
/// Maps access_token -> (DID, expiration)
static AIP_TOKEN_CACHE: OnceLock<Arc<RwLock<HashMap<String, AipTokenCacheEntry>>>> =
    OnceLock::new();

/// Get or initialize the AIP token cache
fn get_aip_token_cache() -> &'static Arc<RwLock<HashMap<String, AipTokenCacheEntry>>> {
    AIP_TOKEN_CACHE.get_or_init(|| Arc::new(RwLock::new(HashMap::new())))
}

/// Clear the AIP token cache (useful for testing or administrative purposes)
#[allow(dead_code)]
pub async fn clear_aip_token_cache() {
    let cache_ref = get_aip_token_cache();
    let mut cache = cache_ref.write().await;
    cache.clear();
    tracing::info!("AIP token cache cleared");
}

/// AIP UserInfo response structure
#[derive(Debug, Deserialize, Serialize)]
struct AipUserInfoResponse {
    sub: String, // The DID of the user
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    did: Option<String>,
}

/// Validate an AIP access token and extract the DID
async fn validate_aip_access_token(
    token: &str,
    web_context: &WebContext,
) -> Result<String, String> {
    tracing::debug!("validate_aip_access_token");

    // First check the cache
    let cache_ref = get_aip_token_cache();
    {
        let cache = cache_ref.read().await;
        if let Some(entry) = cache.get(token) {
            // Check if the cached entry is still valid
            if entry.expires_at > Utc::now() {
                tracing::debug!("AIP token found in cache for DID: {}", entry.did);
                return Ok(entry.did.clone());
            }
        }
    }

    tracing::debug!("aip cache miss");

    // Token not in cache or expired, make API call to AIP userinfo endpoint
    let aip_base_url = web_context
        .config
        .aip_base_url
        .as_ref()
        .ok_or_else(|| "AIP base URL not configured".to_string())?;

    let userinfo_url = format!("https://{}/oauth/userinfo", aip_base_url);

    tracing::debug!(?userinfo_url, "userinfo_url");

    // Make the API request with the access token
    let response = web_context
        .http_client
        .get(&userinfo_url)
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| format!("Failed to call AIP userinfo endpoint: {}", e))?;

    if !response.status().is_success() {
        tracing::error!("response is not successful");
        let status = response.status();
        let error_body = response
            .text()
            .await
            .unwrap_or_else(|_| "Unknown error".to_string());
        return Err(format!(
            "AIP userinfo request failed with status {}: {}",
            status,
            error_body
        ));
    }

    // Parse the userinfo response
    let userinfo: AipUserInfoResponse = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse AIP userinfo response: {}", e))?;

    tracing::debug!(?userinfo, "userinfo");

    // Extract the DID from the response
    // The DID can be in either the 'sub' field or the 'did' field
    let did = userinfo.did.unwrap_or(userinfo.sub.clone());

    // Cache the token with a 5-minute TTL
    // You may want to adjust this based on your security requirements
    let cache_entry = AipTokenCacheEntry {
        did: did.clone(),
        expires_at: Utc::now() + Duration::minutes(5),
    };

    {
        let mut cache = cache_ref.write().await;
        cache.insert(token.to_string(), cache_entry);

        // Clean up expired entries while we have the write lock
        cache.retain(|_, entry| entry.expires_at > Utc::now());
    }

    tracing::debug!("AIP token validated and cached for DID: {}", did);
    Ok(did)
}

/// Check if a DID is in the allowed identities list
/// If the allowed_identities list is empty, returns true (all DIDs allowed)
/// Otherwise, returns whether the DID is in the allowed list
pub(crate) fn is_allowed_identity(
    allowed_identities: &std::collections::HashSet<String>,
    did: &str,
) -> bool {
    if allowed_identities.is_empty() {
        // No restrictions if the list is empty
        true
    } else {
        // Check if the DID is in the allowed list
        allowed_identities.contains(did)
    }
}
