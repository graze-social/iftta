use anyhow::Result;
use async_trait::async_trait;
use moka::future::Cache;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use super::{Node, NodeCache, NodeCacheStats};

/// In-memory node cache implementation using moka
pub struct InMemoryNodeCache {
    cache: Cache<String, Node>,
    blueprint_cache: Cache<String, Vec<Node>>,
    hits: Arc<AtomicU64>,
    misses: Arc<AtomicU64>,
}

impl InMemoryNodeCache {
    /// Create a new in-memory cache with specified capacity and default TTL
    pub fn new(max_capacity: u64, default_ttl: Duration) -> Self {
        let cache = Cache::builder()
            .max_capacity(max_capacity)
            .time_to_live(default_ttl)
            .build();

        let blueprint_cache = Cache::builder()
            .max_capacity(max_capacity / 10) // Fewer blueprint entries expected
            .time_to_live(default_ttl)
            .build();

        Self {
            cache,
            blueprint_cache,
            hits: Arc::new(AtomicU64::new(0)),
            misses: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Create with default settings (1000 entries, 60 second TTL)
    pub fn with_defaults() -> Self {
        Self::new(1000, Duration::from_secs(60))
    }
}

#[async_trait]
impl NodeCache for InMemoryNodeCache {
    async fn get(&self, aturi: &str) -> Result<Option<Node>> {
        let result = self.cache.get(aturi).await;

        if result.is_some() {
            self.hits.fetch_add(1, Ordering::Relaxed);
        } else {
            self.misses.fetch_add(1, Ordering::Relaxed);
        }

        Ok(result)
    }

    async fn get_by_blueprint(&self, blueprint: &str) -> Result<Option<Vec<Node>>> {
        let result = self.blueprint_cache.get(blueprint).await;

        if result.is_some() {
            self.hits.fetch_add(1, Ordering::Relaxed);
        } else {
            self.misses.fetch_add(1, Ordering::Relaxed);
        }

        Ok(result)
    }

    async fn set(&self, aturi: &str, node: Node, _ttl: Duration) -> Result<()> {
        // Also invalidate blueprint cache when a node is updated
        let blueprint = node.blueprint.clone();
        self.blueprint_cache.remove(&blueprint).await;

        // Insert into cache - moka will handle TTL based on cache configuration
        self.cache.insert(aturi.to_string(), node).await;

        Ok(())
    }

    async fn set_by_blueprint(
        &self,
        blueprint: &str,
        nodes: Vec<Node>,
        _ttl: Duration,
    ) -> Result<()> {
        // Also update individual node entries
        for node in &nodes {
            self.cache.insert(node.aturi.clone(), node.clone()).await;
        }

        // Insert blueprint list into cache
        self.blueprint_cache
            .insert(blueprint.to_string(), nodes)
            .await;

        Ok(())
    }

    async fn invalidate(&self, aturi: &str) -> Result<()> {
        // Get the node to find its blueprint
        if let Some(node) = self.cache.get(aturi).await {
            // Invalidate the blueprint cache entry
            self.blueprint_cache.remove(&node.blueprint).await;
        }

        self.cache.remove(aturi).await;
        Ok(())
    }

    async fn invalidate_by_blueprint(&self, blueprint: &str) -> Result<()> {
        // Remove blueprint cache entry
        self.blueprint_cache.remove(blueprint).await;

        // Also remove all individual nodes for this blueprint
        // Note: This is a best-effort approach since we don't track all nodes by blueprint
        // The cache TTL will eventually clean up any orphaned entries

        Ok(())
    }

    async fn clear(&self) -> Result<()> {
        self.cache.invalidate_all();
        self.blueprint_cache.invalidate_all();
        self.cache.run_pending_tasks().await;
        self.blueprint_cache.run_pending_tasks().await;
        Ok(())
    }

    async fn stats(&self) -> NodeCacheStats {
        NodeCacheStats {
            hits: self.hits.load(Ordering::Relaxed),
            misses: self.misses.load(Ordering::Relaxed),
            entries: self.cache.entry_count() + self.blueprint_cache.entry_count(),
            evictions: 0, // moka doesn't expose eviction count directly
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::Node;
    use chrono::Utc;

    #[tokio::test]
    async fn test_in_memory_cache_basic_operations() {
        let cache = InMemoryNodeCache::new(10, Duration::from_secs(60));

        let node = Node {
            aturi: "at://did:plc:test/app.bsky.node/123".to_string(),
            blueprint: "at://did:plc:test/app.bsky.blueprint/456".to_string(),
            node_type: "test_node".to_string(),
            payload: serde_json::json!({"test": "data"}),
            configuration: serde_json::json!({"config": "value"}),
            created_at: Utc::now(),
        };

        // Cache miss
        assert!(cache.get(&node.aturi).await.unwrap().is_none());

        // Set value
        cache
            .set(&node.aturi, node.clone(), Duration::from_secs(60))
            .await
            .unwrap();

        // Cache hit
        let cached = cache.get(&node.aturi).await.unwrap();
        assert!(cached.is_some());
        assert_eq!(cached.unwrap().blueprint, node.blueprint);

        // Invalidate
        cache.invalidate(&node.aturi).await.unwrap();
        assert!(cache.get(&node.aturi).await.unwrap().is_none());

        // Check stats
        let stats = cache.stats().await;
        assert_eq!(stats.hits, 1);
        assert_eq!(stats.misses, 2);
    }

    #[tokio::test]
    async fn test_in_memory_cache_blueprint_operations() {
        let cache = InMemoryNodeCache::new(20, Duration::from_secs(60));

        let blueprint = "at://did:plc:test/app.bsky.blueprint/789".to_string();
        let nodes = vec![
            Node {
                aturi: "at://did:plc:test/app.bsky.node/1".to_string(),
                blueprint: blueprint.clone(),
                node_type: "test_node".to_string(),
                payload: serde_json::json!({"id": 1}),
                configuration: serde_json::json!({}),
                created_at: Utc::now(),
            },
            Node {
                aturi: "at://did:plc:test/app.bsky.node/2".to_string(),
                blueprint: blueprint.clone(),
                node_type: "test_node".to_string(),
                payload: serde_json::json!({"id": 2}),
                configuration: serde_json::json!({}),
                created_at: Utc::now(),
            },
        ];

        // Cache miss for blueprint
        assert!(cache.get_by_blueprint(&blueprint).await.unwrap().is_none());

        // Set blueprint nodes
        cache
            .set_by_blueprint(&blueprint, nodes.clone(), Duration::from_secs(60))
            .await
            .unwrap();

        // Cache hit for blueprint
        let cached = cache.get_by_blueprint(&blueprint).await.unwrap();
        assert!(cached.is_some());
        assert_eq!(cached.unwrap().len(), 2);

        // Individual nodes should also be cached
        assert!(cache.get(&nodes[0].aturi).await.unwrap().is_some());
        assert!(cache.get(&nodes[1].aturi).await.unwrap().is_some());

        // Invalidate by blueprint
        cache.invalidate_by_blueprint(&blueprint).await.unwrap();
        assert!(cache.get_by_blueprint(&blueprint).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_in_memory_cache_ttl() {
        // Create cache with very short TTL for testing
        let cache = InMemoryNodeCache::new(10, Duration::from_millis(100));

        let node = Node {
            aturi: "at://did:plc:test/app.bsky.node/ttl".to_string(),
            blueprint: "at://did:plc:test/app.bsky.blueprint/ttl".to_string(),
            node_type: "test_node".to_string(),
            payload: serde_json::json!({"test": "ttl"}),
            configuration: serde_json::json!({}),
            created_at: Utc::now(),
        };

        // Set node (will use cache's configured TTL)
        cache
            .set(&node.aturi, node.clone(), Duration::from_millis(100))
            .await
            .unwrap();

        // Should exist immediately
        assert!(cache.get(&node.aturi).await.unwrap().is_some());

        // Wait for TTL to expire
        tokio::time::sleep(Duration::from_millis(200)).await;

        // Force cache to run maintenance to evict expired entries
        cache.cache.run_pending_tasks().await;

        // Should be expired
        assert!(cache.get(&node.aturi).await.unwrap().is_none());
    }
}
