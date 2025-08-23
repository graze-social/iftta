//! Storage for tracking scheduler state and blueprint execution times.
//!
//! This module provides storage for tracking when blueprints with periodic_entry
//! nodes were last executed and when they should next be executed.

use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::collections::HashMap;
use std::sync::Arc;

/// State information for a scheduled blueprint
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchedulerState {
    /// The blueprint ID (aturi)
    pub blueprint_id: String,
    /// When the blueprint was last evaluated
    pub last_run: Option<DateTime<Utc>>,
    /// When the blueprint should next be evaluated
    pub next_run: DateTime<Utc>,
    /// The cron expression from the periodic_entry node
    pub cron_expression: String,
    /// Whether this blueprint is enabled
    pub enabled: bool,
}

/// Trait for scheduler state storage operations
#[async_trait]
pub trait SchedulerStateStorage: Send + Sync {
    /// Get the state for a specific blueprint
    async fn get_state(&self, blueprint_id: &str) -> Result<Option<SchedulerState>>;

    /// Save or update the state for a blueprint
    async fn save_state(&self, state: &SchedulerState) -> Result<()>;

    /// Get all states for blueprints that should be evaluated
    /// Returns blueprints where next_run <= now
    async fn get_ready_blueprints(&self) -> Result<Vec<SchedulerState>>;

    /// Get all scheduler states
    async fn get_all_states(&self) -> Result<Vec<SchedulerState>>;

    /// Delete state for a blueprint
    async fn delete_state(&self, blueprint_id: &str) -> Result<()>;

    /// Update just the last_run and next_run times
    async fn update_run_times(
        &self,
        blueprint_id: &str,
        last_run: DateTime<Utc>,
        next_run: DateTime<Utc>,
    ) -> Result<()>;
}

/// PostgreSQL implementation of scheduler state storage
pub struct PostgresSchedulerStateStorage {
    pool: Arc<PgPool>,
}

impl PostgresSchedulerStateStorage {
    /// Create a new PostgreSQL scheduler state storage
    pub fn new(pool: Arc<PgPool>) -> Self {
        Self { pool }
    }

    /// Initialize the database schema for scheduler state
    pub async fn initialize_schema(&self) -> Result<()> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS scheduler_state (
                blueprint_id TEXT PRIMARY KEY,
                last_run TIMESTAMPTZ,
                next_run TIMESTAMPTZ NOT NULL,
                cron_expression TEXT NOT NULL,
                enabled BOOLEAN NOT NULL DEFAULT true,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
            )
            "#,
        )
        .execute(self.pool.as_ref())
        .await?;

        // Create index for efficient queries
        sqlx::query(
            r#"
            CREATE INDEX IF NOT EXISTS idx_scheduler_state_next_run 
            ON scheduler_state (next_run) 
            WHERE enabled = true
            "#,
        )
        .execute(self.pool.as_ref())
        .await?;

        Ok(())
    }
}

#[async_trait]
impl SchedulerStateStorage for PostgresSchedulerStateStorage {
    async fn get_state(&self, blueprint_id: &str) -> Result<Option<SchedulerState>> {
        let row = sqlx::query_as::<_, SchedulerStateRow>(
            r#"
            SELECT blueprint_id, last_run, next_run, cron_expression, enabled
            FROM scheduler_state
            WHERE blueprint_id = $1
            "#,
        )
        .bind(blueprint_id)
        .fetch_optional(self.pool.as_ref())
        .await?;

        Ok(row.map(|r| SchedulerState {
            blueprint_id: r.blueprint_id,
            last_run: r.last_run,
            next_run: r.next_run,
            cron_expression: r.cron_expression,
            enabled: r.enabled,
        }))
    }

    async fn save_state(&self, state: &SchedulerState) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO scheduler_state (blueprint_id, last_run, next_run, cron_expression, enabled)
            VALUES ($1, $2, $3, $4, $5)
            ON CONFLICT (blueprint_id) DO UPDATE SET
                last_run = EXCLUDED.last_run,
                next_run = EXCLUDED.next_run,
                cron_expression = EXCLUDED.cron_expression,
                enabled = EXCLUDED.enabled,
                updated_at = NOW()
            "#,
        )
        .bind(&state.blueprint_id)
        .bind(state.last_run)
        .bind(state.next_run)
        .bind(&state.cron_expression)
        .bind(state.enabled)
        .execute(self.pool.as_ref())
        .await?;

        Ok(())
    }

    async fn get_ready_blueprints(&self) -> Result<Vec<SchedulerState>> {
        let now = Utc::now();
        let rows = sqlx::query_as::<_, SchedulerStateRow>(
            r#"
            SELECT blueprint_id, last_run, next_run, cron_expression, enabled
            FROM scheduler_state
            WHERE enabled = true AND next_run <= $1
            ORDER BY next_run ASC
            "#,
        )
        .bind(now)
        .fetch_all(self.pool.as_ref())
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| SchedulerState {
                blueprint_id: r.blueprint_id,
                last_run: r.last_run,
                next_run: r.next_run,
                cron_expression: r.cron_expression,
                enabled: r.enabled,
            })
            .collect())
    }

    async fn get_all_states(&self) -> Result<Vec<SchedulerState>> {
        let rows = sqlx::query_as::<_, SchedulerStateRow>(
            r#"
            SELECT blueprint_id, last_run, next_run, cron_expression, enabled
            FROM scheduler_state
            ORDER BY next_run ASC
            "#,
        )
        .fetch_all(self.pool.as_ref())
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| SchedulerState {
                blueprint_id: r.blueprint_id,
                last_run: r.last_run,
                next_run: r.next_run,
                cron_expression: r.cron_expression,
                enabled: r.enabled,
            })
            .collect())
    }

    async fn delete_state(&self, blueprint_id: &str) -> Result<()> {
        sqlx::query(
            r#"
            DELETE FROM scheduler_state
            WHERE blueprint_id = $1
            "#,
        )
        .bind(blueprint_id)
        .execute(self.pool.as_ref())
        .await?;

        Ok(())
    }

    async fn update_run_times(
        &self,
        blueprint_id: &str,
        last_run: DateTime<Utc>,
        next_run: DateTime<Utc>,
    ) -> Result<()> {
        sqlx::query(
            r#"
            UPDATE scheduler_state
            SET last_run = $2, next_run = $3, updated_at = NOW()
            WHERE blueprint_id = $1
            "#,
        )
        .bind(blueprint_id)
        .bind(last_run)
        .bind(next_run)
        .execute(self.pool.as_ref())
        .await?;

        Ok(())
    }
}

// Helper struct for database queries
#[derive(Debug, sqlx::FromRow)]
struct SchedulerStateRow {
    blueprint_id: String,
    last_run: Option<DateTime<Utc>>,
    next_run: DateTime<Utc>,
    cron_expression: String,
    enabled: bool,
}

/// In-memory implementation for testing
pub struct MemorySchedulerStateStorage {
    states: Arc<tokio::sync::RwLock<HashMap<String, SchedulerState>>>,
}

impl MemorySchedulerStateStorage {
    /// Create a new in-memory scheduler state storage
    pub fn new() -> Self {
        Self {
            states: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
        }
    }
}

impl Default for MemorySchedulerStateStorage {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl SchedulerStateStorage for MemorySchedulerStateStorage {
    async fn get_state(&self, blueprint_id: &str) -> Result<Option<SchedulerState>> {
        let states = self.states.read().await;
        Ok(states.get(blueprint_id).cloned())
    }

    async fn save_state(&self, state: &SchedulerState) -> Result<()> {
        let mut states = self.states.write().await;
        states.insert(state.blueprint_id.clone(), state.clone());
        Ok(())
    }

    async fn get_ready_blueprints(&self) -> Result<Vec<SchedulerState>> {
        let now = Utc::now();
        let states = self.states.read().await;
        Ok(states
            .values()
            .filter(|s| s.enabled && s.next_run <= now)
            .cloned()
            .collect())
    }

    async fn get_all_states(&self) -> Result<Vec<SchedulerState>> {
        let states = self.states.read().await;
        Ok(states.values().cloned().collect())
    }

    async fn delete_state(&self, blueprint_id: &str) -> Result<()> {
        let mut states = self.states.write().await;
        states.remove(blueprint_id);
        Ok(())
    }

    async fn update_run_times(
        &self,
        blueprint_id: &str,
        last_run: DateTime<Utc>,
        next_run: DateTime<Utc>,
    ) -> Result<()> {
        let mut states = self.states.write().await;
        if let Some(state) = states.get_mut(blueprint_id) {
            state.last_run = Some(last_run);
            state.next_run = next_run;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_memory_storage_basic_operations() {
        let storage = MemorySchedulerStateStorage::new();

        let state = SchedulerState {
            blueprint_id: "test_blueprint".to_string(),
            last_run: None,
            next_run: Utc::now(),
            cron_expression: "0 * * * * *".to_string(),
            enabled: true,
        };

        // Test save
        storage.save_state(&state).await.unwrap();

        // Test get
        let retrieved = storage.get_state("test_blueprint").await.unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().blueprint_id, "test_blueprint");

        // Test update run times
        let last_run = Utc::now();
        let next_run = Utc::now() + chrono::Duration::hours(1);
        storage
            .update_run_times("test_blueprint", last_run, next_run)
            .await
            .unwrap();

        let updated = storage.get_state("test_blueprint").await.unwrap().unwrap();
        assert_eq!(updated.last_run, Some(last_run));
        assert_eq!(updated.next_run, next_run);

        // Test delete
        storage.delete_state("test_blueprint").await.unwrap();
        let deleted = storage.get_state("test_blueprint").await.unwrap();
        assert!(deleted.is_none());
    }

    #[tokio::test]
    async fn test_get_ready_blueprints() {
        let storage = MemorySchedulerStateStorage::new();
        let now = Utc::now();

        // Add blueprint that should run (past time)
        let ready_state = SchedulerState {
            blueprint_id: "ready_blueprint".to_string(),
            last_run: None,
            next_run: now - chrono::Duration::hours(1),
            cron_expression: "0 * * * * *".to_string(),
            enabled: true,
        };
        storage.save_state(&ready_state).await.unwrap();

        // Add blueprint that shouldn't run yet (future time)
        let future_state = SchedulerState {
            blueprint_id: "future_blueprint".to_string(),
            last_run: None,
            next_run: now + chrono::Duration::hours(1),
            cron_expression: "0 * * * * *".to_string(),
            enabled: true,
        };
        storage.save_state(&future_state).await.unwrap();

        // Add disabled blueprint
        let disabled_state = SchedulerState {
            blueprint_id: "disabled_blueprint".to_string(),
            last_run: None,
            next_run: now - chrono::Duration::hours(1),
            cron_expression: "0 * * * * *".to_string(),
            enabled: false,
        };
        storage.save_state(&disabled_state).await.unwrap();

        let ready = storage.get_ready_blueprints().await.unwrap();
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].blueprint_id, "ready_blueprint");
    }
}
