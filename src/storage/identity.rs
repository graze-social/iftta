use super::StorageResult;
use crate::errors::StorageError;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Identity {
    pub did: String,
    pub handle: Option<String>,
    pub document: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[async_trait]
pub trait IdentityStorage: Send + Sync {
    async fn get_identity(&self, did: &str) -> StorageResult<Option<Identity>>;
    async fn upsert_identity(&self, identity: &Identity) -> StorageResult<()>;
    async fn delete_identity(&self, did: &str) -> StorageResult<()>;
}

pub struct PostgresIdentityStorage {
    pool: PgPool,
}

impl PostgresIdentityStorage {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl IdentityStorage for PostgresIdentityStorage {
    async fn get_identity(&self, did: &str) -> StorageResult<Option<Identity>> {
        let row = sqlx::query(
            r#"
            SELECT did, handle, document, created_at, updated_at
            FROM identities
            WHERE did = $1
            "#,
        )
        .bind(did)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::QueryFailed { source: e })?;

        Ok(row.map(|row| Identity {
            did: row.get("did"),
            handle: row.get("handle"),
            document: row.get("document"),
            created_at: row.get("created_at"),
            updated_at: row.get("updated_at"),
        }))
    }

    async fn upsert_identity(&self, identity: &Identity) -> StorageResult<()> {
        sqlx::query(
            r#"
            INSERT INTO identities (did, handle, document, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5)
            ON CONFLICT (did) DO UPDATE SET
                handle = EXCLUDED.handle,
                document = EXCLUDED.document,
                updated_at = EXCLUDED.updated_at
            "#,
        )
        .bind(&identity.did)
        .bind(&identity.handle)
        .bind(&identity.document)
        .bind(identity.created_at)
        .bind(identity.updated_at)
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::QueryFailed { source: e })?;

        Ok(())
    }

    async fn delete_identity(&self, did: &str) -> StorageResult<()> {
        sqlx::query("DELETE FROM identities WHERE did = $1")
            .bind(did)
            .execute(&self.pool)
            .await
            .map_err(|e| StorageError::QueryFailed { source: e })?;

        Ok(())
    }
}
