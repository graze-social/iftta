use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::time::Duration;

use super::Node;

/// Trait for caching node data with TTL support
#[async_trait]
pub trait NodeCache: Send + Sync {
    /// Get a node from cache by its ATURI
    async fn get(&self, aturi: &str) -> Result<Option<Node>>;

    /// Get multiple nodes from cache by blueprint
    async fn get_by_blueprint(&self, blueprint: &str) -> Result<Option<Vec<Node>>>;

    /// Set a node in cache with a TTL
    async fn set(&self, aturi: &str, node: Node, ttl: Duration) -> Result<()>;

    /// Set multiple nodes for a blueprint in cache with a TTL
    async fn set_by_blueprint(
        &self,
        blueprint: &str,
        nodes: Vec<Node>,
        ttl: Duration,
    ) -> Result<()>;

    /// Invalidate (delete) a node from cache
    async fn invalidate(&self, aturi: &str) -> Result<()>;

    /// Invalidate all nodes for a blueprint
    async fn invalidate_by_blueprint(&self, blueprint: &str) -> Result<()>;

    /// Clear all cached nodes
    async fn clear(&self) -> Result<()>;

    /// Get cache statistics (optional implementation)
    async fn stats(&self) -> NodeCacheStats {
        NodeCacheStats::default()
    }
}

/// Cache statistics for nodes
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct NodeCacheStats {
    pub hits: u64,
    pub misses: u64,
    pub entries: u64,
    pub evictions: u64,
}

impl NodeCacheStats {
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
pub struct NoOpNodeCache;

#[async_trait]
impl NodeCache for NoOpNodeCache {
    async fn get(&self, _aturi: &str) -> Result<Option<Node>> {
        Ok(None)
    }

    async fn get_by_blueprint(&self, _blueprint: &str) -> Result<Option<Vec<Node>>> {
        Ok(None)
    }

    async fn set(&self, _aturi: &str, _node: Node, _ttl: Duration) -> Result<()> {
        Ok(())
    }

    async fn set_by_blueprint(
        &self,
        _blueprint: &str,
        _nodes: Vec<Node>,
        _ttl: Duration,
    ) -> Result<()> {
        Ok(())
    }

    async fn invalidate(&self, _aturi: &str) -> Result<()> {
        Ok(())
    }

    async fn invalidate_by_blueprint(&self, _blueprint: &str) -> Result<()> {
        Ok(())
    }

    async fn clear(&self) -> Result<()> {
        Ok(())
    }
}
