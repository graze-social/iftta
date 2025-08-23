use super::{Node, NodeCache, NodeStorage, StorageResult};
use async_trait::async_trait;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, instrument};

/// A wrapper around NodeStorage that adds caching capabilities
pub struct CachedNodeStorage<S: NodeStorage, C: NodeCache> {
    storage: Arc<S>,
    cache: Arc<C>,
    ttl: Duration,
}

impl<S: NodeStorage, C: NodeCache> CachedNodeStorage<S, C> {
    /// Create a new cached storage with the given backend and cache
    pub fn new(storage: Arc<S>, cache: Arc<C>, ttl: Duration) -> Self {
        Self {
            storage,
            cache,
            ttl,
        }
    }

    /// Create with default TTL of 60 seconds
    pub fn with_default_ttl(storage: Arc<S>, cache: Arc<C>) -> Self {
        Self::new(storage, cache, Duration::from_secs(60))
    }
}

#[async_trait]
impl<S: NodeStorage, C: NodeCache> NodeStorage for CachedNodeStorage<S, C> {
    #[instrument(skip(self), fields(node.aturi = %aturi))]
    async fn get_node(&self, aturi: &str) -> StorageResult<Option<Node>> {
        // Try cache first
        if let Ok(Some(node)) = self.cache.get(aturi).await {
            debug!("Node cache hit");
            return Ok(Some(node));
        }

        debug!("Node cache miss, fetching from storage");

        // Fetch from storage
        let node = self.storage.get_node(aturi).await?;

        // Cache the result if found
        if let Some(ref n) = node
            && let Err(e) = self.cache.set(aturi, n.clone(), self.ttl).await
        {
            debug!(error = ?e, "Failed to cache node");
        }

        Ok(node)
    }

    #[instrument(skip(self), fields(node.blueprint = %blueprint))]
    async fn list_nodes(&self, blueprint: &str) -> StorageResult<Vec<Node>> {
        // Try cache first
        if let Ok(Some(nodes)) = self.cache.get_by_blueprint(blueprint).await {
            debug!("Blueprint nodes cache hit");
            return Ok(nodes);
        }

        debug!("Blueprint nodes cache miss, fetching from storage");

        // Fetch from storage
        let nodes = self.storage.list_nodes(blueprint).await?;

        // Cache the result
        if !nodes.is_empty()
            && let Err(e) = self
                .cache
                .set_by_blueprint(blueprint, nodes.clone(), self.ttl)
                .await
        {
            debug!(error = ?e, "Failed to cache blueprint nodes");
        }

        Ok(nodes)
    }

    #[instrument(skip(self, node), fields(
        node.aturi = %node.aturi,
        node.type = %node.node_type,
        node.blueprint = %node.blueprint
    ))]
    async fn upsert_node(&self, node: &Node) -> StorageResult<()> {
        // Perform the upsert
        self.storage.upsert_node(node).await?;

        // Invalidate cache entries for both the node and its blueprint
        if let Err(e) = self.cache.invalidate(&node.aturi).await {
            debug!(error = ?e, "Failed to invalidate node cache");
        }

        if let Err(e) = self.cache.invalidate_by_blueprint(&node.blueprint).await {
            debug!(error = ?e, "Failed to invalidate blueprint cache");
        }

        debug!("Node upserted and cache invalidated");

        Ok(())
    }

    #[instrument(skip(self), fields(node.aturi = %aturi))]
    async fn delete_node(&self, aturi: &str) -> StorageResult<()> {
        // Get the node first to find its blueprint for cache invalidation
        let node = self.storage.get_node(aturi).await?;

        // Perform the deletion
        self.storage.delete_node(aturi).await?;

        // Invalidate cache entries
        if let Err(e) = self.cache.invalidate(aturi).await {
            debug!(error = ?e, "Failed to invalidate node cache");
        }

        // If we found the node, invalidate its blueprint cache too
        if let Some(node) = node
            && let Err(e) = self.cache.invalidate_by_blueprint(&node.blueprint).await
        {
            debug!(error = ?e, "Failed to invalidate blueprint cache");
        }

        debug!("Node deleted and cache invalidated");

        Ok(())
    }

    #[instrument(skip(self), fields(node.blueprint = %blueprint))]
    async fn delete_nodes_for_blueprint(&self, blueprint: &str) -> StorageResult<()> {
        // Perform the deletion
        self.storage.delete_nodes_for_blueprint(blueprint).await?;

        // Invalidate blueprint cache
        if let Err(e) = self.cache.invalidate_by_blueprint(blueprint).await {
            debug!(error = ?e, "Failed to invalidate blueprint cache");
        }

        debug!("Blueprint nodes deleted and cache invalidated");

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{InMemoryNodeCache, PostgresNodeStorage};
    use chrono::Utc;
    use sqlx::PgPool;

    async fn create_test_storage(
        pool: PgPool,
    ) -> CachedNodeStorage<PostgresNodeStorage, InMemoryNodeCache> {
        let storage = Arc::new(PostgresNodeStorage::new(pool));
        let cache = Arc::new(InMemoryNodeCache::new(100, Duration::from_secs(60)));
        CachedNodeStorage::new(storage, cache, Duration::from_secs(60))
    }

    #[tokio::test]
    #[ignore] // Requires database connection
    async fn test_cached_node_storage() {
        // This test requires a PostgreSQL database to be available
        // Set DATABASE_URL environment variable to run this test

        let db_url =
            std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for integration tests");

        let pool = PgPool::connect(&db_url)
            .await
            .expect("Failed to connect to database");

        let cached_storage = create_test_storage(pool).await;

        let node = Node {
            aturi: format!("at://did:plc:test/app.bsky.node/{}", uuid::Uuid::new_v4()),
            blueprint: "at://did:plc:test/app.bsky.blueprint/test".to_string(),
            node_type: "test_node".to_string(),
            payload: serde_json::json!({"test": "data"}),
            configuration: serde_json::json!({"config": "value"}),
            created_at: Utc::now(),
        };

        // First get should miss cache and fetch from DB
        let result1 = cached_storage.get_node(&node.aturi).await.unwrap();
        assert!(result1.is_none());

        // Upsert the node
        cached_storage.upsert_node(&node).await.unwrap();

        // Get should now return the node (might be from cache or DB)
        let result2 = cached_storage.get_node(&node.aturi).await.unwrap();
        assert!(result2.is_some());
        assert_eq!(result2.unwrap().aturi, node.aturi);

        // Second get should hit cache (though we can't verify this directly)
        let result3 = cached_storage.get_node(&node.aturi).await.unwrap();
        assert!(result3.is_some());

        // Delete the node
        cached_storage.delete_node(&node.aturi).await.unwrap();

        // Should no longer exist
        let result4 = cached_storage.get_node(&node.aturi).await.unwrap();
        assert!(result4.is_none());
    }

    #[tokio::test]
    #[ignore] // Requires database connection
    async fn test_cached_blueprint_nodes() {
        let db_url =
            std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for integration tests");

        let pool = PgPool::connect(&db_url)
            .await
            .expect("Failed to connect to database");

        let cached_storage = create_test_storage(pool).await;

        let blueprint = format!(
            "at://did:plc:test/app.bsky.blueprint/{}",
            uuid::Uuid::new_v4()
        );

        let node1 = Node {
            aturi: format!("at://did:plc:test/app.bsky.node/{}", uuid::Uuid::new_v4()),
            blueprint: blueprint.clone(),
            node_type: "test_node".to_string(),
            payload: serde_json::json!({"id": 1}),
            configuration: serde_json::json!({}),
            created_at: Utc::now(),
        };

        let node2 = Node {
            aturi: format!("at://did:plc:test/app.bsky.node/{}", uuid::Uuid::new_v4()),
            blueprint: blueprint.clone(),
            node_type: "test_node".to_string(),
            payload: serde_json::json!({"id": 2}),
            configuration: serde_json::json!({}),
            created_at: Utc::now(),
        };

        // Insert nodes
        cached_storage.upsert_node(&node1).await.unwrap();
        cached_storage.upsert_node(&node2).await.unwrap();

        // List nodes for blueprint (first call might miss cache)
        let nodes1 = cached_storage.list_nodes(&blueprint).await.unwrap();
        assert_eq!(nodes1.len(), 2);

        // Second call should potentially hit cache
        let nodes2 = cached_storage.list_nodes(&blueprint).await.unwrap();
        assert_eq!(nodes2.len(), 2);

        // Delete all nodes for blueprint
        cached_storage
            .delete_nodes_for_blueprint(&blueprint)
            .await
            .unwrap();

        // Should return empty list
        let nodes3 = cached_storage.list_nodes(&blueprint).await.unwrap();
        assert_eq!(nodes3.len(), 0);
    }
}
