use super::StorageResult;
use crate::errors::StorageError;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use std::sync::Arc;
use tracing::{Instrument, debug, error, instrument};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    pub aturi: String,
    pub blueprint: String,
    pub node_type: String,
    pub payload: serde_json::Value,
    pub configuration: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

#[async_trait]
pub trait NodeStorage: Send + Sync {
    async fn get_node(&self, aturi: &str) -> StorageResult<Option<Node>>;
    async fn list_nodes(&self, blueprint: &str) -> StorageResult<Vec<Node>>;
    async fn upsert_node(&self, node: &Node) -> StorageResult<()>;
    async fn delete_node(&self, aturi: &str) -> StorageResult<()>;
    async fn delete_nodes_for_blueprint(&self, blueprint: &str) -> StorageResult<()>;
}

#[async_trait]
impl<T: NodeStorage + ?Sized> NodeStorage for Arc<T> {
    async fn get_node(&self, aturi: &str) -> StorageResult<Option<Node>> {
        self.as_ref().get_node(aturi).await
    }

    async fn list_nodes(&self, blueprint: &str) -> StorageResult<Vec<Node>> {
        self.as_ref().list_nodes(blueprint).await
    }

    async fn upsert_node(&self, node: &Node) -> StorageResult<()> {
        self.as_ref().upsert_node(node).await
    }

    async fn delete_node(&self, aturi: &str) -> StorageResult<()> {
        self.as_ref().delete_node(aturi).await
    }

    async fn delete_nodes_for_blueprint(&self, blueprint: &str) -> StorageResult<()> {
        self.as_ref().delete_nodes_for_blueprint(blueprint).await
    }
}

pub struct PostgresNodeStorage {
    pool: PgPool,
}

impl PostgresNodeStorage {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl NodeStorage for PostgresNodeStorage {
    #[instrument(skip(self), fields(db.operation = "get_node", node.aturi = %aturi))]
    async fn get_node(&self, aturi: &str) -> StorageResult<Option<Node>> {
        debug!("Fetching node from database");

        let span = tracing::debug_span!(
            "database_query",
            query = "SELECT node",
            table = "nodes",
            aturi = %aturi
        );

        let row = sqlx::query(
            r#"
            SELECT aturi, blueprint, type, payload, configuration, created_at
            FROM nodes
            WHERE aturi = $1
            "#,
        )
        .bind(aturi)
        .fetch_optional(&self.pool)
        .instrument(span)
        .await
        .map_err(|e| {
            error!(error = ?e, aturi = %aturi, "Failed to get node");
            StorageError::QueryFailed { source: e }
        })?;

        let result = row.map(|row| Node {
            aturi: row.get("aturi"),
            blueprint: row.get("blueprint"),
            node_type: row.get("type"),
            payload: row.get("payload"),
            configuration: row.get("configuration"),
            created_at: row.get("created_at"),
        });

        if result.is_some() {
            debug!("Node found");
        } else {
            debug!("Node not found");
        }

        Ok(result)
    }

    #[instrument(skip(self), fields(db.operation = "list_nodes", node.blueprint = %blueprint))]
    async fn list_nodes(&self, blueprint: &str) -> StorageResult<Vec<Node>> {
        debug!("Listing nodes for blueprint");

        let span = tracing::debug_span!(
            "database_query",
            query = "SELECT nodes",
            table = "nodes",
            blueprint = %blueprint
        );

        let rows = sqlx::query(
            r#"
            SELECT aturi, blueprint, type, payload, configuration, created_at
            FROM nodes
            WHERE blueprint = $1
            ORDER BY created_at
            "#,
        )
        .bind(blueprint)
        .fetch_all(&self.pool)
        .instrument(span)
        .await
        .map_err(|e| {
            error!(error = ?e, blueprint = %blueprint, "Failed to list nodes");
            StorageError::QueryFailed { source: e }
        })?;

        let nodes: Vec<Node> = rows
            .into_iter()
            .map(|row| Node {
                aturi: row.get("aturi"),
                blueprint: row.get("blueprint"),
                node_type: row.get("type"),
                payload: row.get("payload"),
                configuration: row.get("configuration"),
                created_at: row.get("created_at"),
            })
            .collect();

        debug!(count = nodes.len(), "Found nodes");

        Ok(nodes)
    }

    #[instrument(skip(self, node), fields(
        db.operation = "upsert_node",
        node.aturi = %node.aturi,
        node.type = %node.node_type,
        node.blueprint = %node.blueprint
    ))]
    async fn upsert_node(&self, node: &Node) -> StorageResult<()> {
        debug!("Upserting node");

        // Normalize the configuration and payload before storing
        let normalized_config =
            super::normalize_node_configuration(&node.node_type, &node.configuration).map_err(
                |e| {
                    error!(error = ?e, "Failed to normalize node configuration");
                    StorageError::InvalidInput {
                        details: format!("Failed to normalize configuration: {}", e),
                    }
                },
            )?;

        let normalized_payload =
            super::normalize_node_configuration(&node.node_type, &node.payload).map_err(|e| {
                error!(error = ?e, "Failed to normalize node payload");
                StorageError::InvalidInput {
                    details: format!("Failed to normalize payload: {}", e),
                }
            })?;

        let span = tracing::debug_span!(
            "database_query",
            query = "UPSERT node",
            table = "nodes",
            aturi = %node.aturi,
            node_type = %node.node_type
        );

        let result = sqlx::query(
            r#"
            INSERT INTO nodes (aturi, blueprint, type, payload, configuration, created_at)
            VALUES ($1, $2, $3, $4, $5, $6)
            ON CONFLICT (aturi) DO UPDATE SET
                blueprint = EXCLUDED.blueprint,
                type = EXCLUDED.type,
                payload = EXCLUDED.payload,
                configuration = EXCLUDED.configuration
            "#,
        )
        .bind(&node.aturi)
        .bind(&node.blueprint)
        .bind(&node.node_type)
        .bind(&normalized_payload)
        .bind(&normalized_config)
        .bind(node.created_at)
        .execute(&self.pool)
        .instrument(span)
        .await
        .map_err(|e| {
            error!(error = ?e, aturi = %node.aturi, "Failed to upsert node");
            StorageError::QueryFailed { source: e }
        })?;

        debug!(
            rows_affected = result.rows_affected(),
            "Node upserted successfully"
        );

        Ok(())
    }

    #[instrument(skip(self), fields(db.operation = "delete_node", node.aturi = %aturi))]
    async fn delete_node(&self, aturi: &str) -> StorageResult<()> {
        debug!("Deleting node");

        let span = tracing::debug_span!(
            "database_query",
            query = "DELETE node",
            table = "nodes",
            aturi = %aturi
        );

        let result = sqlx::query("DELETE FROM nodes WHERE aturi = $1")
            .bind(aturi)
            .execute(&self.pool)
            .instrument(span)
            .await
            .map_err(|e| {
                error!(error = ?e, aturi = %aturi, "Failed to delete node");
                StorageError::QueryFailed { source: e }
            })?;

        debug!(
            rows_affected = result.rows_affected(),
            "Node deletion completed"
        );

        Ok(())
    }

    #[instrument(skip(self), fields(db.operation = "delete_nodes_for_blueprint", node.blueprint = %blueprint))]
    async fn delete_nodes_for_blueprint(&self, blueprint: &str) -> StorageResult<()> {
        debug!("Deleting all nodes for blueprint");

        let span = tracing::debug_span!(
            "database_query",
            query = "DELETE nodes by blueprint",
            table = "nodes",
            blueprint = %blueprint
        );

        let result = sqlx::query("DELETE FROM nodes WHERE blueprint = $1")
            .bind(blueprint)
            .execute(&self.pool)
            .instrument(span)
            .await
            .map_err(|e| {
                error!(error = ?e, blueprint = %blueprint, "Failed to delete nodes for blueprint");
                StorageError::QueryFailed { source: e }
            })?;

        debug!(
            rows_affected = result.rows_affected(),
            "Deleted all nodes for blueprint"
        );

        Ok(())
    }
}
