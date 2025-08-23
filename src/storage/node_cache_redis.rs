use anyhow::Result;
use async_trait::async_trait;
use deadpool_redis::{Pool, redis::AsyncCommands};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tracing::{debug, warn};

use super::{Node, NodeCache, NodeCacheStats};

/// Redis-based node cache implementation using deadpool
pub struct RedisNodeCache {
    pool: Pool,
    key_prefix: String,
    blueprint_key_prefix: String,
    hits: Arc<AtomicU64>,
    misses: Arc<AtomicU64>,
    default_ttl: Duration,
}

impl RedisNodeCache {
    /// Create a new Redis cache with the given pool and configuration
    pub fn new(pool: Pool, key_prefix: String, default_ttl: Duration) -> Self {
        Self {
            pool,
            key_prefix: key_prefix.clone(),
            blueprint_key_prefix: format!("{}blueprint:", key_prefix),
            hits: Arc::new(AtomicU64::new(0)),
            misses: Arc::new(AtomicU64::new(0)),
            default_ttl,
        }
    }

    /// Create with default settings (60 second TTL)
    pub fn with_defaults(pool: Pool) -> Self {
        Self::new(pool, "node:cache:".to_string(), Duration::from_secs(60))
    }

    /// Generate the full Redis key for a node ATURI
    fn make_node_key(&self, aturi: &str) -> String {
        format!("{}{}", self.key_prefix, aturi)
    }

    /// Generate the full Redis key for blueprint nodes
    fn make_blueprint_key(&self, blueprint: &str) -> String {
        format!("{}{}", self.blueprint_key_prefix, blueprint)
    }
}

#[async_trait]
impl NodeCache for RedisNodeCache {
    async fn get(&self, aturi: &str) -> Result<Option<Node>> {
        let mut conn = self.pool.get().await?;
        let key = self.make_node_key(aturi);

        let data: Option<Vec<u8>> = conn.get(&key).await?;

        let result = match data {
            Some(bytes) => {
                match serde_json::from_slice::<Node>(&bytes) {
                    Ok(node) => {
                        self.hits.fetch_add(1, Ordering::Relaxed);
                        debug!(aturi = %aturi, "Node cache hit");
                        Some(node)
                    }
                    Err(e) => {
                        warn!(aturi = %aturi, error = ?e, "Failed to deserialize cached node");
                        self.misses.fetch_add(1, Ordering::Relaxed);
                        // Remove corrupted entry
                        let _: Result<(), _> = conn.del(&key).await;
                        None
                    }
                }
            }
            None => {
                self.misses.fetch_add(1, Ordering::Relaxed);
                debug!(aturi = %aturi, "Node cache miss");
                None
            }
        };

        Ok(result)
    }

    async fn get_by_blueprint(&self, blueprint: &str) -> Result<Option<Vec<Node>>> {
        let mut conn = self.pool.get().await?;
        let key = self.make_blueprint_key(blueprint);

        let data: Option<Vec<u8>> = conn.get(&key).await?;

        let result = match data {
            Some(bytes) => {
                match serde_json::from_slice::<Vec<Node>>(&bytes) {
                    Ok(nodes) => {
                        self.hits.fetch_add(1, Ordering::Relaxed);
                        debug!(blueprint = %blueprint, count = nodes.len(), "Blueprint nodes cache hit");
                        Some(nodes)
                    }
                    Err(e) => {
                        warn!(blueprint = %blueprint, error = ?e, "Failed to deserialize cached blueprint nodes");
                        self.misses.fetch_add(1, Ordering::Relaxed);
                        // Remove corrupted entry
                        let _: Result<(), _> = conn.del(&key).await;
                        None
                    }
                }
            }
            None => {
                self.misses.fetch_add(1, Ordering::Relaxed);
                debug!(blueprint = %blueprint, "Blueprint nodes cache miss");
                None
            }
        };

        Ok(result)
    }

    async fn set(&self, aturi: &str, node: Node, ttl: Duration) -> Result<()> {
        let mut conn = self.pool.get().await?;
        let key = self.make_node_key(aturi);

        // Also invalidate blueprint cache when a node is updated
        let blueprint_key = self.make_blueprint_key(&node.blueprint);
        let _: Result<(), _> = conn.del(&blueprint_key).await;

        let data = serde_json::to_vec(&node)?;
        let effective_ttl = if ttl.as_secs() > 0 {
            ttl
        } else {
            self.default_ttl
        };

        // SET with expiration in seconds
        let _: () = conn.set_ex(&key, data, effective_ttl.as_secs()).await?;

        debug!(aturi = %aturi, ttl_secs = effective_ttl.as_secs(), "Cached node");
        Ok(())
    }

    async fn set_by_blueprint(
        &self,
        blueprint: &str,
        nodes: Vec<Node>,
        ttl: Duration,
    ) -> Result<()> {
        let mut conn = self.pool.get().await?;
        let blueprint_key = self.make_blueprint_key(blueprint);

        // Also update individual node entries
        for node in &nodes {
            let node_key = self.make_node_key(&node.aturi);
            let node_data = serde_json::to_vec(node)?;
            let effective_ttl = if ttl.as_secs() > 0 {
                ttl
            } else {
                self.default_ttl
            };
            let _: () = conn
                .set_ex(&node_key, node_data, effective_ttl.as_secs())
                .await?;
        }

        // Store the blueprint nodes list
        let data = serde_json::to_vec(&nodes)?;
        let effective_ttl = if ttl.as_secs() > 0 {
            ttl
        } else {
            self.default_ttl
        };

        let _: () = conn
            .set_ex(&blueprint_key, data, effective_ttl.as_secs())
            .await?;

        debug!(blueprint = %blueprint, count = nodes.len(), ttl_secs = effective_ttl.as_secs(), "Cached blueprint nodes");
        Ok(())
    }

    async fn invalidate(&self, aturi: &str) -> Result<()> {
        let mut conn = self.pool.get().await?;
        let key = self.make_node_key(aturi);

        // Get the node to find its blueprint
        let data: Option<Vec<u8>> = conn.get(&key).await?;
        if let Some(bytes) = data
            && let Ok(node) = serde_json::from_slice::<Node>(&bytes)
        {
            // Invalidate the blueprint cache entry
            let blueprint_key = self.make_blueprint_key(&node.blueprint);
            let _: Result<(), _> = conn.del(&blueprint_key).await;
        }

        let deleted: u64 = conn.del(&key).await?;
        if deleted > 0 {
            debug!(aturi = %aturi, "Invalidated cached node");
        }

        Ok(())
    }

    async fn invalidate_by_blueprint(&self, blueprint: &str) -> Result<()> {
        let mut conn = self.pool.get().await?;
        let blueprint_key = self.make_blueprint_key(blueprint);

        // Get all nodes for this blueprint to invalidate them individually
        let data: Option<Vec<u8>> = conn.get(&blueprint_key).await?;
        if let Some(bytes) = data
            && let Ok(nodes) = serde_json::from_slice::<Vec<Node>>(&bytes)
        {
            // Delete all individual node entries
            let node_keys: Vec<String> = nodes
                .iter()
                .map(|node| self.make_node_key(&node.aturi))
                .collect();

            if !node_keys.is_empty() {
                let _: Result<u64, _> = conn.del(node_keys.as_slice()).await;
            }
        }

        // Delete the blueprint cache entry
        let deleted: u64 = conn.del(&blueprint_key).await?;
        if deleted > 0 {
            debug!(blueprint = %blueprint, "Invalidated cached blueprint nodes");
        }

        Ok(())
    }

    async fn clear(&self) -> Result<()> {
        let mut conn = self.pool.get().await?;

        // Clear node entries
        let node_pattern = format!("{}*", self.key_prefix);
        let node_keys: Vec<String> = {
            let mut iter = conn.scan_match::<_, String>(&node_pattern).await?;
            let mut keys = Vec::new();

            while let Some(key) = iter.next_item().await {
                keys.push(key);
            }
            keys
        };

        // Clear blueprint entries
        let blueprint_pattern = format!("{}*", self.blueprint_key_prefix);
        let blueprint_keys: Vec<String> = {
            let mut iter = conn.scan_match::<_, String>(&blueprint_pattern).await?;
            let mut keys = Vec::new();

            while let Some(key) = iter.next_item().await {
                keys.push(key);
            }
            keys
        };

        let mut total_deleted = 0u64;

        if !node_keys.is_empty() {
            let deleted: u64 = conn.del(node_keys.as_slice()).await?;
            total_deleted += deleted;
        }

        if !blueprint_keys.is_empty() {
            let deleted: u64 = conn.del(blueprint_keys.as_slice()).await?;
            total_deleted += deleted;
        }

        if total_deleted > 0 {
            debug!(count = total_deleted, "Cleared node cache entries");
        }

        Ok(())
    }

    async fn stats(&self) -> NodeCacheStats {
        let mut entries = 0;

        // Try to count keys with our prefixes
        if let Ok(mut conn) = self.pool.get().await {
            // Count node entries
            let node_pattern = format!("{}*", self.key_prefix);
            if let Ok(mut iter) = conn.scan_match::<_, String>(&node_pattern).await {
                while iter.next_item().await.is_some() {
                    entries += 1;
                }
            }

            // Count blueprint entries
            let blueprint_pattern = format!("{}*", self.blueprint_key_prefix);
            if let Ok(mut iter) = conn.scan_match::<_, String>(&blueprint_pattern).await {
                while iter.next_item().await.is_some() {
                    entries += 1;
                }
            }
        }

        NodeCacheStats {
            hits: self.hits.load(Ordering::Relaxed),
            misses: self.misses.load(Ordering::Relaxed),
            entries,
            evictions: 0, // Redis doesn't track this for us
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::Node;
    use crate::storage::cache::create_cache_pool;
    use chrono::Utc;

    async fn redis_available() -> bool {
        match std::env::var("TEST_REDIS_URL") {
            Ok(url) => create_cache_pool(&url).is_ok(),
            Err(_) => false,
        }
    }

    #[tokio::test]
    async fn test_redis_cache_basic_operations() {
        if !redis_available().await {
            eprintln!("Skipping test: Redis not available. Set TEST_REDIS_URL to enable.");
            return;
        }

        let redis_url = std::env::var("TEST_REDIS_URL").unwrap();
        let pool = create_cache_pool(&redis_url).unwrap();
        let cache = RedisNodeCache::new(
            pool.clone(),
            format!("test:node:cache:{}:", uuid::Uuid::new_v4()),
            Duration::from_secs(60),
        );

        let node = Node {
            aturi: "at://did:plc:test/app.bsky.node/789".to_string(),
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

        // Clean up
        cache.clear().await.unwrap();
    }

    #[tokio::test]
    async fn test_redis_cache_blueprint_operations() {
        if !redis_available().await {
            eprintln!("Skipping test: Redis not available. Set TEST_REDIS_URL to enable.");
            return;
        }

        let redis_url = std::env::var("TEST_REDIS_URL").unwrap();
        let pool = create_cache_pool(&redis_url).unwrap();
        let cache = RedisNodeCache::new(
            pool.clone(),
            format!("test:node:blueprint:{}:", uuid::Uuid::new_v4()),
            Duration::from_secs(60),
        );

        let blueprint = "at://did:plc:test/app.bsky.blueprint/test".to_string();
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

        // Clean up
        cache.clear().await.unwrap();
    }

    #[tokio::test]
    async fn test_redis_cache_ttl() {
        if !redis_available().await {
            eprintln!("Skipping test: Redis not available. Set TEST_REDIS_URL to enable.");
            return;
        }

        let redis_url = std::env::var("TEST_REDIS_URL").unwrap();
        let pool = create_cache_pool(&redis_url).unwrap();
        let cache = RedisNodeCache::new(
            pool.clone(),
            format!("test:node:ttl:{}:", uuid::Uuid::new_v4()),
            Duration::from_secs(60),
        );

        let node = Node {
            aturi: "at://did:plc:test/app.bsky.node/ttl".to_string(),
            blueprint: "at://did:plc:test/app.bsky.blueprint/ttl".to_string(),
            node_type: "test_node".to_string(),
            payload: serde_json::json!({"test": "ttl"}),
            configuration: serde_json::json!({}),
            created_at: Utc::now(),
        };

        // Set with 1 second TTL
        cache
            .set(&node.aturi, node.clone(), Duration::from_secs(1))
            .await
            .unwrap();

        // Should exist immediately
        assert!(cache.get(&node.aturi).await.unwrap().is_some());

        // Wait for TTL to expire
        tokio::time::sleep(Duration::from_secs(2)).await;

        // Should be expired
        assert!(cache.get(&node.aturi).await.unwrap().is_none());

        // Clean up
        cache.clear().await.unwrap();
    }
}
