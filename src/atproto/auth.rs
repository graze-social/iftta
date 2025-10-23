use anyhow::Result;
use atproto_client::client::{AppPasswordAuth, Auth, DPoPAuth};
use atproto_identity::key::identify_key;
use atproto_oauth::workflow::TokenResponse;
use atproto_oauth_aip::workflow::{ATProtocolSession, session_exchange_with_options};
use chrono::{DateTime, Duration, Utc};
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};

use crate::errors::AuthError;

/// Internal state for the service access token manager
#[derive(Debug, Clone)]
struct AccessTokenState {
    /// The current access token
    access_token: String,

    /// When the access token expires
    expires_at: DateTime<Utc>,
}

/// Thread-safe manager for service access tokens with automatic refresh capability.
///
/// This manager handles the lifecycle of OAuth access tokens obtained via client
/// credentials grant, providing thread-safe access and automatic refresh when needed.
#[derive(Clone)]
pub struct ServiceAccessTokenManager {
    /// HTTP client for making requests
    http_client: Arc<reqwest::Client>,

    /// AIP hostname for token requests and session exchange
    aip_hostname: String,

    /// OAuth client ID
    client_id: String,

    /// OAuth client secret
    client_secret: String,

    /// Current token state protected by RwLock
    state: Arc<RwLock<Option<AccessTokenState>>>,

    /// Mutex to prevent concurrent token refreshes
    refresh_mutex: Arc<Mutex<()>>,
}

impl ServiceAccessTokenManager {
    /// Creates a new ServiceAccessTokenManager instance.
    ///
    /// # Arguments
    ///
    /// * `http_client` - Shared HTTP client for making requests
    /// * `aip_hostname` - The hostname of the AIP instance (without https://)
    /// * `client_id` - The OAuth client ID for authentication
    /// * `client_secret` - The OAuth client secret for authentication
    pub fn new(
        http_client: Arc<reqwest::Client>,
        aip_hostname: String,
        client_id: String,
        client_secret: String,
    ) -> Self {
        Self {
            http_client,
            aip_hostname,
            client_id,
            client_secret,
            state: Arc::new(RwLock::new(None)),
            refresh_mutex: Arc::new(Mutex::new(())),
        }
    }

    /// Initialize the manager by obtaining an initial access token.
    ///
    /// This method uses the client credentials grant flow to obtain an initial
    /// access token and stores it in the internal state.
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` on success, or an error if token generation fails
    ///
    /// # Example
    ///
    /// ```no_run
    /// # async fn example() -> anyhow::Result<()> {
    /// # use std::sync::Arc;
    /// # let http_client = Arc::new(reqwest::Client::new());
    /// let manager = ifthisthenat::ServiceAccessTokenManager::new(
    ///     http_client,
    ///     "auth.example.com".to_string(),
    ///     "client_id".to_string(),
    ///     "client_secret".to_string(),
    /// );
    /// manager.init().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn init(&self) -> Result<()> {
        // Use the refresh mutex to ensure thread safety during initialization
        let _refresh_guard = self.refresh_mutex.lock().await;

        // Check if already initialized
        {
            let state_lock = self.state.read().await;
            if state_lock.is_some() {
                return Ok(());
            }
        }

        // Generate initial access token
        let token_response = self.generate_service_access_token().await.map_err(|e| {
            AuthError::OAuthOperationFailed {
                operation: "initialize".to_string(),
                details: format!("Failed to initialize ServiceAccessTokenManager: {}", e),
            }
        })?;

        // Calculate expiration time
        let expires_in_seconds = token_response.expires_in as i64;
        // Subtract 5 minutes for safety margin
        let actual_expires_in = expires_in_seconds - 300;
        let expires_at = Utc::now() + Duration::seconds(actual_expires_in.max(216000));

        // Store the token state
        let state = AccessTokenState {
            access_token: token_response.access_token.clone(),
            expires_at,
        };

        let mut state_lock = self.state.write().await;
        *state_lock = Some(state);

        Ok(())
    }

    /// Get an AT Protocol session for a specific subject using the managed access token.
    ///
    /// This method uses the current access token to exchange for an AT Protocol session
    /// for the specified subject. It will automatically refresh the access token if it
    /// has expired.
    ///
    /// # Arguments
    ///
    /// * `subject` - The subject (DID) to get a session for
    ///
    /// # Returns
    ///
    /// Returns a tuple of `(Auth, ATProtocolSession)` for the specified subject
    ///
    /// # Example
    ///
    /// ```no_run
    /// # async fn example() -> anyhow::Result<()> {
    /// # use std::sync::Arc;
    /// # let http_client = Arc::new(reqwest::Client::new());
    /// # let manager = ifthisthenat::ServiceAccessTokenManager::new(
    /// #     http_client,
    /// #     "auth.example.com".to_string(),
    /// #     "client_id".to_string(),
    /// #     "client_secret".to_string(),
    /// # );
    /// # manager.init().await?;
    /// let (auth, session) = manager.auth("did:plc:example123").await?;
    /// println!("Got session for: {} ({})", session.handle, session.did);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn auth(&self, subject: &str) -> Result<(Auth, ATProtocolSession)> {
        // Retry logic for token expiry edge cases
        const MAX_RETRIES: u32 = 2;
        let mut last_error = None;

        for _attempt in 0..MAX_RETRIES {
            // Ensure we have a valid token
            self.ensure_valid_token().await?;

            // Get the current access token and check if it's still valid
            let (access_token, expires_soon) = {
                let state_lock = self.state.read().await;
                let state = state_lock
                    .as_ref()
                    .ok_or_else(|| AuthError::AuthenticationFailed {
                        details: "Token manager not initialized".to_string(),
                    })?;

                // Check if token will expire very soon (within 10 seconds)
                let expires_soon = Utc::now() >= state.expires_at - Duration::seconds(10);
                (state.access_token.clone(), expires_soon)
            };

            // If token expires very soon, refresh proactively
            if expires_soon {
                self.refresh_token().await?;
                continue;
            }

            // Try to exchange the access token for an AT Protocol session
            let protected_resource_base = format!("https://{}", self.aip_hostname);
            match session_exchange_with_options(
                &self.http_client,
                &protected_resource_base,
                &access_token,
                &Some("best"),
                &Some(subject),
            )
            .await
            {
                Ok(session) => {
                    // Successfully got session, create Auth and return
                    let auth = self.create_auth_from_session(&session)?;
                    return Ok((auth, session));
                }
                Err(e) => {
                    // Check if error is due to expired token
                    let error_str = e.to_string();
                    if error_str.contains("unauthorized")
                        || error_str.contains("401")
                        || error_str.contains("expired")
                        || error_str.contains("invalid_token")
                    {
                        // Token might have expired, refresh and retry
                        last_error = Some(e);
                        self.refresh_token().await?;
                        continue;
                    } else {
                        // Other error, propagate immediately
                        return Err(e);
                    }
                }
            }
        }

        // All retries exhausted
        Err(last_error.unwrap_or_else(|| {
            AuthError::AuthenticationFailed {
                details: format!("Failed to authenticate after {} attempts", MAX_RETRIES),
            }
            .into()
        }))
    }

    /// Create Auth from an AT Protocol session response
    fn create_auth_from_session(&self, session: &ATProtocolSession) -> Result<Auth> {
        match session.token_type.as_str() {
            "dpop" => {
                // Parse the DPoP private key
                let dpop_key =
                    session
                        .dpop_key
                        .as_ref()
                        .ok_or_else(|| AuthError::TokenValidationFailed {
                            details: "DPoP key not found in session response".to_string(),
                        })?;
                let dpop_private_key_data = identify_key(dpop_key)?;

                Ok(Auth::DPoP(DPoPAuth {
                    dpop_private_key_data,
                    oauth_access_token: session.access_token.clone(),
                }))
            }
            "bearer" => {
                // Use AppPasswordAuth for bearer tokens
                Ok(Auth::AppPassword(AppPasswordAuth {
                    access_token: session.access_token.clone(),
                }))
            }
            token_type => Err(AuthError::TokenValidationFailed {
                details: format!("Unsupported token type: {}", token_type),
            }
            .into()),
        }
    }

    /// Ensure we have a valid access token, refreshing if necessary.
    ///
    /// This is called internally by `auth()` but can also be called manually
    /// if needed for other purposes.
    async fn ensure_valid_token(&self) -> Result<()> {
        // First check without lock if token is valid
        let needs_refresh = {
            let state_lock = self.state.read().await;
            match state_lock.as_ref() {
                None => {
                    return Err(AuthError::AuthenticationFailed {
                        details: "Token manager not initialized. Call init() first".to_string(),
                    }
                    .into());
                }
                Some(state) => {
                    // Check if token is expired or will expire soon (within 1 minute)
                    Utc::now() >= state.expires_at - Duration::seconds(60)
                }
            }
        };

        if needs_refresh {
            // Acquire refresh mutex to prevent concurrent refreshes
            let _refresh_guard = self.refresh_mutex.lock().await;

            // Double-check pattern: Check again after acquiring the mutex
            // Another thread may have already refreshed the token
            let still_needs_refresh = {
                let state_lock = self.state.read().await;
                match state_lock.as_ref() {
                    None => true,
                    Some(state) => Utc::now() >= state.expires_at - Duration::seconds(216000),
                }
            };

            if still_needs_refresh {
                self.refresh_token_internal().await?;
            }
        }

        Ok(())
    }

    /// Internal refresh method that doesn't acquire the mutex (called when mutex is already held)
    async fn refresh_token_internal(&self) -> Result<()> {
        // Generate new access token using client credentials
        let token_response = self.generate_service_access_token().await.map_err(|e| {
            AuthError::OAuthOperationFailed {
                operation: "refresh_token".to_string(),
                details: format!("Failed to generate service access token: {}", e),
            }
        })?;

        // Calculate expiration time
        let expires_in_seconds = token_response.expires_in as i64;
        // Subtract 5 minutes for safety margin to avoid edge cases
        let actual_expires_in = expires_in_seconds - 300;
        let expires_at = Utc::now() + Duration::seconds(actual_expires_in.max(216000));

        // Update the token state
        let state = AccessTokenState {
            access_token: token_response.access_token.clone(),
            expires_at,
        };

        let mut state_lock = self.state.write().await;
        *state_lock = Some(state);

        Ok(())
    }

    /// Refresh the access token by generating a new one using client credentials.
    /// Client credentials grants don't support refresh tokens, so we always generate a new token.
    /// This method acquires the refresh mutex to prevent concurrent refreshes.
    async fn refresh_token(&self) -> Result<()> {
        let _refresh_guard = self.refresh_mutex.lock().await;
        self.refresh_token_internal().await
    }

    /// Generate a service access token using OAuth client credentials grant.
    ///
    /// This method authenticates the service with the AIP instance using client
    /// credentials to obtain an access token for service-to-service communication.
    /// Client credentials grants do not provide refresh tokens.
    ///
    /// # Returns
    ///
    /// Returns a `TokenResponse` containing the access token and metadata
    async fn generate_service_access_token(&self) -> Result<TokenResponse> {
        // Construct the token endpoint URL
        let token_url = format!("https://{}/oauth/token", self.aip_hostname);

        // Prepare the form data for client credentials grant
        let params = [("grant_type", "client_credentials")];

        // Send the request with Basic authentication
        let response = self
            .http_client
            .post(&token_url)
            .basic_auth(&self.client_id, Some(&self.client_secret))
            .form(&params)
            .send()
            .await
            .map_err(|e| AuthError::OAuthOperationFailed {
                operation: "send_token_request".to_string(),
                details: e.to_string(),
            })?;

        // Check if the request was successful
        if !response.status().is_success() {
            let status = response.status();
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            return Err(AuthError::OAuthOperationFailed {
                operation: "token_request".to_string(),
                details: format!(
                    "Token request failed with status {}: {}",
                    status, error_text
                ),
            }
            .into());
        }

        // Parse the response
        let token_response = response.json::<TokenResponse>().await.map_err(|e| {
            AuthError::OAuthOperationFailed {
                operation: "parse_token_response".to_string(),
                details: e.to_string(),
            }
        })?;

        Ok(token_response)
    }
}
