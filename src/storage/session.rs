use super::StorageResult;
use crate::errors::StorageError;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use std::sync::Arc;
use tracing::{Instrument, debug, error, info, instrument};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub enum SessionType {
    #[default]
    #[serde(rename = "oauth")]
    OAuth,
    #[serde(rename = "app_password")]
    AppPassword,
}

impl SessionType {
    pub fn as_str(&self) -> &str {
        match self {
            SessionType::OAuth => "oauth",
            SessionType::AppPassword => "app_password",
        }
    }

    pub fn from_string(s: &str) -> Option<Self> {
        match s {
            "oauth" => Some(SessionType::OAuth),
            "app_password" => Some(SessionType::AppPassword),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub session_id: String,
    pub did: String,
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub access_token_expires_at: DateTime<Utc>,
    pub session_type: SessionType,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Session {
    /// Create a new OAuth session
    pub fn new_oauth(
        session_id: String,
        did: String,
        access_token: String,
        refresh_token: Option<String>,
        access_token_expires_at: DateTime<Utc>,
    ) -> Self {
        let now = Utc::now();
        Self {
            session_id,
            did,
            access_token,
            refresh_token,
            access_token_expires_at,
            session_type: SessionType::OAuth,
            created_at: now,
            updated_at: now,
        }
    }

    /// Create a new app-password session
    pub fn new_app_password(
        session_id: String,
        did: String,
        access_token: String,
        access_token_expires_at: DateTime<Utc>,
    ) -> Self {
        let now = Utc::now();
        Self {
            session_id,
            did,
            access_token,
            refresh_token: None, // App passwords typically don't have refresh tokens
            access_token_expires_at,
            session_type: SessionType::AppPassword,
            created_at: now,
            updated_at: now,
        }
    }

    /// Check if this is an OAuth session
    pub fn is_oauth(&self) -> bool {
        matches!(self.session_type, SessionType::OAuth)
    }

    /// Check if this is an app-password session
    pub fn is_app_password(&self) -> bool {
        matches!(self.session_type, SessionType::AppPassword)
    }
}

#[async_trait]
pub trait SessionStorage: Send + Sync {
    async fn get_session(&self, session_id: &str) -> StorageResult<Option<Session>>;
    async fn get_session_by_did(&self, did: &str) -> StorageResult<Option<Session>>;
    async fn upsert_session(&self, session: &Session) -> StorageResult<()>;
    async fn delete_session(&self, session_id: &str) -> StorageResult<()>;
    async fn delete_expired_sessions(&self) -> StorageResult<u64>;
}

#[async_trait]
impl<T: SessionStorage + ?Sized> SessionStorage for Arc<T> {
    async fn get_session(&self, session_id: &str) -> StorageResult<Option<Session>> {
        self.as_ref().get_session(session_id).await
    }

    async fn get_session_by_did(&self, did: &str) -> StorageResult<Option<Session>> {
        self.as_ref().get_session_by_did(did).await
    }

    async fn upsert_session(&self, session: &Session) -> StorageResult<()> {
        self.as_ref().upsert_session(session).await
    }

    async fn delete_session(&self, session_id: &str) -> StorageResult<()> {
        self.as_ref().delete_session(session_id).await
    }

    async fn delete_expired_sessions(&self) -> StorageResult<u64> {
        self.as_ref().delete_expired_sessions().await
    }
}

pub struct PostgresSessionStorage {
    pool: PgPool,
}

impl PostgresSessionStorage {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl SessionStorage for PostgresSessionStorage {
    #[instrument(skip(self), fields(db.operation = "get_session", session.id = %session_id))]
    async fn get_session(&self, session_id: &str) -> StorageResult<Option<Session>> {
        debug!("Fetching session from database");

        let span = tracing::debug_span!(
            "database_query",
            query = "SELECT session",
            table = "sessions"
        );

        let row = sqlx::query(
            r#"
            SELECT session_id, did, access_token, refresh_token, 
                   access_token_expires_at, session_type, created_at, updated_at
            FROM sessions
            WHERE session_id = $1 AND access_token_expires_at > NOW()
            "#,
        )
        .bind(session_id)
        .fetch_optional(&self.pool)
        .instrument(span)
        .await
        .map_err(|e| {
            error!(error = ?e, session_id = %session_id, "Failed to get session");
            StorageError::QueryFailed { source: e }
        })?;

        let result = row.map(|row| {
            let session_type_str: String = row.get("session_type");
            Session {
                session_id: row.get("session_id"),
                did: row.get("did"),
                access_token: row.get("access_token"),
                refresh_token: row.get("refresh_token"),
                access_token_expires_at: row.get("access_token_expires_at"),
                session_type: SessionType::from_string(&session_type_str).unwrap_or_default(),
                created_at: row.get("created_at"),
                updated_at: row.get("updated_at"),
            }
        });

        if result.is_some() {
            debug!("Session found and is valid");
        } else {
            debug!("Session not found or expired");
        }

        Ok(result)
    }

    #[instrument(skip(self), fields(db.operation = "get_session_by_did", session.did = %did))]
    async fn get_session_by_did(&self, did: &str) -> StorageResult<Option<Session>> {
        debug!("Fetching session by DID");

        let span = tracing::debug_span!(
            "database_query",
            query = "SELECT session by DID",
            table = "sessions"
        );

        let row = sqlx::query(
            r#"
            SELECT session_id, did, access_token, refresh_token,
                   access_token_expires_at, session_type, created_at, updated_at
            FROM sessions
            WHERE did = $1 AND access_token_expires_at > NOW()
            ORDER BY updated_at DESC
            LIMIT 1
            "#,
        )
        .bind(did)
        .fetch_optional(&self.pool)
        .instrument(span)
        .await
        .map_err(|e| {
            error!(error = ?e, did = %did, "Failed to get session by DID");
            StorageError::QueryFailed { source: e }
        })?;

        Ok(row.map(|row| {
            let session_type_str: String = row.get("session_type");
            Session {
                session_id: row.get("session_id"),
                did: row.get("did"),
                access_token: row.get("access_token"),
                refresh_token: row.get("refresh_token"),
                access_token_expires_at: row.get("access_token_expires_at"),
                session_type: SessionType::from_string(&session_type_str).unwrap_or_default(),
                created_at: row.get("created_at"),
                updated_at: row.get("updated_at"),
            }
        }))
    }

    async fn upsert_session(&self, session: &Session) -> StorageResult<()> {
        sqlx::query(
            r#"
            INSERT INTO sessions (session_id, did, access_token, refresh_token,
                                  access_token_expires_at, session_type, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            ON CONFLICT (session_id) DO UPDATE SET
                did = EXCLUDED.did,
                access_token = EXCLUDED.access_token,
                refresh_token = EXCLUDED.refresh_token,
                access_token_expires_at = EXCLUDED.access_token_expires_at,
                session_type = EXCLUDED.session_type,
                updated_at = EXCLUDED.updated_at
            "#,
        )
        .bind(&session.session_id)
        .bind(&session.did)
        .bind(&session.access_token)
        .bind(&session.refresh_token)
        .bind(session.access_token_expires_at)
        .bind(session.session_type.as_str())
        .bind(session.created_at)
        .bind(session.updated_at)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            error!(error = ?e, session_id = %session.session_id, did = %session.did, "Failed to upsert session");
            StorageError::QueryFailed { source: e }
        })?;

        Ok(())
    }

    async fn delete_session(&self, session_id: &str) -> StorageResult<()> {
        sqlx::query("DELETE FROM sessions WHERE session_id = $1")
            .bind(session_id)
            .execute(&self.pool)
            .await
            .map_err(|e| {
                error!(error = ?e, session_id = %session_id, "Failed to delete session");
                StorageError::QueryFailed { source: e }
            })?;

        Ok(())
    }

    async fn delete_expired_sessions(&self) -> StorageResult<u64> {
        let result = sqlx::query("DELETE FROM sessions WHERE access_token_expires_at <= NOW()")
            .execute(&self.pool)
            .await
            .map_err(|e| {
                error!(error = ?e, "Failed to delete expired sessions");
                StorageError::QueryFailed { source: e }
            })?;

        let deleted_count = result.rows_affected();
        if deleted_count > 0 {
            info!(count = deleted_count, "Cleaned up expired sessions");
        }
        Ok(deleted_count)
    }
}
