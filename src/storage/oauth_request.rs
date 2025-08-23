use super::StorageResult;
use crate::errors::StorageError;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthRequest {
    pub oauth_state: String,
    pub nonce: String,
    pub pkce_verifier: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

#[async_trait]
pub trait OAuthRequestStorage: Send + Sync {
    async fn get_oauth_request_by_state(&self, state: &str) -> StorageResult<Option<OAuthRequest>>;
    async fn insert_oauth_request(&self, request: &OAuthRequest) -> StorageResult<()>;
    async fn delete_oauth_request_by_state(&self, state: &str) -> StorageResult<()>;
    async fn delete_expired_requests(&self) -> StorageResult<u64>;
}

#[async_trait]
impl<T: OAuthRequestStorage + ?Sized> OAuthRequestStorage for Arc<T> {
    async fn get_oauth_request_by_state(&self, state: &str) -> StorageResult<Option<OAuthRequest>> {
        self.as_ref().get_oauth_request_by_state(state).await
    }

    async fn insert_oauth_request(&self, request: &OAuthRequest) -> StorageResult<()> {
        self.as_ref().insert_oauth_request(request).await
    }

    async fn delete_oauth_request_by_state(&self, state: &str) -> StorageResult<()> {
        self.as_ref().delete_oauth_request_by_state(state).await
    }

    async fn delete_expired_requests(&self) -> StorageResult<u64> {
        self.as_ref().delete_expired_requests().await
    }
}

pub struct PostgresOAuthRequestStorage {
    pool: PgPool,
}

impl PostgresOAuthRequestStorage {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl OAuthRequestStorage for PostgresOAuthRequestStorage {
    async fn get_oauth_request_by_state(&self, state: &str) -> StorageResult<Option<OAuthRequest>> {
        let row = sqlx::query(
            r#"
            SELECT oauth_state, nonce, pkce_verifier, created_at, expires_at
            FROM oauth_requests
            WHERE oauth_state = $1 AND expires_at > NOW()
            "#,
        )
        .bind(state)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::QueryFailed { source: e })?;

        Ok(row.map(|row| OAuthRequest {
            oauth_state: row.get("oauth_state"),
            nonce: row.get("nonce"),
            pkce_verifier: row.get("pkce_verifier"),
            created_at: row.get("created_at"),
            expires_at: row.get("expires_at"),
        }))
    }

    async fn insert_oauth_request(&self, request: &OAuthRequest) -> StorageResult<()> {
        sqlx::query(
            r#"
            INSERT INTO oauth_requests (oauth_state, nonce, pkce_verifier, created_at, expires_at)
            VALUES ($1, $2, $3, $4, $5)
            "#,
        )
        .bind(&request.oauth_state)
        .bind(&request.nonce)
        .bind(&request.pkce_verifier)
        .bind(request.created_at)
        .bind(request.expires_at)
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::QueryFailed { source: e })?;

        Ok(())
    }

    async fn delete_oauth_request_by_state(&self, state: &str) -> StorageResult<()> {
        sqlx::query("DELETE FROM oauth_requests WHERE oauth_state = $1")
            .bind(state)
            .execute(&self.pool)
            .await
            .map_err(|e| StorageError::QueryFailed { source: e })?;

        Ok(())
    }

    async fn delete_expired_requests(&self) -> StorageResult<u64> {
        let result = sqlx::query("DELETE FROM oauth_requests WHERE expires_at <= NOW()")
            .execute(&self.pool)
            .await
            .map_err(|e| StorageError::QueryFailed { source: e })?;

        Ok(result.rows_affected())
    }
}
