use anyhow::Result;
use async_trait::async_trait;
use moka::future::Cache;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use super::{Blueprint, BlueprintCache, CacheStats};

/// In-memory blueprint cache implementation using moka
pub struct InMemoryBlueprintCache {
    cache: Cache<String, Blueprint>,
    hits: Arc<AtomicU64>,
    misses: Arc<AtomicU64>,
}

impl InMemoryBlueprintCache {
    /// Create a new in-memory cache with specified capacity and default TTL
    pub fn new(max_capacity: u64, default_ttl: Duration) -> Self {
        let cache = Cache::builder()
            .max_capacity(max_capacity)
            .time_to_live(default_ttl)
            .build();

        Self {
            cache,
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
impl BlueprintCache for InMemoryBlueprintCache {
    async fn get(&self, aturi: &str) -> Result<Option<Blueprint>> {
        let result = self.cache.get(aturi).await;

        if result.is_some() {
            self.hits.fetch_add(1, Ordering::Relaxed);
        } else {
            self.misses.fetch_add(1, Ordering::Relaxed);
        }

        Ok(result)
    }

    async fn set(&self, aturi: &str, blueprint: Blueprint, _ttl: Duration) -> Result<()> {
        // Insert into cache - moka will handle TTL based on cache configuration
        self.cache.insert(aturi.to_string(), blueprint).await;

        Ok(())
    }

    async fn invalidate(&self, aturi: &str) -> Result<()> {
        self.cache.remove(aturi).await;
        Ok(())
    }

    async fn clear(&self) -> Result<()> {
        self.cache.invalidate_all();
        self.cache.run_pending_tasks().await;
        Ok(())
    }

    async fn stats(&self) -> CacheStats {
        CacheStats {
            hits: self.hits.load(Ordering::Relaxed),
            misses: self.misses.load(Ordering::Relaxed),
            entries: self.cache.entry_count(),
            evictions: 0, // moka doesn't expose eviction count directly
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::Blueprint;

    #[tokio::test]
    async fn test_in_memory_cache_basic_operations() {
        let cache = InMemoryBlueprintCache::new(10, Duration::from_secs(60));

        let blueprint = Blueprint {
            aturi: "at://did:plc:test/app.bsky.blueprint/123".to_string(),
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

        // Check stats
        let stats = cache.stats().await;
        assert_eq!(stats.hits, 1);
        assert_eq!(stats.misses, 2);
    }

    #[tokio::test]
    async fn test_in_memory_cache_ttl() {
        // Create cache with very short TTL for testing
        let cache = InMemoryBlueprintCache::new(10, Duration::from_millis(100));

        let blueprint = Blueprint {
            aturi: "at://did:plc:test/app.bsky.blueprint/456".to_string(),
            did: "did:plc:test".to_string(),
            node_order: vec!["node1".to_string()],
            enabled: true,
            created_at: chrono::Utc::now(),
            error: None,
        };

        // Set blueprint (will use cache's configured TTL)
        cache
            .set(
                &blueprint.aturi,
                blueprint.clone(),
                Duration::from_millis(100),
            )
            .await
            .unwrap();

        // Should exist immediately
        assert!(cache.get(&blueprint.aturi).await.unwrap().is_some());

        // Wait for TTL to expire
        tokio::time::sleep(Duration::from_millis(200)).await;

        // Force cache to run maintenance to evict expired entries
        cache.cache.run_pending_tasks().await;

        // Should be expired
        assert!(cache.get(&blueprint.aturi).await.unwrap().is_none());
    }
}
