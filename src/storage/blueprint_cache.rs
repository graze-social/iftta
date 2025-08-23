use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::time::Duration;

use super::Blueprint;

/// Trait for caching blueprint data with TTL support
#[async_trait]
pub trait BlueprintCache: Send + Sync {
    /// Get a blueprint from cache by its ATURI
    async fn get(&self, aturi: &str) -> Result<Option<Blueprint>>;

    /// Set a blueprint in cache with a TTL
    async fn set(&self, aturi: &str, blueprint: Blueprint, ttl: Duration) -> Result<()>;

    /// Invalidate (delete) a blueprint from cache
    async fn invalidate(&self, aturi: &str) -> Result<()>;

    /// Clear all cached blueprints
    async fn clear(&self) -> Result<()>;

    /// Get cache statistics (optional implementation)
    async fn stats(&self) -> CacheStats {
        CacheStats::default()
    }
}

/// Cache statistics
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct CacheStats {
    pub hits: u64,
    pub misses: u64,
    pub entries: u64,
    pub evictions: u64,
}

impl CacheStats {
    pub fn hit_rate(&self) -> f64 {
        let total = self.hits + self.misses;
        if total == 0 {
            0.0
        } else {
            self.hits as f64 / total as f64
        }
    }
}

/// No-op cache implementation that always returns None
pub struct NoOpCache;

#[async_trait]
impl BlueprintCache for NoOpCache {
    async fn get(&self, _aturi: &str) -> Result<Option<Blueprint>> {
        Ok(None)
    }

    async fn set(&self, _aturi: &str, _blueprint: Blueprint, _ttl: Duration) -> Result<()> {
        Ok(())
    }

    async fn invalidate(&self, _aturi: &str) -> Result<()> {
        Ok(())
    }

    async fn clear(&self) -> Result<()> {
        Ok(())
    }
}
