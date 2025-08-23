use crate::errors::StorageError;
use sqlx::{Pool, Postgres};

type Result<T> = std::result::Result<T, StorageError>;

pub mod model {
    use chrono::{DateTime, Utc};
    use serde::Deserialize;
    use sqlx::FromRow;

    #[derive(Clone, FromRow, Deserialize)]
    pub struct OAuthRequest {
        pub oauth_state: String,
        pub issuer: String,
        pub authorization_server: String,
        pub did: String,
        pub nonce: String,
        pub pkce_verifier: String,
        pub secret_jwk_id: String,
        pub destination: Option<String>,
        pub dpop_jwk: String,
        pub created_at: DateTime<Utc>,
        pub expires_at: DateTime<Utc>,
    }

    #[derive(Clone, FromRow, Deserialize)]
    pub struct OAuthSession {
        pub session_chain: String,
        pub access_token: String,
        pub did: String,
        pub issuer: String,
        pub refresh_token: String,
        pub secret_jwk_id: String,
        pub dpop_jwk: String,
        pub created_at: DateTime<Utc>,
        pub access_token_expires_at: DateTime<Utc>,
    }
}

use self::model::OAuthSession;

#[derive(Clone)]
pub struct PostgresOAuthSessionStorage {
    pool: Pool<Postgres>,
}

impl PostgresOAuthSessionStorage {
    pub fn new(pool: Pool<Postgres>) -> Self {
        Self { pool }
    }

    pub async fn create_session(&self, session: &OAuthSession) -> Result<()> {
        atpoauth_session_create(&self.pool, session).await
    }

    pub async fn get_session(&self, session_chain: &str) -> Result<Option<OAuthSession>> {
        atpoauth_session_get(&self.pool, session_chain).await
    }

    pub async fn delete_session(&self, session_chain: &str) -> Result<()> {
        atpoauth_session_delete(&self.pool, session_chain).await
    }
}

pub(crate) async fn atpoauth_session_create(
    pool: &Pool<Postgres>,
    session: &OAuthSession,
) -> Result<()> {
    if session.session_chain.trim().is_empty() {
        return Err(StorageError::InvalidInput {
            details: "Empty session chain".to_string(),
        });
    }
    if session.access_token.trim().is_empty() {
        return Err(StorageError::InvalidInput {
            details: "Empty access token".to_string(),
        });
    }
    if session.did.trim().is_empty() {
        return Err(StorageError::InvalidInput {
            details: "Empty DID".to_string(),
        });
    }

    let mut tx = pool
        .begin()
        .await
        .map_err(|e| StorageError::ConnectionFailed { source: e })?;

    sqlx::query(
        r"
        INSERT INTO atpoauth_sessions (
            session_chain, access_token, did, issuer, refresh_token, 
            secret_jwk_id, dpop_jwk, created_at, access_token_expires_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
        ",
    )
    .bind(&session.session_chain)
    .bind(&session.access_token)
    .bind(&session.did)
    .bind(&session.issuer)
    .bind(&session.refresh_token)
    .bind(&session.secret_jwk_id)
    .bind(&session.dpop_jwk)
    .bind(session.created_at)
    .bind(session.access_token_expires_at)
    .execute(tx.as_mut())
    .await
    .map_err(|e| StorageError::QueryFailed { source: e })?;

    tx.commit()
        .await
        .map_err(|e| StorageError::TransactionFailed { source: e })?;

    Ok(())
}

pub(crate) async fn atpoauth_session_get(
    pool: &Pool<Postgres>,
    session_chain: &str,
) -> Result<Option<OAuthSession>> {
    if session_chain.trim().is_empty() {
        return Err(StorageError::InvalidInput {
            details: "Empty session chain".to_string(),
        });
    }

    let mut tx = pool
        .begin()
        .await
        .map_err(|e| StorageError::ConnectionFailed { source: e })?;

    let session = sqlx::query_as::<_, OAuthSession>(
        r"
        SELECT session_chain, access_token, did, issuer, refresh_token, 
               secret_jwk_id, dpop_jwk, created_at, access_token_expires_at
        FROM atpoauth_sessions 
        WHERE session_chain = $1 AND access_token_expires_at > NOW()
        ",
    )
    .bind(session_chain)
    .fetch_optional(tx.as_mut())
    .await
    .map_err(|e| StorageError::QueryFailed { source: e })?;

    tx.commit()
        .await
        .map_err(|e| StorageError::TransactionFailed { source: e })?;

    Ok(session)
}

pub(crate) async fn atpoauth_session_delete(
    pool: &Pool<Postgres>,
    session_chain: &str,
) -> Result<()> {
    if session_chain.trim().is_empty() {
        return Err(StorageError::InvalidInput {
            details: "Empty session chain".to_string(),
        });
    }

    let mut tx = pool
        .begin()
        .await
        .map_err(|e| StorageError::ConnectionFailed { source: e })?;

    sqlx::query(
        r"
        DELETE FROM atpoauth_sessions 
        WHERE session_chain = $1
        ",
    )
    .bind(session_chain)
    .execute(tx.as_mut())
    .await
    .map_err(|e| StorageError::QueryFailed { source: e })?;

    tx.commit()
        .await
        .map_err(|e| StorageError::TransactionFailed { source: e })?;

    Ok(())
}
