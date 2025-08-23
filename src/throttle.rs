//! Blueprint evaluation throttling.
//!
//! This module provides traits and implementations for throttling blueprint evaluations
//! to prevent abuse and manage resource consumption.

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use deadpool_redis::{Pool as RedisPool, redis::AsyncCommands};
use metrohash::MetroHash64;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

/// Trait for throttling blueprint evaluations.
///
/// Implementations of this trait can be used to control the rate at which
/// blueprints are evaluated, preventing abuse and managing system resources.
#[async_trait]
pub trait BlueprintThrottler: Send + Sync {
    /// Check if a blueprint evaluation should be throttled.
    ///
    /// # Arguments
    ///
    /// * `aturi` - The AT-URI of the blueprint to check
    ///
    /// # Returns
    ///
    /// Returns `Ok(true)` if the evaluation should be throttled (blocked),
    /// `Ok(false)` if the evaluation can proceed, or an error if the check fails.
    async fn throttle(&self, aturi: &str) -> Result<bool>;
}

/// A no-op implementation of BlueprintThrottler that never throttles.
///
/// This implementation always returns `false`, allowing all blueprint
/// evaluations to proceed without throttling.
#[derive(Debug, Clone, Default)]
pub struct NoOpBlueprintThrottler;

impl NoOpBlueprintThrottler {
    /// Create a new NoOpBlueprintThrottler instance.
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl BlueprintThrottler for NoOpBlueprintThrottler {
    async fn throttle(&self, _aturi: &str) -> Result<bool> {
        // Never throttle - always return false
        Ok(false)
    }
}

/// A Redis-backed implementation of BlueprintThrottler that throttles based on
/// AT-URI and authority components using time-bucketed evaluation counts.
#[derive(Debug, Clone)]
pub struct RedisBlueprintThrottler {
    pool: RedisPool,
    authority_limit: Option<i32>,
    authority_window: Option<i32>,
    aturi_limit: Option<i32>,
    aturi_window: Option<i32>,
}

impl RedisBlueprintThrottler {
    /// Create a new RedisBlueprintThrottler instance with configuration.
    ///
    /// # Arguments
    ///
    /// * `pool` - Redis connection pool
    /// * `authority_limit` - Maximum evaluations per authority per window
    /// * `authority_window` - Time window in minutes for authority throttling
    /// * `aturi_limit` - Maximum evaluations per AT-URI per window
    /// * `aturi_window` - Time window in minutes for AT-URI throttling
    ///
    /// # Returns
    ///
    /// Returns a Result containing the throttler or an error if configuration is invalid.
    pub fn new(
        pool: RedisPool,
        authority_limit: Option<i32>,
        authority_window: Option<i32>,
        aturi_limit: Option<i32>,
        aturi_window: Option<i32>,
    ) -> Result<Self> {
        // Validate authority configuration
        match (authority_limit, authority_window) {
            (None, None) => {
                // Both None is valid
            }
            (Some(limit), Some(window)) => {
                if limit <= 0 {
                    return Err(anyhow!(
                        "authority_limit must be greater than 0, got {}",
                        limit
                    ));
                }
                if window <= 0 {
                    return Err(anyhow!(
                        "authority_window must be greater than 0, got {}",
                        window
                    ));
                }
            }
            _ => {
                return Err(anyhow!(
                    "authority_limit and authority_window must both be None or both be Some"
                ));
            }
        }

        // Validate aturi configuration
        match (aturi_limit, aturi_window) {
            (None, None) => {
                // Both None is valid
            }
            (Some(limit), Some(window)) => {
                if limit <= 0 {
                    return Err(anyhow!("aturi_limit must be greater than 0, got {}", limit));
                }
                if window <= 0 {
                    return Err(anyhow!(
                        "aturi_window must be greater than 0, got {}",
                        window
                    ));
                }
            }
            _ => {
                return Err(anyhow!(
                    "aturi_limit and aturi_window must both be None or both be Some"
                ));
            }
        }

        Ok(Self {
            pool,
            authority_limit,
            authority_window,
            aturi_limit,
            aturi_window,
        })
    }

    /// Round up timestamp to the nearest n minutes
    fn round_up_to_minutes(timestamp: u64, minutes: i32) -> u64 {
        let window_seconds = (minutes as u64) * 60;
        ((timestamp + window_seconds - 1) / window_seconds) * window_seconds
    }

    /// Create a Redis key using metrohash64
    fn create_redis_key(timestamp: u64, value: &str) -> String {
        let mut hasher = MetroHash64::new();
        timestamp.hash(&mut hasher);
        value.hash(&mut hasher);
        let hash = hasher.finish();

        format!("throttle:{:016x}", hash)
    }
}

#[async_trait]
impl BlueprintThrottler for RedisBlueprintThrottler {
    async fn throttle(&self, aturi: &str) -> Result<bool> {
        // Return false if both limits are None
        if self.authority_limit.is_none() && self.aturi_limit.is_none() {
            return Ok(false);
        }

        // Get current timestamp
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| anyhow!("Failed to get current time: {}", e))?
            .as_secs();

        // Get a Redis connection
        let mut conn = self
            .pool
            .get()
            .await
            .map_err(|e| anyhow!("Failed to get Redis connection: {}", e))?;

        // Check authority throttling if configured
        if let (Some(limit), Some(window)) = (self.authority_limit, self.authority_window) {
            // Parse AT-URI to get authority
            let parsed = atproto_record::aturi::ATURI::from_str(aturi)
                .map_err(|e| anyhow!("Failed to parse AT-URI: {}", e))?;
            let authority = &parsed.authority;

            // Create timestamp rounded up to nearest window
            let timestamp = Self::round_up_to_minutes(now, window);

            // Create Redis key
            let key = Self::create_redis_key(timestamp, authority);

            // Increment with expiration (window in seconds)
            let count: i32 = conn
                .incr(&key, 1)
                .await
                .map_err(|e| anyhow!("Failed to increment Redis key: {}", e))?;

            // Set expiration if this is the first increment
            if count == 1 {
                let expire_seconds = (window * 60) as u64;
                let _: () = conn
                    .expire(&key, expire_seconds as i64)
                    .await
                    .map_err(|e| anyhow!("Failed to set Redis key expiration: {}", e))?;
            }

            // Check if over limit
            if count > limit {
                return Ok(true);
            }
        }

        // Check AT-URI throttling if configured
        if let (Some(limit), Some(window)) = (self.aturi_limit, self.aturi_window) {
            // Create timestamp rounded up to nearest window
            let timestamp = Self::round_up_to_minutes(now, window);

            // Create Redis key
            let key = Self::create_redis_key(timestamp, aturi);

            // Increment with expiration (window in seconds)
            let count: i32 = conn
                .incr(&key, 1)
                .await
                .map_err(|e| anyhow!("Failed to increment Redis key: {}", e))?;

            // Set expiration if this is the first increment
            if count == 1 {
                let expire_seconds = (window * 60) as u64;
                let _: () = conn
                    .expire(&key, expire_seconds as i64)
                    .await
                    .map_err(|e| anyhow!("Failed to set Redis key expiration: {}", e))?;
            }

            // Check if over limit
            if count > limit {
                return Ok(true);
            }
        }

        // Not throttled
        Ok(false)
    }
}

/// A Redis-backed implementation of BlueprintThrottler that throttles based on
/// per-identity limits stored in Redis, with defaults for identities without custom limits.
#[derive(Debug, Clone)]
pub struct RedisPerIdentityBlueprintThrottler {
    pool: RedisPool,
    authority_default_limit: i32,
    authority_default_window: i32,
    aturi_default_limit: i32,
    aturi_default_window: i32,
}

impl RedisPerIdentityBlueprintThrottler {
    /// Create a new RedisPerIdentityBlueprintThrottler instance with configuration.
    ///
    /// # Arguments
    ///
    /// * `pool` - Redis connection pool
    /// * `authority_default_limit` - Default maximum evaluations per authority per window
    /// * `authority_default_window` - Default time window in minutes for authority throttling
    /// * `aturi_default_limit` - Default maximum evaluations per AT-URI per window
    /// * `aturi_default_window` - Default time window in minutes for AT-URI throttling
    ///
    /// # Returns
    ///
    /// Returns a Result containing the throttler or an error if configuration is invalid.
    pub fn new(
        pool: RedisPool,
        authority_default_limit: i32,
        authority_default_window: i32,
        aturi_default_limit: i32,
        aturi_default_window: i32,
    ) -> Result<Self> {
        // Validate authority defaults
        if authority_default_limit <= 0 {
            return Err(anyhow!(
                "authority_default_limit must be greater than 0, got {}",
                authority_default_limit
            ));
        }
        if authority_default_window <= 0 {
            return Err(anyhow!(
                "authority_default_window must be greater than 0, got {}",
                authority_default_window
            ));
        }

        // Validate aturi defaults
        if aturi_default_limit <= 0 {
            return Err(anyhow!(
                "aturi_default_limit must be greater than 0, got {}",
                aturi_default_limit
            ));
        }
        if aturi_default_window <= 0 {
            return Err(anyhow!(
                "aturi_default_window must be greater than 0, got {}",
                aturi_default_window
            ));
        }

        Ok(Self {
            pool,
            authority_default_limit,
            authority_default_window,
            aturi_default_limit,
            aturi_default_window,
        })
    }

    /// Round up timestamp to the nearest n minutes
    fn round_up_to_minutes(timestamp: u64, minutes: i32) -> u64 {
        let window_seconds = (minutes as u64) * 60;
        ((timestamp + window_seconds - 1) / window_seconds) * window_seconds
    }

    /// Create a Redis key using metrohash64
    fn create_redis_key(timestamp: u64, value: &str) -> String {
        let mut hasher = MetroHash64::new();
        timestamp.hash(&mut hasher);
        value.hash(&mut hasher);
        let hash = hasher.finish();

        format!("throttle:{:016x}", hash)
    }

    /// Get per-identity limits from Redis or use defaults
    async fn get_identity_limits(
        &self,
        conn: &mut deadpool_redis::Connection,
        did: &str,
    ) -> Result<(i32, i32, i32, i32)> {
        // Build the config key for this DID
        let config_key = format!("config:limit:{}", did);

        // Try to get the hash from Redis
        let config: HashMap<String, String> = conn
            .hgetall(&config_key)
            .await
            .unwrap_or_else(|_| HashMap::new());

        // Parse authority limit or use default
        let authority_limit = config
            .get("authority_limit")
            .and_then(|v| v.parse::<i32>().ok())
            .unwrap_or(self.authority_default_limit);

        // Parse authority window or use default
        let authority_window = config
            .get("authority_window")
            .and_then(|v| v.parse::<i32>().ok())
            .unwrap_or(self.authority_default_window);

        // Parse aturi limit or use default
        let aturi_limit = config
            .get("aturi_limit")
            .and_then(|v| v.parse::<i32>().ok())
            .unwrap_or(self.aturi_default_limit);

        // Parse aturi window or use default
        let aturi_window = config
            .get("aturi_window")
            .and_then(|v| v.parse::<i32>().ok())
            .unwrap_or(self.aturi_default_window);

        Ok((authority_limit, authority_window, aturi_limit, aturi_window))
    }
}

#[async_trait]
impl BlueprintThrottler for RedisPerIdentityBlueprintThrottler {
    async fn throttle(&self, aturi: &str) -> Result<bool> {
        // Parse AT-URI to get authority
        let parsed = atproto_record::aturi::ATURI::from_str(aturi)
            .map_err(|e| anyhow!("Failed to parse AT-URI: {}", e))?;
        let authority = &parsed.authority;

        // Get current timestamp
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| anyhow!("Failed to get current time: {}", e))?
            .as_secs();

        // Get a Redis connection
        let mut conn = self
            .pool
            .get()
            .await
            .map_err(|e| anyhow!("Failed to get Redis connection: {}", e))?;

        // Get per-identity limits or defaults
        let (authority_limit, authority_window, aturi_limit, aturi_window) =
            self.get_identity_limits(&mut conn, authority).await?;

        // Check authority throttling
        {
            // Create timestamp rounded up to nearest window
            let timestamp = Self::round_up_to_minutes(now, authority_window);

            // Create Redis key
            let key = Self::create_redis_key(timestamp, authority);

            // Increment with expiration (window in seconds)
            let count: i32 = conn
                .incr(&key, 1)
                .await
                .map_err(|e| anyhow!("Failed to increment Redis key: {}", e))?;

            // Set expiration if this is the first increment
            if count == 1 {
                let expire_seconds = (authority_window * 60) as u64;
                let _: () = conn
                    .expire(&key, expire_seconds as i64)
                    .await
                    .map_err(|e| anyhow!("Failed to set Redis key expiration: {}", e))?;
            }

            // Check if over limit
            if count > authority_limit {
                return Ok(true);
            }
        }

        // Check AT-URI throttling
        {
            // Create timestamp rounded up to nearest window
            let timestamp = Self::round_up_to_minutes(now, aturi_window);

            // Create Redis key
            let key = Self::create_redis_key(timestamp, aturi);

            // Increment with expiration (window in seconds)
            let count: i32 = conn
                .incr(&key, 1)
                .await
                .map_err(|e| anyhow!("Failed to increment Redis key: {}", e))?;

            // Set expiration if this is the first increment
            if count == 1 {
                let expire_seconds = (aturi_window * 60) as u64;
                let _: () = conn
                    .expire(&key, expire_seconds as i64)
                    .await
                    .map_err(|e| anyhow!("Failed to set Redis key expiration: {}", e))?;
            }

            // Check if over limit
            if count > aturi_limit {
                return Ok(true);
            }
        }

        // Not throttled
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_noop_throttler_never_throttles() {
        let throttler = NoOpBlueprintThrottler::new();

        // Test with various AT-URIs
        assert_eq!(
            throttler
                .throttle("at://did:plc:test/blueprint/123")
                .await
                .unwrap(),
            false
        );
        assert_eq!(
            throttler
                .throttle("at://did:plc:user/blueprint/abc")
                .await
                .unwrap(),
            false
        );
        assert_eq!(throttler.throttle("").await.unwrap(), false);
    }

    #[tokio::test]
    async fn test_noop_throttler_is_send_sync() {
        // This test ensures the trait bounds are satisfied
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<NoOpBlueprintThrottler>();
        assert_send_sync::<Box<dyn BlueprintThrottler>>();
    }

    #[test]
    fn test_redis_throttler_validation() {
        use deadpool_redis::Config;

        // Create a dummy pool for testing (won't actually connect)
        let cfg = Config::from_url("redis://localhost");
        let pool = cfg
            .create_pool(Some(deadpool_redis::Runtime::Tokio1))
            .unwrap();

        // Test valid configurations
        assert!(RedisBlueprintThrottler::new(pool.clone(), None, None, None, None).is_ok());

        assert!(RedisBlueprintThrottler::new(pool.clone(), Some(10), Some(5), None, None).is_ok());

        assert!(
            RedisBlueprintThrottler::new(pool.clone(), None, None, Some(100), Some(15)).is_ok()
        );

        assert!(
            RedisBlueprintThrottler::new(pool.clone(), Some(10), Some(5), Some(100), Some(15))
                .is_ok()
        );

        // Test invalid configurations - mismatched authority limit/window
        assert!(RedisBlueprintThrottler::new(pool.clone(), Some(10), None, None, None).is_err());

        assert!(RedisBlueprintThrottler::new(pool.clone(), None, Some(5), None, None).is_err());

        // Test invalid configurations - mismatched aturi limit/window
        assert!(RedisBlueprintThrottler::new(pool.clone(), None, None, Some(100), None).is_err());

        assert!(RedisBlueprintThrottler::new(pool.clone(), None, None, None, Some(15)).is_err());

        // Test invalid configurations - zero or negative values
        assert!(RedisBlueprintThrottler::new(pool.clone(), Some(0), Some(5), None, None).is_err());

        assert!(RedisBlueprintThrottler::new(pool.clone(), Some(-1), Some(5), None, None).is_err());

        assert!(RedisBlueprintThrottler::new(pool.clone(), Some(10), Some(0), None, None).is_err());

        assert!(
            RedisBlueprintThrottler::new(pool.clone(), Some(10), Some(-1), None, None).is_err()
        );

        assert!(RedisBlueprintThrottler::new(pool.clone(), None, None, Some(0), Some(15)).is_err());

        assert!(
            RedisBlueprintThrottler::new(pool.clone(), None, None, Some(100), Some(0)).is_err()
        );
    }

    #[test]
    fn test_round_up_to_minutes() {
        // Test rounding up to 5 minutes
        assert_eq!(RedisBlueprintThrottler::round_up_to_minutes(0, 5), 0);
        assert_eq!(RedisBlueprintThrottler::round_up_to_minutes(1, 5), 300);
        assert_eq!(RedisBlueprintThrottler::round_up_to_minutes(299, 5), 300);
        assert_eq!(RedisBlueprintThrottler::round_up_to_minutes(300, 5), 300);
        assert_eq!(RedisBlueprintThrottler::round_up_to_minutes(301, 5), 600);

        // Test rounding up to 15 minutes
        assert_eq!(RedisBlueprintThrottler::round_up_to_minutes(0, 15), 0);
        assert_eq!(RedisBlueprintThrottler::round_up_to_minutes(1, 15), 900);
        assert_eq!(RedisBlueprintThrottler::round_up_to_minutes(899, 15), 900);
        assert_eq!(RedisBlueprintThrottler::round_up_to_minutes(900, 15), 900);
        assert_eq!(RedisBlueprintThrottler::round_up_to_minutes(901, 15), 1800);
    }

    #[test]
    fn test_create_redis_key() {
        // Test that keys are consistent for same inputs
        let key1 = RedisBlueprintThrottler::create_redis_key(1234567890, "test-value");
        let key2 = RedisBlueprintThrottler::create_redis_key(1234567890, "test-value");
        assert_eq!(key1, key2);

        // Test that keys are different for different timestamps
        let key3 = RedisBlueprintThrottler::create_redis_key(1234567891, "test-value");
        assert_ne!(key1, key3);

        // Test that keys are different for different values
        let key4 = RedisBlueprintThrottler::create_redis_key(1234567890, "other-value");
        assert_ne!(key1, key4);

        // Test that keys have the correct prefix
        assert!(key1.starts_with("throttle:"));
    }

    #[test]
    fn test_redis_throttler_is_send_sync() {
        // This test ensures the trait bounds are satisfied
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<RedisBlueprintThrottler>();
    }

    #[test]
    fn test_redis_per_identity_throttler_validation() {
        use deadpool_redis::Config;

        // Create a dummy pool for testing (won't actually connect)
        let cfg = Config::from_url("redis://localhost");
        let pool = cfg
            .create_pool(Some(deadpool_redis::Runtime::Tokio1))
            .unwrap();

        // Test valid configurations
        assert!(RedisPerIdentityBlueprintThrottler::new(pool.clone(), 10, 5, 100, 15).is_ok());

        // Test invalid configurations - zero or negative limits
        assert!(RedisPerIdentityBlueprintThrottler::new(pool.clone(), 0, 5, 100, 15).is_err());
        assert!(RedisPerIdentityBlueprintThrottler::new(pool.clone(), -1, 5, 100, 15).is_err());
        assert!(RedisPerIdentityBlueprintThrottler::new(pool.clone(), 10, 0, 100, 15).is_err());
        assert!(RedisPerIdentityBlueprintThrottler::new(pool.clone(), 10, -1, 100, 15).is_err());
        assert!(RedisPerIdentityBlueprintThrottler::new(pool.clone(), 10, 5, 0, 15).is_err());
        assert!(RedisPerIdentityBlueprintThrottler::new(pool.clone(), 10, 5, -1, 15).is_err());
        assert!(RedisPerIdentityBlueprintThrottler::new(pool.clone(), 10, 5, 100, 0).is_err());
        assert!(RedisPerIdentityBlueprintThrottler::new(pool.clone(), 10, 5, 100, -1).is_err());
    }

    #[test]
    fn test_redis_per_identity_throttler_is_send_sync() {
        // This test ensures the trait bounds are satisfied
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<RedisPerIdentityBlueprintThrottler>();
    }
}
