use super::StorageResult;
use crate::constants::{is_action_node_type, is_entry_node_type};
use crate::errors::StorageError;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use std::sync::Arc;
use tracing::{Instrument, error};

/// A blueprint defines an automation workflow for processing events.
///
/// Blueprints are the core data structure for automation rules in the ifthisthenat service.
/// Each blueprint consists of an ordered list of nodes that define how to process incoming
/// events, from entry conditions through transformations to final actions.
///
/// # Structure
///
/// A valid blueprint must:
/// 1. Have at least 2 nodes
/// 2. Start with an entry node (jetstream_entry, webhook_entry, or periodic_entry)
/// 3. Contain at least one action node (publish_record, publish_webhook, debug_action)
///
/// # Node Execution Order
///
/// Nodes are executed in the order specified by `node_order`. If any node returns `None`
/// or fails, execution stops immediately. This allows for early filtering and conditional
/// execution.
///
/// # Examples
///
/// ```rust,ignore
/// use ifthisthenat::storage::blueprint::Blueprint;
/// use chrono::Utc;
///
/// let blueprint = Blueprint {
///     aturi: "at://did:web:example.com/com.example.blueprint/my-automation".to_string(),
///     did: "did:web:example.com".to_string(),
///     node_order: vec![
///         "entry-node".to_string(),
///         "condition-node".to_string(),
///         "action-node".to_string(),
///     ],
///     enabled: true,
///     error: None,
///     created_at: Utc::now(),
/// };
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Blueprint {
    /// AT-URI that uniquely identifies this blueprint.
    ///
    /// Format: `at://[authority]/[collection]/[record_key]`
    /// Example: `at://did:web:example.com/com.example.blueprint/my-automation`
    pub aturi: String,

    /// DID of the user who owns this blueprint.
    ///
    /// This determines access control and resource limits for the blueprint.
    pub did: String,

    /// Ordered list of node AT-URIs defining execution sequence.
    ///
    /// Nodes are executed in this exact order. The first node must be an entry node,
    /// and at least one node must be an action node.
    pub node_order: Vec<String>,

    /// Whether this blueprint is currently enabled for execution.
    ///
    /// Disabled blueprints are ignored during event processing.
    #[serde(default = "default_enabled")]
    pub enabled: bool,

    /// Error message if the blueprint is in an error state.
    ///
    /// When a blueprint encounters repeated failures, it may be automatically
    /// disabled with an error message describing the issue.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,

    /// Timestamp when this blueprint was created.
    pub created_at: DateTime<Utc>,
}

fn default_enabled() -> bool {
    true
}

impl Blueprint {
    /// Check if the blueprint is valid
    /// A valid blueprint has at least two nodes:
    /// 1. First node must be an entry node (jetstream_entry or webhook_entry)
    /// 2. Must contain at least one action node (publish_record)
    pub fn is_valid(&self, nodes: &[super::node::Node]) -> bool {
        // Must have at least 2 nodes
        if self.node_order.len() < 2 || nodes.len() < 2 {
            return false;
        }

        // First node must be an entry node
        if let Some(first_node_aturi) = self.node_order.first() {
            if let Some(first_node) = nodes.iter().find(|n| &n.aturi == first_node_aturi) {
                if !is_entry_node_type(&first_node.node_type) {
                    return false;
                }
            } else {
                return false; // First node not found
            }
        } else {
            return false; // No nodes in order
        }

        // Must have at least one action node (publish_record)
        self.node_order.iter().any(|node_aturi| {
            nodes
                .iter()
                .any(|node| &node.aturi == node_aturi && is_action_node_type(&node.node_type))
        })
    }
}

/// Storage trait for blueprint persistence and retrieval operations.
///
/// This trait defines the interface for storing and managing blueprints in the
/// persistent storage layer. Implementations typically use PostgreSQL or other
/// database backends to provide durable storage for blueprint configurations.
///
/// # Thread Safety
///
/// All implementations must be `Send + Sync` to work properly in the async
/// runtime environment with multiple concurrent operations.
///
/// # Access Control
///
/// Most operations support filtering by DID to implement proper access control.
/// Users should only be able to access blueprints they own or have explicit
/// permission to view.
///
/// # Error Handling
///
/// All methods return [`StorageResult<T>`] which wraps potential [`StorageError`]s.
/// Common error scenarios include:
/// - Network or database connectivity issues
/// - Invalid blueprint data or constraints violations
/// - Access control violations
/// - Resource not found errors
///
/// # Examples
///
/// ```rust,ignore
/// use ifthisthenat::storage::blueprint::{Blueprint, BlueprintStorage};
///
/// async fn example_usage(storage: &dyn BlueprintStorage) -> Result<(), Box<dyn std::error::Error>> {
///     // Get a specific blueprint
///     if let Some(blueprint) = storage.get_blueprint("at://did:web:example.com/blueprint/test").await? {
///         println!("Found blueprint: {}", blueprint.aturi);
///     }
///
///     // List all blueprints for a user
///     let user_blueprints = storage.list_blueprints(Some("did:web:example.com")).await?;
///     println!("User has {} blueprints", user_blueprints.len());
///
///     Ok(())
/// }
/// ```
#[async_trait]
pub trait BlueprintStorage: Send + Sync {
    /// Retrieves a blueprint by its AT-URI.
    ///
    /// # Arguments
    ///
    /// * `aturi` - The AT-URI of the blueprint to retrieve
    ///
    /// # Returns
    ///
    /// * `Ok(Some(blueprint))` - Blueprint found and retrieved
    /// * `Ok(None)` - No blueprint found with the given AT-URI
    /// * `Err(StorageError)` - Database or other storage error occurred
    async fn get_blueprint(&self, aturi: &str) -> StorageResult<Option<Blueprint>>;

    /// Lists blueprints, optionally filtered by owner DID.
    ///
    /// # Arguments
    ///
    /// * `did` - Optional DID to filter blueprints by owner. If `None`, returns all blueprints.
    ///
    /// # Returns
    ///
    /// * `Ok(Vec<Blueprint>)` - List of matching blueprints
    /// * `Err(StorageError)` - Database or other storage error occurred
    ///
    /// # Performance Note
    ///
    /// For large datasets, consider using [`list_blueprints_paginated`] instead.
    async fn list_blueprints(&self, did: Option<&str>) -> StorageResult<Vec<Blueprint>>;

    /// Lists blueprints with pagination support.
    ///
    /// This method provides paginated access to blueprints, which is essential
    /// for handling large datasets efficiently.
    ///
    /// # Arguments
    ///
    /// * `did` - Optional DID to filter blueprints by owner
    /// * `limit` - Maximum number of blueprints to return
    /// * `offset` - Number of blueprints to skip (for pagination)
    ///
    /// # Returns
    ///
    /// * `Ok(Vec<Blueprint>)` - List of blueprints for the requested page
    /// * `Err(StorageError)` - Database or other storage error occurred
    async fn list_blueprints_paginated(
        &self,
        did: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> StorageResult<Vec<Blueprint>>;

    /// Counts the total number of blueprints, optionally filtered by owner DID.
    ///
    /// This is useful for implementing pagination by determining the total number
    /// of pages available.
    ///
    /// # Arguments
    ///
    /// * `did` - Optional DID to filter blueprints by owner
    ///
    /// # Returns
    ///
    /// * `Ok(count)` - Total number of matching blueprints
    /// * `Err(StorageError)` - Database or other storage error occurred
    async fn count_blueprints(&self, did: Option<&str>) -> StorageResult<u32>;

    /// Creates a new blueprint in storage.
    ///
    /// # Arguments
    ///
    /// * `blueprint` - The blueprint to create
    ///
    /// # Returns
    ///
    /// * `Ok(())` - Blueprint created successfully
    /// * `Err(StorageError)` - Creation failed (e.g., duplicate AT-URI, validation error)
    ///
    /// # Errors
    ///
    /// This method will return an error if:
    /// - A blueprint with the same AT-URI already exists
    /// - The blueprint data is invalid
    /// - Database constraints are violated
    async fn create_blueprint(&self, blueprint: &Blueprint) -> StorageResult<()>;

    /// Updates an existing blueprint in storage.
    ///
    /// # Arguments
    ///
    /// * `blueprint` - The blueprint with updated data
    ///
    /// # Returns
    ///
    /// * `Ok(())` - Blueprint updated successfully
    /// * `Err(StorageError)` - Update failed (e.g., blueprint not found, validation error)
    ///
    /// # Behavior
    ///
    /// The blueprint is identified by its AT-URI field. All other fields are updated
    /// to match the provided blueprint data.
    async fn update_blueprint(&self, blueprint: &Blueprint) -> StorageResult<()>;

    /// Deletes a blueprint from storage.
    ///
    /// # Arguments
    ///
    /// * `aturi` - The AT-URI of the blueprint to delete
    ///
    /// # Returns
    ///
    /// * `Ok(())` - Blueprint deleted successfully (or didn't exist)
    /// * `Err(StorageError)` - Deletion failed due to database error
    ///
    /// # Note
    ///
    /// This method should also clean up associated nodes and other related data.
    async fn delete_blueprint(&self, aturi: &str) -> StorageResult<()>;

    /// Updates the error state and enabled status of a blueprint.
    ///
    /// This method is used by the automation system to disable blueprints that
    /// encounter repeated failures and to record error information for debugging.
    ///
    /// # Arguments
    ///
    /// * `aturi` - The AT-URI of the blueprint to update
    /// * `enabled` - New enabled status for the blueprint
    /// * `error` - Optional error message (None to clear existing error)
    ///
    /// # Returns
    ///
    /// * `Ok(())` - Blueprint error state updated successfully
    /// * `Err(StorageError)` - Update failed (e.g., blueprint not found)
    ///
    /// # Use Cases
    ///
    /// - Automatically disable blueprints after repeated failures
    /// - Clear error states when blueprints are manually re-enabled
    /// - Record diagnostic information for troubleshooting
    async fn update_blueprint_error(
        &self,
        aturi: &str,
        enabled: bool,
        error: Option<String>,
    ) -> StorageResult<()>;
}

#[async_trait]
impl<T: BlueprintStorage + ?Sized> BlueprintStorage for Arc<T> {
    async fn get_blueprint(&self, aturi: &str) -> StorageResult<Option<Blueprint>> {
        self.as_ref().get_blueprint(aturi).await
    }

    async fn list_blueprints(&self, did: Option<&str>) -> StorageResult<Vec<Blueprint>> {
        self.as_ref().list_blueprints(did).await
    }

    async fn list_blueprints_paginated(
        &self,
        did: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> StorageResult<Vec<Blueprint>> {
        self.as_ref()
            .list_blueprints_paginated(did, limit, offset)
            .await
    }

    async fn count_blueprints(&self, did: Option<&str>) -> StorageResult<u32> {
        self.as_ref().count_blueprints(did).await
    }

    async fn create_blueprint(&self, blueprint: &Blueprint) -> StorageResult<()> {
        self.as_ref().create_blueprint(blueprint).await
    }

    async fn update_blueprint(&self, blueprint: &Blueprint) -> StorageResult<()> {
        self.as_ref().update_blueprint(blueprint).await
    }

    async fn delete_blueprint(&self, aturi: &str) -> StorageResult<()> {
        self.as_ref().delete_blueprint(aturi).await
    }

    async fn update_blueprint_error(
        &self,
        aturi: &str,
        enabled: bool,
        error: Option<String>,
    ) -> StorageResult<()> {
        self.as_ref()
            .update_blueprint_error(aturi, enabled, error)
            .await
    }
}

pub struct PostgresBlueprintStorage {
    pool: PgPool,
}

impl PostgresBlueprintStorage {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl BlueprintStorage for PostgresBlueprintStorage {
    // Removed #[instrument] to prevent sharded_slab accumulation on hot path
    async fn get_blueprint(&self, aturi: &str) -> StorageResult<Option<Blueprint>> {
        let span = tracing::debug_span!("database_query", query = "SELECT blueprint");

        let row = sqlx::query(
            r#"
            SELECT aturi, did, node_order, enabled, error, created_at
            FROM blueprints
            WHERE aturi = $1
            "#,
        )
        .bind(aturi)
        .fetch_optional(&self.pool)
        .instrument(span)
        .await
        .map_err(|e| {
            error!(error = ?e, aturi = %aturi, "Failed to get blueprint");
            StorageError::QueryFailed { source: e }
        })?;

        Ok(row.map(|row| Blueprint {
            aturi: row.get("aturi"),
            did: row.get("did"),
            node_order: row
                .get::<serde_json::Value, _>("node_order")
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default(),
            enabled: row.get("enabled"),
            error: row.get("error"),
            created_at: row.get("created_at"),
        }))
    }

    // Removed #[instrument] to prevent sharded_slab accumulation on hot path
    // This function is called frequently (every 30s) by load_blueprints_cache
    async fn list_blueprints(&self, did: Option<&str>) -> StorageResult<Vec<Blueprint>> {
        let span = tracing::debug_span!("database_query", query = "SELECT blueprints");

        let query = if let Some(did) = did {
            sqlx::query(
                r#"
                SELECT aturi, did, node_order, enabled, error, created_at
                FROM blueprints
                WHERE did = $1
                ORDER BY created_at DESC
                "#,
            )
            .bind(did)
        } else {
            sqlx::query(
                r#"
                SELECT aturi, did, node_order, enabled, error, created_at
                FROM blueprints
                ORDER BY created_at DESC
                "#,
            )
        };

        let rows = query
            .fetch_all(&self.pool)
            .instrument(span)
            .await
            .map_err(|e| {
                error!(error = ?e, "Failed to list blueprints");
                StorageError::QueryFailed { source: e }
            })?;

        Ok(rows
            .into_iter()
            .map(|row| Blueprint {
                aturi: row.get("aturi"),
                did: row.get("did"),
                node_order: row
                    .get::<serde_json::Value, _>("node_order")
                    .as_array()
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default(),
                enabled: row.get("enabled"),
                error: row.get("error"),
                created_at: row.get("created_at"),
            })
            .collect())
    }

    // Removed #[instrument] to prevent sharded_slab accumulation on hot path
    async fn list_blueprints_paginated(
        &self,
        did: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> StorageResult<Vec<Blueprint>> {
        let span = tracing::debug_span!("database_query", query = "SELECT blueprints LIMIT OFFSET");

        let query = if let Some(did) = did {
            sqlx::query(
                r#"
                SELECT aturi, did, node_order, enabled, error, created_at
                FROM blueprints
                WHERE did = $1
                ORDER BY created_at DESC
                LIMIT $2 OFFSET $3
                "#,
            )
            .bind(did)
            .bind(limit as i64)
            .bind(offset as i64)
        } else {
            sqlx::query(
                r#"
                SELECT aturi, did, node_order, enabled, error, created_at
                FROM blueprints
                ORDER BY created_at DESC
                LIMIT $1 OFFSET $2
                "#,
            )
            .bind(limit as i64)
            .bind(offset as i64)
        };

        let rows = query
            .fetch_all(&self.pool)
            .instrument(span)
            .await
            .map_err(|e| {
                error!(error = ?e, "Failed to list blueprints with pagination");
                StorageError::QueryFailed { source: e }
            })?;

        Ok(rows
            .into_iter()
            .map(|row| Blueprint {
                aturi: row.get("aturi"),
                did: row.get("did"),
                node_order: row
                    .get::<serde_json::Value, _>("node_order")
                    .as_array()
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default(),
                enabled: row.get("enabled"),
                error: row.get("error"),
                created_at: row.get("created_at"),
            })
            .collect())
    }

    // Removed #[instrument] to prevent sharded_slab accumulation on hot path
    async fn count_blueprints(&self, did: Option<&str>) -> StorageResult<u32> {
        let span = tracing::debug_span!("database_query", query = "COUNT blueprints");

        let query = if let Some(did) = did {
            sqlx::query_scalar("SELECT COUNT(*) FROM blueprints WHERE did = $1").bind(did)
        } else {
            sqlx::query_scalar("SELECT COUNT(*) FROM blueprints")
        };

        let count: i64 = query
            .fetch_one(&self.pool)
            .instrument(span)
            .await
            .map_err(|e| {
                error!(error = ?e, "Failed to count blueprints");
                StorageError::QueryFailed { source: e }
            })?;

        Ok(count as u32)
    }

    async fn create_blueprint(&self, blueprint: &Blueprint) -> StorageResult<()> {
        sqlx::query(
            r#"
            INSERT INTO blueprints (aturi, did, node_order, enabled, error, created_at)
            VALUES ($1, $2, $3, $4, $5, $6)
            ON CONFLICT (aturi) DO UPDATE SET
                did = EXCLUDED.did,
                node_order = EXCLUDED.node_order,
                enabled = EXCLUDED.enabled,
                error = EXCLUDED.error
            "#,
        )
        .bind(&blueprint.aturi)
        .bind(&blueprint.did)
        .bind(serde_json::to_value(&blueprint.node_order).unwrap_or(serde_json::json!([])))
        .bind(blueprint.enabled)
        .bind(&blueprint.error)
        .bind(blueprint.created_at)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            error!(error = ?e, aturi = %blueprint.aturi, "Failed to create blueprint");
            StorageError::QueryFailed { source: e }
        })?;

        Ok(())
    }

    async fn update_blueprint(&self, blueprint: &Blueprint) -> StorageResult<()> {
        sqlx::query(
            r#"
            UPDATE blueprints 
            SET node_order = $2, enabled = $3, error = $4
            WHERE aturi = $1
            "#,
        )
        .bind(&blueprint.aturi)
        .bind(serde_json::to_value(&blueprint.node_order).unwrap_or(serde_json::json!([])))
        .bind(blueprint.enabled)
        .bind(&blueprint.error)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            error!(error = ?e, aturi = %blueprint.aturi, "Failed to update blueprint");
            StorageError::QueryFailed { source: e }
        })?;

        Ok(())
    }

    async fn delete_blueprint(&self, aturi: &str) -> StorageResult<()> {
        sqlx::query("DELETE FROM blueprints WHERE aturi = $1")
            .bind(aturi)
            .execute(&self.pool)
            .await
            .map_err(|e| {
                error!(error = ?e, aturi = %aturi, "Failed to delete blueprint");
                StorageError::QueryFailed { source: e }
            })?;

        Ok(())
    }

    // Removed #[instrument] to prevent sharded_slab accumulation on hot path
    async fn update_blueprint_error(
        &self,
        aturi: &str,
        enabled: bool,
        error: Option<String>,
    ) -> StorageResult<()> {
        let span = tracing::debug_span!("database_query", query = "UPDATE blueprint error");

        sqlx::query(
            r#"
            UPDATE blueprints 
            SET enabled = $2, error = $3
            WHERE aturi = $1
            "#,
        )
        .bind(aturi)
        .bind(enabled)
        .bind(error)
        .execute(&self.pool)
        .instrument(span)
        .await
        .map_err(|e| {
            error!(error = ?e, aturi = %aturi, "Failed to update blueprint error status");
            StorageError::QueryFailed { source: e }
        })?;

        Ok(())
    }
}
