//! Leadership election system for distributed task coordination.
//!
//! This module provides a trait-based leadership election system that ensures
//! only one instance processes specific tasks at a time. Different tasks can have
//! independent leadership elections using different election keys.
//!
//! # Design
//!
//! - **Trait-based**: `LeadershipElection` trait allows multiple implementations
//! - **Redis-backed**: Initial implementation uses Redis for coordination
//! - **Configurable**: Leadership key, retry intervals, and TTL are configurable
//! - **Heartbeat**: Leaders must maintain their status through periodic renewal
//! - **Graceful**: Supports graceful leadership transitions and shutdowns
//!
//! # Usage
//!
//! ```rust,no_run
//! use ifthisthenat::leadership::{LeadershipElection, RedisLeadershipElection, LeadershipConfig, LeadershipScope};
//! use deadpool_redis::redis::Client;
//!
//! # async fn example() -> anyhow::Result<()> {
//! let redis_client = Client::open("redis://127.0.0.1:6379")?;
//! let config = LeadershipConfig::for_scope(LeadershipScope::BlueprintEvaluation);
//! let election = RedisLeadershipElection::new(redis_client, config);
//!
//! // Try to become leader
//! if election.try_acquire_leadership().await? {
//!     println!("I am now the leader!");
//!     
//!     // Maintain leadership with periodic heartbeats
//!     let heartbeat_interval = std::time::Duration::from_secs(30);
//!     tokio::spawn(async move {
//!         let mut interval = tokio::time::interval(heartbeat_interval);
//!         loop {
//!             interval.tick().await;
//!             if let Err(e) = election.maintain_leadership().await {
//!                 eprintln!("Failed to maintain leadership: {}", e);
//!                 break;
//!             }
//!         }
//!     });
//! }
//! # Ok(())
//! # }
//! ```

use anyhow::Result;
use async_trait::async_trait;
use deadpool_redis::redis::{AsyncCommands, Client as RedisClient};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::errors::LeadershipError;

/// Task types that can use leadership election
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LeadershipScope {
    /// Blueprint evaluation from Jetstream events
    BlueprintEvaluation,
    /// Webhook processing
    WebhookProcessor,
    /// Periodic task scheduler
    Scheduler,
    /// Custom scope for other tasks
    Custom(String),
}

impl LeadershipScope {
    /// Get the default election key for this scope
    pub fn default_election_key(&self) -> String {
        match self {
            Self::BlueprintEvaluation => "ifthisthenat:leadership:blueprint_evaluation".to_string(),
            Self::WebhookProcessor => "ifthisthenat:leadership:webhook_processor".to_string(),
            Self::Scheduler => "ifthisthenat:leadership:scheduler".to_string(),
            Self::Custom(name) => format!("ifthisthenat:leadership:{}", name),
        }
    }

    /// Get environment variable prefix for this scope
    pub fn env_prefix(&self) -> &'static str {
        match self {
            Self::BlueprintEvaluation => "BLUEPRINT_LEADERSHIP",
            Self::WebhookProcessor => "WEBHOOK_LEADERSHIP",
            Self::Scheduler => "SCHEDULER_LEADERSHIP",
            Self::Custom(_) => "LEADERSHIP",
        }
    }
}

/// Configuration for leadership election
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeadershipConfig {
    /// The scope/purpose of this leadership election
    pub scope: LeadershipScope,

    /// Redis key used for leadership election coordination
    pub election_key: String,

    /// How often to retry acquiring leadership when not leader (in seconds)
    pub retry_interval_secs: u64,

    /// How long leadership lasts without renewal (in seconds)
    /// Leaders must call maintain_leadership() within this time
    pub leadership_ttl_secs: u64,

    /// Unique identifier for this instance
    pub instance_id: String,

    /// Whether leadership election is enabled for this scope
    pub enabled: bool,
}

impl Default for LeadershipConfig {
    fn default() -> Self {
        Self::for_scope(LeadershipScope::BlueprintEvaluation)
    }
}

impl LeadershipConfig {
    /// Create a config for a specific scope with default values
    pub fn for_scope(scope: LeadershipScope) -> Self {
        let election_key = scope.default_election_key();
        Self {
            scope,
            election_key,
            retry_interval_secs: 30,
            leadership_ttl_secs: 120, // 2 minutes
            instance_id: Uuid::new_v4().to_string(),
            enabled: true,
        }
    }

    /// Create leadership config from environment variables for a specific scope
    pub fn from_env_for_scope(scope: &LeadershipScope) -> Self {
        let prefix = scope.env_prefix();

        // Check if leadership is enabled for this scope
        let enabled = std::env::var(format!("{}_ENABLED", prefix))
            .map(|v| v.to_lowercase() == "true")
            .unwrap_or(false);

        // Use scope-specific environment variables with fallback to generic ones
        let election_key = std::env::var(format!("{}_ELECTION_KEY", prefix))
            .or_else(|_| std::env::var("LEADERSHIP_ELECTION_KEY"))
            .unwrap_or_else(|_| scope.default_election_key());

        let retry_interval_secs = std::env::var(format!("{}_RETRY_INTERVAL_SECS", prefix))
            .or_else(|_| std::env::var("LEADERSHIP_RETRY_INTERVAL_SECS"))
            .unwrap_or_else(|_| "30".to_string())
            .parse::<u64>()
            .unwrap_or(30)
            .max(5); // Minimum 5 seconds

        let leadership_ttl_secs = std::env::var(format!("{}_TTL_SECS", prefix))
            .or_else(|_| std::env::var("LEADERSHIP_TTL_SECS"))
            .unwrap_or_else(|_| "120".to_string())
            .parse::<u64>()
            .unwrap_or(120)
            .max(30); // Minimum 30 seconds

        let instance_id = std::env::var(format!("{}_INSTANCE_ID", prefix))
            .or_else(|_| std::env::var("LEADERSHIP_INSTANCE_ID"))
            .unwrap_or_else(|_| Uuid::new_v4().to_string());

        Self {
            scope: scope.clone(),
            election_key,
            retry_interval_secs,
            leadership_ttl_secs,
            instance_id,
            enabled,
        }
    }

    /// Create leadership config from environment variables (backward compatible)
    pub fn from_env() -> Self {
        Self::from_env_for_scope(&LeadershipScope::BlueprintEvaluation)
    }

    /// Validate the configuration
    pub fn validate(&self) -> anyhow::Result<()> {
        // Ensure TTL is longer than retry interval
        if self.leadership_ttl_secs <= self.retry_interval_secs {
            return Err(LeadershipError::InvalidConfiguration {
                details: format!(
                    "Leadership TTL ({} seconds) must be greater than retry interval ({} seconds)",
                    self.leadership_ttl_secs,
                    self.retry_interval_secs
                ),
            }.into());
        }

        // Ensure TTL is at least 3x retry interval for safe heartbeat operation
        if self.leadership_ttl_secs < self.retry_interval_secs * 3 {
            tracing::warn!(
                "Leadership TTL ({} seconds) should be at least 3x retry interval ({} seconds) for reliable operation",
                self.leadership_ttl_secs,
                self.retry_interval_secs
            );
        }

        // Validate election key is not empty
        if self.election_key.trim().is_empty() {
            return Err(LeadershipError::InvalidConfiguration {
                details: "Leadership election key cannot be empty".to_string(),
            }.into());
        }

        // Validate instance ID is not empty
        if self.instance_id.trim().is_empty() {
            return Err(LeadershipError::InvalidConfiguration {
                details: "Leadership instance ID cannot be empty".to_string(),
            }.into());
        }

        tracing::info!(
            scope = ?self.scope,
            election_key = %self.election_key,
            retry_interval_secs = self.retry_interval_secs,
            ttl_secs = self.leadership_ttl_secs,
            instance_id = %self.instance_id,
            enabled = self.enabled,
            "Leadership election configured"
        );

        Ok(())
    }
}

/// Result of a leadership operation
#[derive(Debug, Clone, PartialEq)]
pub enum LeadershipStatus {
    /// Successfully acquired or maintained leadership
    Leader,
    /// Not the leader, another instance holds leadership
    Follower,
    /// Leadership is vacant, but failed to acquire it
    Vacant,
}

/// Trait for leadership election implementations
#[async_trait]
pub trait LeadershipElection: Send + Sync {
    /// Attempt to acquire leadership
    /// Returns true if leadership was acquired, false otherwise
    async fn try_acquire_leadership(&self) -> Result<bool>;

    /// Check if this instance is currently the leader
    /// Returns true if leader, false otherwise
    async fn is_leader(&self) -> Result<bool>;

    /// Maintain/renew leadership (heartbeat mechanism)
    /// Should be called periodically by the leader
    async fn maintain_leadership(&self) -> Result<LeadershipStatus>;

    /// Gracefully release leadership
    /// Should be called during shutdown
    async fn release_leadership(&self) -> Result<()>;

    /// Get the current leader's instance ID
    async fn get_current_leader(&self) -> Result<Option<String>>;

    /// Get leadership status with detailed information
    async fn get_status(&self) -> Result<LeadershipStatus>;
}

/// Redis-backed leadership election implementation
pub struct RedisLeadershipElection {
    redis_client: RedisClient,
    config: LeadershipConfig,
}

impl RedisLeadershipElection {
    /// Create a new Redis-backed leadership election
    pub fn new(redis_client: RedisClient, config: LeadershipConfig) -> Self {
        Self {
            redis_client,
            config,
        }
    }

    /// Get Redis connection
    async fn get_connection(&self) -> Result<deadpool_redis::redis::aio::MultiplexedConnection> {
        Ok(self.redis_client.get_multiplexed_async_connection().await?)
    }
}

#[async_trait]
impl LeadershipElection for RedisLeadershipElection {
    async fn try_acquire_leadership(&self) -> Result<bool> {
        let mut conn = self.get_connection().await?;

        // Use SET with NX (only set if not exists) and EX (expiration)
        // This atomically sets the key only if it doesn't exist, with a TTL
        let result: Option<String> = conn
            .set_options(
                &self.config.election_key,
                &self.config.instance_id,
                deadpool_redis::redis::SetOptions::default()
                    .conditional_set(deadpool_redis::redis::ExistenceCheck::NX)
                    .get(true)
                    .with_expiration(deadpool_redis::redis::SetExpiry::EX(
                        self.config.leadership_ttl_secs,
                    )),
            )
            .await?;

        let acquired = result.is_none(); // NX returns None if key was set successfully

        if acquired {
            info!(
                instance_id = %self.config.instance_id,
                election_key = %self.config.election_key,
                ttl_secs = self.config.leadership_ttl_secs,
                "Acquired leadership"
            );
        } else {
            debug!(
                instance_id = %self.config.instance_id,
                election_key = %self.config.election_key,
                "Failed to acquire leadership - another instance is leader"
            );
        }

        Ok(acquired)
    }

    async fn is_leader(&self) -> Result<bool> {
        let mut conn = self.get_connection().await?;

        let current_leader: Option<String> = conn.get(&self.config.election_key).await?;

        Ok(current_leader.as_ref() == Some(&self.config.instance_id))
    }

    async fn maintain_leadership(&self) -> Result<LeadershipStatus> {
        let mut conn = self.get_connection().await?;

        // First check if we're still the leader
        let current_leader: Option<String> = conn.get(&self.config.election_key).await?;

        match current_leader {
            Some(leader_id) if leader_id == self.config.instance_id => {
                // We are the leader, renew the TTL
                let renewed: bool = conn
                    .expire(
                        &self.config.election_key,
                        self.config.leadership_ttl_secs as i64,
                    )
                    .await?;

                if renewed {
                    debug!(
                        instance_id = %self.config.instance_id,
                        ttl_secs = self.config.leadership_ttl_secs,
                        "Leadership renewed"
                    );
                    Ok(LeadershipStatus::Leader)
                } else {
                    warn!(
                        instance_id = %self.config.instance_id,
                        "Failed to renew leadership - key may have expired"
                    );
                    Ok(LeadershipStatus::Vacant)
                }
            }
            Some(_other_leader) => {
                debug!(
                    instance_id = %self.config.instance_id,
                    "Not the leader - cannot maintain leadership"
                );
                Ok(LeadershipStatus::Follower)
            }
            None => {
                debug!(
                    instance_id = %self.config.instance_id,
                    "No current leader - leadership is vacant"
                );
                Ok(LeadershipStatus::Vacant)
            }
        }
    }

    async fn release_leadership(&self) -> Result<()> {
        let mut conn = self.get_connection().await?;

        // Only delete the key if we are the current leader (atomic check-and-delete)
        let lua_script = r#"
            local current = redis.call('GET', KEYS[1])
            if current == ARGV[1] then
                return redis.call('DEL', KEYS[1])
            else
                return 0
            end
        "#;

        let result: i32 = deadpool_redis::redis::Script::new(lua_script)
            .key(&self.config.election_key)
            .arg(&self.config.instance_id)
            .invoke_async(&mut conn)
            .await?;

        if result == 1 {
            info!(
                instance_id = %self.config.instance_id,
                election_key = %self.config.election_key,
                "Released leadership gracefully"
            );
        } else {
            debug!(
                instance_id = %self.config.instance_id,
                "Not the leader - nothing to release"
            );
        }

        Ok(())
    }

    async fn get_current_leader(&self) -> Result<Option<String>> {
        let mut conn = self.get_connection().await?;
        let leader: Option<String> = conn.get(&self.config.election_key).await?;
        Ok(leader)
    }

    async fn get_status(&self) -> Result<LeadershipStatus> {
        let current_leader = self.get_current_leader().await?;

        match current_leader {
            Some(leader_id) if leader_id == self.config.instance_id => Ok(LeadershipStatus::Leader),
            Some(_) => Ok(LeadershipStatus::Follower),
            None => Ok(LeadershipStatus::Vacant),
        }
    }
}

/// Factory for creating task-specific leadership elections
pub struct LeadershipFactory;

impl LeadershipFactory {
    /// Create a leadership election for a specific scope if Redis is available
    pub async fn create_for_scope(
        scope: LeadershipScope,
        redis_client: Option<RedisClient>,
    ) -> Option<Arc<dyn LeadershipElection>> {
        let config = LeadershipConfig::from_env_for_scope(&scope);

        // Check if leadership is enabled for this scope
        if !config.enabled {
            tracing::info!(
                scope = ?scope,
                "Leadership election disabled for scope"
            );
            return None;
        }

        // Require Redis for leadership election
        let redis_client = redis_client?;

        // Validate configuration
        if let Err(e) = config.validate() {
            tracing::error!(
                scope = ?scope,
                error = ?e,
                "Invalid leadership configuration for scope"
            );
            return None;
        }

        Some(Arc::new(RedisLeadershipElection::new(redis_client, config)))
    }

    /// Create a leadership election with explicit configuration
    pub fn create_with_config(
        redis_client: RedisClient,
        config: LeadershipConfig,
    ) -> Arc<dyn LeadershipElection> {
        Arc::new(RedisLeadershipElection::new(redis_client, config))
    }
}

/// Leadership election manager that handles the full lifecycle
pub struct LeadershipManager {
    election: Arc<dyn LeadershipElection>,
    config: LeadershipConfig,
}

impl LeadershipManager {
    /// Create a new leadership manager
    pub fn new(election: Arc<dyn LeadershipElection>, config: LeadershipConfig) -> Self {
        Self { election, config }
    }

    /// Start the leadership election process
    /// Returns a future that runs the leadership election loop
    pub async fn start_election_loop(self) -> Result<()> {
        let mut retry_interval =
            tokio::time::interval(Duration::from_secs(self.config.retry_interval_secs));
        let heartbeat_interval = Duration::from_secs(self.config.leadership_ttl_secs / 3); // Heartbeat 3x more frequently than TTL
        let mut heartbeat_interval_ticker = tokio::time::interval(heartbeat_interval);

        let mut is_leader = false;

        loop {
            tokio::select! {
                _ = retry_interval.tick() => {
                    // Try to acquire leadership if not already leader
                    if !is_leader {
                        match self.election.try_acquire_leadership().await {
                            Ok(acquired) => {
                                if acquired {
                                    is_leader = true;
                                    info!(
                                        instance_id = %self.config.instance_id,
                                        "Became leader - blueprint evaluation enabled"
                                    );
                                }
                            }
                            Err(e) => {
                                warn!(
                                    instance_id = %self.config.instance_id,
                                    error = ?e,
                                    "Failed to attempt leadership acquisition"
                                );
                            }
                        }
                    }
                }

                _ = heartbeat_interval_ticker.tick() => {
                    // Maintain leadership if we are the leader
                    if is_leader {
                        match self.election.maintain_leadership().await {
                            Ok(LeadershipStatus::Leader) => {
                                // Still leader, continue
                            }
                            Ok(LeadershipStatus::Follower) => {
                                info!(
                                    instance_id = %self.config.instance_id,
                                    "Lost leadership to another instance - blueprint evaluation disabled"
                                );
                                is_leader = false;
                            }
                            Ok(LeadershipStatus::Vacant) => {
                                warn!(
                                    instance_id = %self.config.instance_id,
                                    "Leadership became vacant - will try to reacquire"
                                );
                                is_leader = false;
                            }
                            Err(e) => {
                                warn!(
                                    instance_id = %self.config.instance_id,
                                    error = ?e,
                                    "Failed to maintain leadership - assuming lost"
                                );
                                is_leader = false;
                            }
                        }
                    }
                }
            }
        }
    }

    /// Check if this instance is currently the leader
    pub async fn is_leader(&self) -> bool {
        self.election.is_leader().await.unwrap_or(false)
    }

    /// Gracefully shutdown and release leadership
    pub async fn shutdown(&self) -> Result<()> {
        info!(
            instance_id = %self.config.instance_id,
            "Shutting down leadership election - releasing leadership"
        );
        self.election.release_leadership().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use deadpool_redis::redis::Client;
    use tokio::time::{Duration, sleep};

    // Helper to create test config
    fn create_test_config() -> LeadershipConfig {
        LeadershipConfig {
            scope: LeadershipScope::Custom("test".to_string()),
            election_key: format!("test:leadership:{}", Uuid::new_v4()),
            retry_interval_secs: 1,
            leadership_ttl_secs: 5,
            instance_id: Uuid::new_v4().to_string(),
            enabled: true,
        }
    }

    // Helper to create Redis client for testing
    fn create_test_redis_client() -> Result<RedisClient> {
        // Use environment variable or default to localhost
        let redis_url =
            std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());
        Ok(Client::open(redis_url)?)
    }

    #[tokio::test]
    #[ignore] // Requires Redis connection
    async fn test_leadership_acquisition() -> Result<()> {
        let client = create_test_redis_client()?;
        let config = create_test_config();
        let election = RedisLeadershipElection::new(client, config);

        // Should not be leader initially
        assert!(!election.is_leader().await?);

        // Should be able to acquire leadership
        assert!(election.try_acquire_leadership().await?);
        assert!(election.is_leader().await?);

        // Should not be able to acquire leadership again
        assert!(!election.try_acquire_leadership().await?);

        // Clean up
        election.release_leadership().await?;
        Ok(())
    }

    #[tokio::test]
    #[ignore] // Requires Redis connection
    async fn test_leadership_maintenance() -> Result<()> {
        let client = create_test_redis_client()?;
        let config = create_test_config();
        let election = RedisLeadershipElection::new(client, config);

        // Acquire leadership
        assert!(election.try_acquire_leadership().await?);

        // Should be able to maintain leadership
        let status = election.maintain_leadership().await?;
        assert_eq!(status, LeadershipStatus::Leader);

        // Clean up
        election.release_leadership().await?;
        Ok(())
    }

    #[tokio::test]
    #[ignore] // Requires Redis connection
    async fn test_leadership_expiration() -> Result<()> {
        let client = create_test_redis_client()?;
        let mut config = create_test_config();
        config.leadership_ttl_secs = 1; // Very short TTL for testing
        let election = RedisLeadershipElection::new(client, config);

        // Acquire leadership
        assert!(election.try_acquire_leadership().await?);
        assert!(election.is_leader().await?);

        // Wait for expiration
        sleep(Duration::from_secs(2)).await;

        // Should no longer be leader
        assert!(!election.is_leader().await?);

        Ok(())
    }

    #[tokio::test]
    #[ignore] // Requires Redis connection
    async fn test_multiple_instances() -> Result<()> {
        let client1 = create_test_redis_client()?;
        let client2 = create_test_redis_client()?;

        let base_config = create_test_config();
        let config1 = LeadershipConfig {
            instance_id: "instance-1".to_string(),
            ..base_config.clone()
        };
        let config2 = LeadershipConfig {
            instance_id: "instance-2".to_string(),
            election_key: base_config.election_key.clone(), // Same election key
            ..base_config
        };

        let election1 = RedisLeadershipElection::new(client1, config1);
        let election2 = RedisLeadershipElection::new(client2, config2);

        // First instance should acquire leadership
        assert!(election1.try_acquire_leadership().await?);
        assert!(election1.is_leader().await?);
        assert!(!election2.is_leader().await?);

        // Second instance should not be able to acquire leadership
        assert!(!election2.try_acquire_leadership().await?);
        assert!(!election2.is_leader().await?);

        // After first instance releases, second should be able to acquire
        election1.release_leadership().await?;
        assert!(election2.try_acquire_leadership().await?);
        assert!(election2.is_leader().await?);
        assert!(!election1.is_leader().await?);

        // Clean up
        election2.release_leadership().await?;
        Ok(())
    }

    #[tokio::test]
    #[ignore] // Requires Redis connection
    async fn test_graceful_release() -> Result<()> {
        let client = create_test_redis_client()?;
        let config = create_test_config();
        let election = RedisLeadershipElection::new(client, config);

        // Acquire leadership
        assert!(election.try_acquire_leadership().await?);
        assert!(election.is_leader().await?);

        // Release leadership
        election.release_leadership().await?;
        assert!(!election.is_leader().await?);

        // Should be able to acquire again after release
        assert!(election.try_acquire_leadership().await?);

        // Clean up
        election.release_leadership().await?;
        Ok(())
    }
}
