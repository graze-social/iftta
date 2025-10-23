//! Redis cache pool management using deadpool-redis

use crate::errors::StorageError;
use anyhow::Result;
use deadpool_redis::{Config, Pool, Runtime};

/// Create a Redis connection pool from a Redis URL
///
/// # Arguments
/// * `redis_url` - Redis connection URL (e.g., "redis://localhost:6379")
///
/// # Returns
/// A deadpool-redis Pool configured for async operation
pub fn create_cache_pool(redis_url: &str) -> Result<Pool> {
    let cfg = Config::from_url(redis_url);
    cfg.create_pool(Some(Runtime::Tokio1)).map_err(|err| {
        StorageError::ConnectionFailed {
            source: sqlx::Error::Configuration(
                format!("Failed to create Redis pool: {}", err).into(),
            ),
        }
        .into()
    })
}

/// Redis keys for various cache operations
pub mod keys {
    /// Prefix for Jetstream cursor storage
    pub const JETSTREAM_CURSOR_PREFIX: &str = "jetstream:cursor";

    /// Prefix for leadership election
    pub const LEADERSHIP_PREFIX: &str = "leadership";

    /// Build a Jetstream cursor key
    pub fn jetstream_cursor_key(suffix: &str) -> String {
        format!("{}:{}", JETSTREAM_CURSOR_PREFIX, suffix)
    }

    /// Build a leadership key
    pub fn leadership_key(group: &str) -> String {
        format!("{}:{}", LEADERSHIP_PREFIX, group)
    }
}
