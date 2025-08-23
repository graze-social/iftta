use anyhow::Result;
use async_trait::async_trait;
use deadpool_redis::{Pool, redis::AsyncCommands};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tracing::{debug, warn};

use super::{Blueprint, BlueprintCache, CacheStats};

/// Redis-based blueprint cache implementation using deadpool
pub struct RedisBlueprintCache {
    pool: Pool,
    key_prefix: String,
    hits: Arc<AtomicU64>,
    misses: Arc<AtomicU64>,
    default_ttl: Duration,
}

impl RedisBlueprintCache {
    /// Create a new Redis cache with the given pool and configuration
    pub fn new(pool: Pool, key_prefix: String, default_ttl: Duration) -> Self {
        Self {
            pool,
            key_prefix,
            hits: Arc::new(AtomicU64::new(0)),
            misses: Arc::new(AtomicU64::new(0)),
            default_ttl,
        }
    }

    /// Create with default settings (60 second TTL)
    pub fn with_defaults(pool: Pool) -> Self {
        Self::new(
            pool,
            "blueprint:cache:".to_string(),
            Duration::from_secs(60),
        )
    }

    /// Generate the full Redis key for an ATURI
    fn make_key(&self, aturi: &str) -> String {
        format!("{}{}", self.key_prefix, aturi)
    }
}

#[async_trait]
impl BlueprintCache for RedisBlueprintCache {
    async fn get(&self, aturi: &str) -> Result<Option<Blueprint>> {
        let mut conn = self.pool.get().await?;
        let key = self.make_key(aturi);

        let data: Option<Vec<u8>> = conn.get(&key).await?;

        let result = match data {
            Some(bytes) => {
                match serde_json::from_slice::<Blueprint>(&bytes) {
                    Ok(blueprint) => {
                        self.hits.fetch_add(1, Ordering::Relaxed);
                        debug!(aturi = %aturi, "Cache hit");
                        Some(blueprint)
                    }
                    Err(e) => {
                        warn!(aturi = %aturi, error = ?e, "Failed to deserialize cached blueprint");
                        self.misses.fetch_add(1, Ordering::Relaxed);
                        // Remove corrupted entry
                        let _: Result<(), _> = conn.del(&key).await;
                        None
                    }
                }
            }
            None => {
                self.misses.fetch_add(1, Ordering::Relaxed);
                debug!(aturi = %aturi, "Cache miss");
                None
            }
        };

        Ok(result)
    }

    async fn set(&self, aturi: &str, blueprint: Blueprint, ttl: Duration) -> Result<()> {
        let mut conn = self.pool.get().await?;
        let key = self.make_key(aturi);

        let data = serde_json::to_vec(&blueprint)?;
        let effective_ttl = if ttl.as_secs() > 0 {
            ttl
        } else {
            self.default_ttl
        };

        // SET with expiration in seconds
        let _: () = conn.set_ex(&key, data, effective_ttl.as_secs()).await?;

        debug!(aturi = %aturi, ttl_secs = effective_ttl.as_secs(), "Cached blueprint");
        Ok(())
    }

    async fn invalidate(&self, aturi: &str) -> Result<()> {
        let mut conn = self.pool.get().await?;
        let key = self.make_key(aturi);

        let deleted: u64 = conn.del(&key).await?;
        if deleted > 0 {
            debug!(aturi = %aturi, "Invalidated cached blueprint");
        }

        Ok(())
    }

    async fn clear(&self) -> Result<()> {
        let mut conn = self.pool.get().await?;

        // Use SCAN to find all keys with our prefix
        let pattern = format!("{}*", self.key_prefix);

        // Redis SCAN returns an iterator, we need to collect the keys
        let keys: Vec<String> = {
            let mut iter = conn.scan_match::<_, String>(&pattern).await?;
            let mut keys = Vec::new();

            while let Some(key) = iter.next_item().await {
                keys.push(key);
            }
            keys
        };

        if !keys.is_empty() {
            let deleted: u64 = conn.del(keys.as_slice()).await?;
            debug!(count = deleted, "Cleared cache entries");
        }

        Ok(())
    }

    async fn stats(&self) -> CacheStats {
        let mut entries = 0;

        // Try to count keys with our prefix
        if let Ok(mut conn) = self.pool.get().await {
            let pattern = format!("{}*", self.key_prefix);
            if let Ok(mut iter) = conn.scan_match::<_, String>(&pattern).await {
                let mut count = 0u64;
                while iter.next_item().await.is_some() {
                    count += 1;
                }
                entries = count;
            }
        }

        CacheStats {
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
    use crate::storage::Blueprint;
    use crate::storage::cache::create_cache_pool;

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
        let cache = RedisBlueprintCache::new(
            pool.clone(),
            format!("test:blueprint:cache:{}:", uuid::Uuid::new_v4()),
            Duration::from_secs(60),
        );

        let blueprint = Blueprint {
            aturi: "at://did:plc:test/app.bsky.blueprint/789".to_string(),
            did: "did:plc:test".to_string(),
            node_order: vec!["node1".to_string(), "node2".to_string()],
            enabled: true,
            created_at: chrono::Utc::now(),
            error: None,
        };

        // Cache miss
        assert!(cache.get(&blueprint.aturi).await.unwrap().is_none());

        // Set value
        cache
            .set(&blueprint.aturi, blueprint.clone(), Duration::from_secs(60))
            .await
            .unwrap();

        // Cache hit
        let cached = cache.get(&blueprint.aturi).await.unwrap();
        assert!(cached.is_some());
        assert_eq!(cached.unwrap().did, "did:plc:test");

        // Invalidate
        cache.invalidate(&blueprint.aturi).await.unwrap();
        assert!(cache.get(&blueprint.aturi).await.unwrap().is_none());

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
        let cache = RedisBlueprintCache::new(
            pool.clone(),
            format!("test:blueprint:ttl:{}:", uuid::Uuid::new_v4()),
            Duration::from_secs(60),
        );

        let blueprint = Blueprint {
            aturi: "at://did:plc:test/app.bsky.blueprint/ttl".to_string(),
            did: "did:plc:test".to_string(),
            node_order: vec!["node1".to_string()],
            enabled: true,
            created_at: chrono::Utc::now(),
            error: None,
        };

        // Set with 1 second TTL
        cache
            .set(&blueprint.aturi, blueprint.clone(), Duration::from_secs(1))
            .await
            .unwrap();

        // Should exist immediately
        assert!(cache.get(&blueprint.aturi).await.unwrap().is_some());

        // Wait for TTL to expire
        tokio::time::sleep(Duration::from_secs(2)).await;

        // Should be expired
        assert!(cache.get(&blueprint.aturi).await.unwrap().is_none());

        // Clean up
        cache.clear().await.unwrap();
    }
}
