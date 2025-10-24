use crate::errors::ConfigError;
use crate::leadership::{LeadershipConfig, LeadershipScope};
use atproto_identity::key::{KeyType, identify_key, to_public};
use axum_extra::extract::cookie::Key;
use base64::{Engine as _, engine::general_purpose};
use ordermap::OrderMap;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

type Result<T> = std::result::Result<T, ConfigError>;

/// HTTP server port configuration.
///
/// Wraps a u16 port number for the HTTP server. Provides type safety
/// and validation for port values.
#[derive(Clone)]
pub struct HttpPort(u16);

/// HTTP cookie encryption key configuration.
///
/// Wraps an Axum cookie key used for encrypting session cookies.
/// The key must be exactly 64 bytes when base64 decoded.
#[derive(Clone)]
pub struct HttpCookieKey(Key);

/// AT Protocol signing keys configuration.
///
/// Contains a map of public keys to their corresponding private keys
/// for signing AT Protocol records and tokens. Keys are validated
/// during construction to ensure they are valid cryptographic keypairs.
#[derive(Clone, Debug)]
pub struct SigningKeys(OrderMap<String, String>);

/// HTTP client timeout configuration.
///
/// Specifies the default timeout duration for HTTP client requests
/// made by the service, such as webhook deliveries and external API calls.
#[derive(Clone)]
pub struct HttpClientTimeout(std::time::Duration);

/// OAuth configuration for AT Protocol authentication.
///
/// Contains the OAuth client configuration used for authenticating users
/// through their AT Protocol provider (typically their PDS).
#[derive(Clone, Debug)]
pub struct OAuthConfig {
    pub hostname: String,
    pub client_id: String,
    pub client_secret: String,
    /// OAuth scopes for ATProtocol authentication
    ///
    /// ATProtocol OAuth scopes define what permissions and capabilities your service
    /// will have when users authenticate. Different scopes provide access to different
    /// parts of the ATProtocol ecosystem:
    ///
    /// - `account:email`: Allows the identity email address to be read from their PDS
    ///
    /// Be mindful that requesting excessive scopes may reduce user trust, while
    /// requesting insufficient scopes will limit your service's functionality.
    ///
    /// Default: "openid email profile atproto account:email"
    pub scope: String,
}

/// Service issuer DID configuration.
///
/// The DID (Decentralized Identifier) that identifies this service instance
/// in the AT Protocol network. Must be a valid DID format starting with "did:".
#[derive(Clone)]
pub struct IssuerDid(String);

/// Service issuer cryptographic key configuration.
///
/// Contains the private key data used by this service for signing AT Protocol
/// records and authentication tokens. The key is validated during construction.
#[derive(Clone)]
pub struct IssuerKey(atproto_identity::key::KeyData);

/// Identity cache size configuration.
///
/// Specifies the maximum number of DID resolution results to cache in memory.
/// Larger caches reduce DID resolution latency but use more memory.
#[derive(Clone, Debug)]
pub struct IdentityCacheSize(usize);

impl Default for IdentityCacheSize {
    fn default() -> Self {
        Self(1000)
    }
}

impl TryFrom<String> for IdentityCacheSize {
    type Error = ConfigError;

    fn try_from(value: String) -> Result<Self> {
        let size = value
            .parse::<usize>()
            .map_err(|_| ConfigError::InvalidKeyFormat {
                details: format!("Invalid cache size: {}", value),
            })?;

        if size == 0 {
            return Err(ConfigError::InvalidKeyFormat {
                details: "Cache size must be greater than 0".to_string(),
            });
        }

        Ok(Self(size))
    }
}

impl AsRef<usize> for IdentityCacheSize {
    fn as_ref(&self) -> &usize {
        &self.0
    }
}

/// Identity cache TTL (time-to-live) configuration.
///
/// Specifies how long (in minutes) DID resolution results should be cached
/// before being considered stale and requiring re-resolution.
#[derive(Clone, Debug)]
pub struct IdentityCacheTtlMinutes(u32);

impl Default for IdentityCacheTtlMinutes {
    fn default() -> Self {
        Self(30)
    }
}

impl TryFrom<String> for IdentityCacheTtlMinutes {
    type Error = ConfigError;

    fn try_from(value: String) -> Result<Self> {
        let minutes = value
            .parse::<u32>()
            .map_err(|_| ConfigError::InvalidKeyFormat {
                details: format!("Invalid TTL minutes: {}", value),
            })?;

        if minutes == 0 {
            return Err(ConfigError::InvalidKeyFormat {
                details: "TTL must be greater than 0 minutes".to_string(),
            });
        }

        Ok(Self(minutes))
    }
}

impl IdentityCacheTtlMinutes {
    pub fn to_seconds(&self) -> i64 {
        (self.0 as i64) * 60
    }
}

impl AsRef<u32> for IdentityCacheTtlMinutes {
    fn as_ref(&self) -> &u32 {
        &self.0
    }
}

/// Blueprint cache reload interval configuration.
///
/// Specifies the minimum interval (in seconds) between blueprint cache reloads
/// to prevent excessive database queries when blueprints are updated frequently.
#[derive(Clone, Debug)]
pub struct BlueprintCacheReloadSeconds(u32);

impl Default for BlueprintCacheReloadSeconds {
    fn default() -> Self {
        Self(30) // Default to 30 seconds
    }
}

impl TryFrom<String> for BlueprintCacheReloadSeconds {
    type Error = ConfigError;

    fn try_from(value: String) -> Result<Self> {
        let seconds = value
            .parse::<u32>()
            .map_err(|_| ConfigError::InvalidKeyFormat {
                details: format!("Invalid cache reload seconds: {}", value),
            })?;

        if seconds == 0 {
            return Err(ConfigError::InvalidKeyFormat {
                details: "Cache reload interval must be greater than 0 seconds".to_string(),
            });
        }

        Ok(Self(seconds))
    }
}

impl AsRef<u32> for BlueprintCacheReloadSeconds {
    fn as_ref(&self) -> &u32 {
        &self.0
    }
}

impl BlueprintCacheReloadSeconds {
    pub fn to_duration(&self) -> std::time::Duration {
        std::time::Duration::from_secs(self.0 as u64)
    }
}

/// Configuration for webhook processing
#[derive(Clone, Debug)]
pub struct WebhookQueueConfig {
    /// Whether to enable async webhook queue processing
    pub enabled: bool,
    /// Maximum number of concurrent webhook requests
    pub max_concurrent: usize,
    /// Default timeout for webhook requests in milliseconds
    pub default_timeout_ms: u64,
    /// Maximum retry attempts for failed webhooks
    pub max_retries: u32,
    /// Initial retry delay in milliseconds
    pub retry_delay_ms: u64,
    /// Whether to log webhook request/response bodies
    pub log_bodies: bool,
    /// Size of the webhook work queue (for mpsc channel)
    pub queue_size: usize,
}

impl Default for WebhookQueueConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_concurrent: 10,
            default_timeout_ms: 30000,
            max_retries: 3,
            retry_delay_ms: 1000,
            log_bodies: false,
            queue_size: 500,  // Reduced from 1000 to limit memory usage
        }
    }
}

/// Partition key strategy for distributing events across workers
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum PartitionKeyStrategy {
    /// Partition by DID - ensures all events from same user go to same worker
    DidBased,
    /// Partition by collection - ensures all events from same collection go to same worker
    CollectionBased,
    /// Round-robin distribution (no specific partitioning)
    RoundRobin,
    /// Custom field-based partitioning
    CustomField(String),
}

impl Default for PartitionKeyStrategy {
    fn default() -> Self {
        Self::DidBased
    }
}

impl PartitionKeyStrategy {
    pub fn from_string(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "did" | "did-based" => Ok(Self::DidBased),
            "collection" | "collection-based" => Ok(Self::CollectionBased),
            "round-robin" | "roundrobin" => Ok(Self::RoundRobin),
            s if s.starts_with("custom:") => {
                let field = s.strip_prefix("custom:").unwrap().to_string();
                Ok(Self::CustomField(field))
            }
            _ => Err(ConfigError::InvalidKeyFormat {
                details: format!("Invalid partition key strategy: {}", s),
            }),
        }
    }
}

/// Jetstream consumer configuration
#[derive(Clone, Debug)]
pub struct JetstreamConfig {
    /// Whether Jetstream consumer is enabled
    pub enabled: bool,
    /// Jetstream hostname
    pub hostname: String,
    /// Collections to filter (empty means all)
    pub collections: Vec<String>,
    /// Cursor storage path (file-based)
    pub cursor_path: Option<String>,
    /// Instance configuration for distributed processing
    pub instance_id: usize,
    /// Total number of instances for distributed processing
    pub total_instances: usize,
    /// Number of worker threads per instance
    pub worker_threads: usize,
    /// Partition key strategy
    pub partition_strategy: PartitionKeyStrategy,
}

impl JetstreamConfig {
    /// Validate the configuration
    pub fn validate(&self) -> Result<()> {
        // Validate instance configuration
        if self.instance_id >= self.total_instances {
            return Err(ConfigError::InvalidKeyFormat {
                details: format!(
                    "Instance ID {} must be less than total instances {}",
                    self.instance_id, self.total_instances
                ),
            });
        }

        // Validate worker threads
        if self.worker_threads == 0 {
            return Err(ConfigError::InvalidKeyFormat {
                details: "Worker threads must be at least 1".to_string(),
            });
        }

        // Validate hostname
        if self.enabled && self.hostname.is_empty() {
            return Err(ConfigError::InvalidKeyFormat {
                details: "Jetstream hostname cannot be empty when enabled".to_string(),
            });
        }

        // Validate collections (warning only)
        if self.enabled && self.collections.is_empty() {
            tracing::warn!("No collections configured for Jetstream - will receive all events");
        }

        // Log partition strategy for clarity
        if self.total_instances > 1 {
            tracing::info!(
                "Jetstream configured for distributed processing: instance {}/{} with {} worker threads using {:?} strategy",
                self.instance_id,
                self.total_instances,
                self.worker_threads,
                self.partition_strategy
            );
        } else {
            tracing::info!(
                "Jetstream configured for single instance with {} worker threads",
                self.worker_threads
            );
        }

        Ok(())
    }

    pub fn from_env() -> Result<Self> {
        let enabled = std::env::var("JETSTREAM_ENABLED")
            .map(|v| v.to_lowercase() == "true")
            .unwrap_or(false);

        let hostname = default_env("JETSTREAM_HOSTNAME", "jetstream2.us-east.bsky.network");

        let collections: Vec<String> = std::env::var("JETSTREAM_COLLECTIONS")
            .unwrap_or_default()
            .split(',')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect();

        let cursor_path = std::env::var("JETSTREAM_CURSOR_PATH").ok();

        // Parse distributed processing configuration
        // Support both old format ("0/3") and new format (separate vars)
        let (instance_id, total_instances) =
            if let Ok(partition) = std::env::var("JETSTREAM_PARTITION") {
                // Legacy format: "0/3"
                let parts: Vec<&str> = partition.split('/').collect();
                if parts.len() == 2 {
                    let id = parts[0].parse::<usize>().unwrap_or(0);
                    let total = parts[1].parse::<usize>().unwrap_or(1).max(1);
                    (id, total)
                } else {
                    (0, 1)
                }
            } else {
                // New format: separate environment variables
                let id = std::env::var("JETSTREAM_INSTANCE_ID")
                    .unwrap_or_else(|_| "0".to_string())
                    .parse::<usize>()
                    .unwrap_or(0);
                let total = std::env::var("JETSTREAM_TOTAL_INSTANCES")
                    .unwrap_or_else(|_| "1".to_string())
                    .parse::<usize>()
                    .unwrap_or(1)
                    .max(1);
                (id, total)
            };

        // Validate instance configuration
        if instance_id >= total_instances {
            return Err(ConfigError::InvalidKeyFormat {
                details: format!(
                    "Instance ID {} must be less than total instances {}",
                    instance_id, total_instances
                ),
            });
        }

        // Worker threads per instance (renamed from consumer_count for clarity)
        let worker_threads = std::env::var("JETSTREAM_WORKER_THREADS")
            .or_else(|_| std::env::var("JETSTREAM_CONSUMER_COUNT")) // Backward compatibility
            .unwrap_or_else(|_| "1".to_string())
            .parse::<usize>()
            .unwrap_or(1)
            .max(1);

        // Partition strategy
        let partition_strategy = std::env::var("JETSTREAM_PARTITION_STRATEGY")
            .map(|s| PartitionKeyStrategy::from_string(&s))
            .unwrap_or_else(|_| Ok(PartitionKeyStrategy::default()))?;

        let config = Self {
            enabled,
            hostname,
            collections,
            cursor_path,
            instance_id,
            total_instances,
            worker_threads,
            partition_strategy,
        };

        // Validate configuration before returning
        config.validate()?;
        Ok(config)
    }

    /// Check if this instance should process a given event based on partitioning
    pub fn should_process_event(&self, partition_key: &str) -> bool {
        if self.total_instances <= 1 {
            return true; // No partitioning
        }

        let hash = Self::hash_key(partition_key);
        let target_instance = (hash as usize) % self.total_instances;
        target_instance == self.instance_id
    }

    /// Simple hash function for partition keys
    fn hash_key(key: &str) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        key.hash(&mut hasher);
        hasher.finish()
    }
}

impl WebhookQueueConfig {
    pub fn from_env() -> Self {
        Self {
            enabled: std::env::var("WEBHOOK_QUEUE_ENABLED")
                .map(|v| v.to_lowercase() == "true")
                .unwrap_or(false),
            max_concurrent: std::env::var("WEBHOOK_MAX_CONCURRENT")
                .unwrap_or_else(|_| "10".to_string())
                .parse::<usize>()
                .unwrap_or(10)
                .max(1),
            default_timeout_ms: std::env::var("WEBHOOK_DEFAULT_TIMEOUT_MS")
                .unwrap_or_else(|_| "30000".to_string())
                .parse::<u64>()
                .unwrap_or(30000)
                .max(1000),
            max_retries: std::env::var("WEBHOOK_MAX_RETRIES")
                .unwrap_or_else(|_| "3".to_string())
                .parse::<u32>()
                .unwrap_or(3),
            retry_delay_ms: std::env::var("WEBHOOK_RETRY_DELAY_MS")
                .unwrap_or_else(|_| "1000".to_string())
                .parse::<u64>()
                .unwrap_or(1000)
                .max(100),
            log_bodies: std::env::var("WEBHOOK_LOG_BODIES")
                .map(|v| v.to_lowercase() == "true")
                .unwrap_or(false),
            queue_size: std::env::var("WEBHOOK_QUEUE_SIZE")
                .unwrap_or_else(|_| "500".to_string())
                .parse::<usize>()
                .unwrap_or(500)
                .max(10),
        }
    }
}

/// Configuration for blueprint queue processing
#[derive(Clone, Debug)]
pub struct BlueprintQueueConfig {
    /// Queue adapter type (mpsc, redis)
    pub adapter_type: String,
    /// Buffer size for MPSC queue
    pub mpsc_buffer_size: usize,
    /// Redis queue name prefix (if using Redis)
    pub redis_queue_prefix: String,
    /// Worker ID for Redis queue (if using Redis)
    pub redis_worker_id: Option<String>,
    /// Maximum retry attempts for queue operations
    pub max_retries: u32,
    /// Enable queue health monitoring
    pub enable_health_check: bool,
    /// Health check interval in seconds
    pub health_check_interval_secs: u64,
}

/// Configuration for periodic scheduler
#[derive(Clone, Debug)]
pub struct SchedulerConfig {
    /// Whether the scheduler is enabled
    pub enabled: bool,
    /// Interval between scheduler checks in seconds
    pub check_interval_secs: u64,
    /// Cache reload interval for periodic blueprints in seconds
    pub cache_reload_secs: u64,
    /// Maximum number of concurrent blueprint evaluations
    pub max_concurrent_evaluations: usize,
    /// Enable detailed scheduler logging
    pub debug_logging: bool,
}

/// Configuration for evaluation result storage
#[derive(Clone, Debug)]
pub struct EvaluationStorageConfig {
    /// Storage backend type: "tracing" (default), "filesystem", or "noop"
    pub storage_type: String,
    /// Base directory for filesystem storage (required if storage_type is "filesystem")
    pub filesystem_base_directory: Option<String>,
}

impl Default for BlueprintQueueConfig {
    fn default() -> Self {
        Self {
            adapter_type: "mpsc".to_string(),
            mpsc_buffer_size: 1000,  // Reduced from 5000 to limit memory usage
            redis_queue_prefix: "queue:blueprint:".to_string(),
            redis_worker_id: None,
            max_retries: 3,
            enable_health_check: false,
            health_check_interval_secs: 60,
        }
    }
}

impl BlueprintQueueConfig {
    pub fn from_env() -> Self {
        Self {
            adapter_type: std::env::var("BLUEPRINT_QUEUE_ADAPTER")
                .unwrap_or_else(|_| "mpsc".to_string())
                .to_lowercase(),
            mpsc_buffer_size: std::env::var("BLUEPRINT_QUEUE_BUFFER_SIZE")
                .unwrap_or_else(|_| "1000".to_string())
                .parse::<usize>()
                .unwrap_or(1000)
                .max(100),
            redis_queue_prefix: std::env::var("BLUEPRINT_QUEUE_REDIS_PREFIX")
                .unwrap_or_else(|_| "queue:blueprint:".to_string()),
            redis_worker_id: std::env::var("BLUEPRINT_QUEUE_REDIS_WORKER_ID").ok(),
            max_retries: std::env::var("BLUEPRINT_QUEUE_MAX_RETRIES")
                .unwrap_or_else(|_| "3".to_string())
                .parse::<u32>()
                .unwrap_or(3),
            enable_health_check: std::env::var("BLUEPRINT_QUEUE_HEALTH_CHECK")
                .map(|v| v.to_lowercase() == "true")
                .unwrap_or(false),
            health_check_interval_secs: std::env::var("BLUEPRINT_QUEUE_HEALTH_INTERVAL")
                .unwrap_or_else(|_| "60".to_string())
                .parse::<u64>()
                .unwrap_or(60)
                .max(10),
        }
    }

    /// Validate the configuration
    pub fn validate(&self) -> Result<()> {
        // Validate adapter type
        match self.adapter_type.as_str() {
            "mpsc" | "redis" => {}
            _ => {
                return Err(ConfigError::InvalidKeyFormat {
                    details: format!(
                        "Invalid blueprint queue adapter type: {}. Must be 'mpsc' or 'redis'",
                        self.adapter_type
                    ),
                });
            }
        }

        // Validate buffer size for MPSC
        if self.adapter_type == "mpsc" && self.mpsc_buffer_size < 10 {
            return Err(ConfigError::InvalidKeyFormat {
                details: "MPSC buffer size must be at least 10".to_string(),
            });
        }

        // Log configuration for clarity
        tracing::info!(
            "Blueprint queue configured: adapter={}, buffer_size={}, health_check={}",
            self.adapter_type,
            self.mpsc_buffer_size,
            self.enable_health_check
        );

        Ok(())
    }
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            check_interval_secs: 10,
            cache_reload_secs: 60,
            max_concurrent_evaluations: 5,
            debug_logging: false,
        }
    }
}

impl SchedulerConfig {
    pub fn from_env() -> Self {
        Self {
            enabled: std::env::var("SCHEDULER_ENABLED")
                .map(|v| v.to_lowercase() == "true")
                .unwrap_or(false),
            check_interval_secs: std::env::var("SCHEDULER_CHECK_INTERVAL_SECS")
                .unwrap_or_else(|_| "10".to_string())
                .parse::<u64>()
                .unwrap_or(10)
                .max(1),
            cache_reload_secs: std::env::var("SCHEDULER_CACHE_RELOAD_SECS")
                .unwrap_or_else(|_| "60".to_string())
                .parse::<u64>()
                .unwrap_or(60)
                .max(10),
            max_concurrent_evaluations: std::env::var("SCHEDULER_MAX_CONCURRENT")
                .unwrap_or_else(|_| "5".to_string())
                .parse::<usize>()
                .unwrap_or(5)
                .max(1),
            debug_logging: std::env::var("SCHEDULER_DEBUG_LOGGING")
                .map(|v| v.to_lowercase() == "true")
                .unwrap_or(false),
        }
    }

    /// Validate the configuration
    pub fn validate(&self) -> Result<()> {
        // Validate check interval
        if self.enabled && self.check_interval_secs == 0 {
            return Err(ConfigError::InvalidKeyFormat {
                details: "Scheduler check interval must be at least 1 second".to_string(),
            });
        }

        // Log configuration for clarity
        if self.enabled {
            tracing::info!(
                "Scheduler configured: check_interval={}s, cache_reload={}s, max_concurrent={}",
                self.check_interval_secs,
                self.cache_reload_secs,
                self.max_concurrent_evaluations
            );
        }

        Ok(())
    }
}

impl Default for EvaluationStorageConfig {
    fn default() -> Self {
        Self {
            storage_type: "tracing".to_string(),
            filesystem_base_directory: None,
        }
    }
}

impl EvaluationStorageConfig {
    pub fn from_env() -> Result<Self> {
        let storage_type = std::env::var("EVALUATION_STORAGE_TYPE")
            .unwrap_or_else(|_| "tracing".to_string())
            .to_lowercase();

        // Validate storage type
        match storage_type.as_str() {
            "tracing" | "filesystem" | "noop" => {}
            _ => {
                return Err(ConfigError::InvalidKeyFormat {
                    details: format!(
                        "Invalid EVALUATION_STORAGE_TYPE '{}'. Must be 'tracing', 'filesystem', or 'noop'",
                        storage_type
                    ),
                });
            }
        }

        // Get filesystem base directory if needed
        let filesystem_base_directory = if storage_type == "filesystem" {
            let dir = std::env::var("EVALUATION_STORAGE_DIRECTORY")
                .map_err(|_| ConfigError::InvalidKeyFormat {
                    details: "EVALUATION_STORAGE_DIRECTORY must be set when using 'filesystem' storage type".to_string(),
                })?;
            Some(dir)
        } else {
            std::env::var("EVALUATION_STORAGE_DIRECTORY").ok()
        };

        Ok(Self {
            storage_type,
            filesystem_base_directory,
        })
    }

    /// Validate the configuration
    pub fn validate(&self) -> Result<()> {
        // Validate that filesystem directory is set when using filesystem storage
        if self.storage_type == "filesystem" && self.filesystem_base_directory.is_none() {
            return Err(ConfigError::InvalidKeyFormat {
                details:
                    "EVALUATION_STORAGE_DIRECTORY must be set when using 'filesystem' storage type"
                        .to_string(),
            });
        }

        // Log configuration for clarity
        match self.storage_type.as_str() {
            "filesystem" => {
                println!(
                    "Evaluation storage configured: type=filesystem, directory={}",
                    self.filesystem_base_directory.as_ref().unwrap()
                );
            }
            storage_type => {
                println!("Evaluation storage configured: type={}", storage_type);
            }
        }

        Ok(())
    }
}

/// Main configuration structure for the ifthisthenat service.
///
/// This structure contains all configuration parameters needed to run the service,
/// including HTTP server settings, database connections, AT Protocol configuration,
/// and feature flags. Configuration is typically loaded from environment variables
/// using the `Config::new()` method.
///
/// # Configuration Sources
///
/// Configuration is loaded from environment variables with sensible defaults where
/// appropriate. Required variables will cause startup to fail if not provided.
///
/// # Examples
///
/// ```rust,ignore
/// use ifthisthenat::config::Config;
///
/// // Load configuration from environment variables
/// let config = Config::new()?;
///
/// // Access configuration values
/// println!("Service running on port: {}", config.http_port.as_ref());
/// println!("Database URL: {}", config.database_url);
/// ```
#[derive(Clone)]
pub struct Config {
    pub version: String,
    pub http_port: HttpPort,
    pub http_cookie_key: HttpCookieKey,
    pub http_static_path: String,
    pub external_base: String,
    pub database_url: String,
    pub user_agent: String,
    pub plc_hostname: String,
    pub resolve_handle_hostname: String,
    pub http_client_timeout: HttpClientTimeout,
    pub issuer_did: IssuerDid,
    pub oauth: OAuthConfig,
    pub admin_dids: HashSet<String>,
    pub allowed_identities: HashSet<String>,
    pub identity_cache_size: IdentityCacheSize,
    pub identity_cache_ttl_minutes: IdentityCacheTtlMinutes,
    pub jetstream: JetstreamConfig,
    // Redis configuration for Jetstream cursor storage
    pub redis_url: Option<String>,
    pub redis_cursor_key: String,
    pub redis_cursor_ttl_seconds: u64,
    // Blueprint cache configuration
    pub blueprint_cache_reload_seconds: BlueprintCacheReloadSeconds,
    pub blueprint_cache_max_size: Option<usize>,
    // AIP integration fields for app-password functionality
    pub aip_base_url: Option<String>,
    pub aip_client_id: Option<String>,
    pub aip_client_secret: Option<String>,
    pub aip_oauth_scope: Option<String>,
    // Sentry configuration for error and performance monitoring
    pub sentry_dsn: Option<String>,
    pub sentry_environment: Option<String>,
    pub sentry_traces_sample_rate: f32,
    pub sentry_debug: bool,
    // Webhook queue configuration
    pub webhook_queue: WebhookQueueConfig,
    // Blueprint queue configuration
    pub blueprint_queue: BlueprintQueueConfig,
    // Disabled node types for blueprint evaluation
    pub disabled_node_types: HashSet<String>,
    // Scheduler configuration for periodic entry nodes
    pub scheduler: SchedulerConfig,
    // Allowed collections for publish_record nodes (empty = all allowed)
    pub allowed_publish_collections: HashSet<String>,
    // Leadership election configuration for distributed blueprint evaluation
    pub blueprint_leadership: LeadershipConfig,
    // Leadership election configuration for webhook processing
    pub webhook_leadership: LeadershipConfig,
    // Leadership election configuration for scheduler
    pub scheduler_leadership: LeadershipConfig,
    // Blueprint throttler configuration
    pub blueprint_throttler: String,
    pub blueprint_throttler_authority_limit: Option<i32>,
    pub blueprint_throttler_authority_window: Option<i32>,
    pub blueprint_throttler_aturi_limit: Option<i32>,
    pub blueprint_throttler_aturi_window: Option<i32>,
    // Per-identity throttler defaults (used when throttler is "redis-per-identity")
    pub blueprint_throttler_authority_default_limit: Option<i32>,
    pub blueprint_throttler_authority_default_window: Option<i32>,
    pub blueprint_throttler_aturi_default_limit: Option<i32>,
    pub blueprint_throttler_aturi_default_window: Option<i32>,
    // Evaluation result storage configuration
    pub evaluation_storage: EvaluationStorageConfig,
    // Denylist configuration
    pub denylist_type: String,
}

impl Config {
    /// Creates a new configuration instance by loading values from environment variables.
    ///
    /// This method reads configuration from environment variables, applies validation,
    /// and constructs a complete configuration object. It will return an error if
    /// required environment variables are missing or if any values are invalid.
    ///
    /// # Required Environment Variables
    ///
    /// - `EXTERNAL_BASE`: Base URL for the service
    /// - `HTTP_COOKIE_KEY`: Base64-encoded 64-byte key for cookie encryption
    /// - `ISSUER_DID`: DID identifying this service instance
    /// - `AIP_HOSTNAME`: OAuth provider hostname
    /// - `AIP_CLIENT_ID`: OAuth client ID
    /// - `AIP_CLIENT_SECRET`: OAuth client secret
    ///
    /// # Optional Environment Variables
    ///
    /// Many environment variables have defaults. See individual field documentation
    /// and the CLAUDE.md file for a complete list of configurable options.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError`] if:
    /// - Required environment variables are missing
    /// - Environment variable values are invalid (wrong format, out of range, etc.)
    /// - Configuration validation fails (e.g., invalid DID format, malformed keys)
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// use ifthisthenat::config::Config;
    ///
    /// // Load configuration from environment
    /// match Config::new() {
    ///     Ok(config) => {
    ///         println!("Configuration loaded successfully");
    ///         println!("Service version: {}", config.version);
    ///     }
    ///     Err(e) => {
    ///         eprintln!("Failed to load configuration: {}", e);
    ///         std::process::exit(1);
    ///     }
    /// }
    /// ```
    pub fn new() -> Result<Self> {
        let http_port: HttpPort = default_env("HTTP_PORT", "8080").try_into()?;
        let http_cookie_key: HttpCookieKey =
            require_env("HTTP_COOKIE_KEY").and_then(|value| value.try_into())?;
        let http_static_path = default_env("HTTP_STATIC_PATH", "static");

        let external_base = require_env("EXTERNAL_BASE")?;
        let database_url = default_env(
            "DATABASE_URL",
            "postgres://username:password@localhost:5432/ifthisthenat",
        );
        let default_user_agent = format!("ifthisthenat/{}", version()?);
        let user_agent = default_env("USER_AGENT", &default_user_agent);
        let plc_hostname = default_env("PLC_HOSTNAME", "plc.directory");
        let resolve_handle_hostname =
            default_env("RESOLVE_HANDLE_HOSTNAME", "quickdid.smokesignal.tools");
        let http_client_timeout: HttpClientTimeout =
            default_env("HTTP_CLIENT_TIMEOUT", "8").try_into()?;
        let issuer_did: IssuerDid = require_env("ISSUER_DID").and_then(|value| value.try_into())?;

        let client_id = require_env("AIP_CLIENT_ID")?;
        let client_secret = require_env("AIP_CLIENT_SECRET")?;

        // OAuth configuration for AIP
        let oauth = OAuthConfig {
            hostname: require_env("AIP_HOSTNAME")?,
            client_id,
            client_secret,
            scope: default_env(
                "AIP_OAUTH_SCOPE",
                "openid email profile atproto account:email",
            ),
        };

        let admin_dids: HashSet<String> = optional_env("ADMIN_DIDS")
            .split(';')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect();

        // Parse and validate allowed identities (DIDs)
        let allowed_identities: HashSet<String> = optional_env("ALLOWED_IDENTITIES")
            .split(',')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .filter_map(|did| {
                // Validate that the value is a valid DID
                if atproto_identity::validation::is_valid_did_method_plc(did)
                    || atproto_identity::validation::is_valid_did_method_web(did, false)
                {
                    Some(did.to_string())
                } else {
                    tracing::warn!(did = %did, "Invalid DID in ALLOWED_IDENTITIES, skipping");
                    None
                }
            })
            .collect();

        let identity_cache_size: IdentityCacheSize = {
            let env_value = optional_env("IDENTITY_CACHE_SIZE");
            if env_value.is_empty() {
                IdentityCacheSize::default()
            } else {
                env_value.try_into()?
            }
        };

        let identity_cache_ttl_minutes: IdentityCacheTtlMinutes = {
            let env_value = optional_env("IDENTITY_CACHE_TTL_MINUTES");
            if env_value.is_empty() {
                IdentityCacheTtlMinutes::default()
            } else {
                env_value.try_into()?
            }
        };

        // Jetstream configuration
        let jetstream = JetstreamConfig::from_env()?;

        // Redis configuration for cursor storage
        let redis_url = std::env::var("REDIS_URL").ok();
        let redis_cursor_key = default_env("REDIS_CURSOR_KEY", "ifthisthenat:jetstream:cursor");
        let redis_cursor_ttl_seconds = std::env::var("REDIS_CURSOR_TTL_SECONDS")
            .unwrap_or_else(|_| "86400".to_string()) // Default to 24 hours
            .parse::<u64>()
            .unwrap_or(86400);

        // Blueprint cache reload interval configuration
        let blueprint_cache_reload_seconds: BlueprintCacheReloadSeconds = {
            let env_value = optional_env("BLUEPRINT_CACHE_RELOAD_SECONDS");
            if env_value.is_empty() {
                BlueprintCacheReloadSeconds::default()
            } else {
                env_value.try_into()?
            }
        };

        // Blueprint cache max size configuration (None = unlimited)
        let blueprint_cache_max_size = std::env::var("BLUEPRINT_CACHE_MAX_SIZE")
            .ok()
            .and_then(|v| v.parse::<usize>().ok());

        // Extract AIP configuration for backward compatibility
        let aip_base_url = Some(oauth.hostname.clone());
        let aip_client_id = Some(oauth.client_id.clone());
        let aip_client_secret = Some(oauth.client_secret.clone());
        let aip_oauth_scope = Some(oauth.scope.clone());

        // Sentry configuration - all optional
        let sentry_dsn = std::env::var("SENTRY_DSN").ok();
        let sentry_environment = std::env::var("SENTRY_ENVIRONMENT")
            .or_else(|_| std::env::var("ENV"))
            .or_else(|_| std::env::var("ENVIRONMENT"))
            .ok()
            .or_else(|| Some("development".to_string()));

        let sentry_traces_sample_rate = std::env::var("SENTRY_TRACES_SAMPLE_RATE")
            .unwrap_or_else(|_| "0.1".to_string())
            .parse::<f32>()
            .unwrap_or(0.1)
            .clamp(0.0, 1.0);

        let sentry_debug = std::env::var("SENTRY_DEBUG")
            .map(|v| v.to_lowercase() == "true")
            .unwrap_or(false);

        // Webhook queue configuration
        let webhook_queue = WebhookQueueConfig::from_env();

        // Blueprint queue configuration
        let blueprint_queue = BlueprintQueueConfig::from_env();
        blueprint_queue.validate()?;

        // Disabled node types configuration
        let disabled_node_types: HashSet<String> = optional_env("DISABLED_NODE_TYPES")
            .split(',')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect();

        // Scheduler configuration
        let scheduler = SchedulerConfig::from_env();

        // Allowed collections for publish_record nodes
        let allowed_publish_collections: HashSet<String> =
            optional_env("ALLOWED_PUBLISH_COLLECTIONS")
                .split(',')
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .collect();
        scheduler.validate()?;

        // Leadership election configurations for different tasks
        let blueprint_leadership =
            LeadershipConfig::from_env_for_scope(&LeadershipScope::BlueprintEvaluation);
        blueprint_leadership
            .validate()
            .map_err(|e| ConfigError::InvalidKeyFormat {
                details: format!("Blueprint leadership configuration error: {}", e),
            })?;

        let webhook_leadership =
            LeadershipConfig::from_env_for_scope(&LeadershipScope::WebhookProcessor);
        webhook_leadership
            .validate()
            .map_err(|e| ConfigError::InvalidKeyFormat {
                details: format!("Webhook leadership configuration error: {}", e),
            })?;

        let scheduler_leadership =
            LeadershipConfig::from_env_for_scope(&LeadershipScope::Scheduler);
        scheduler_leadership
            .validate()
            .map_err(|e| ConfigError::InvalidKeyFormat {
                details: format!("Scheduler leadership configuration error: {}", e),
            })?;

        // Blueprint throttler configuration
        let blueprint_throttler = default_env("BLUEPRINT_THROTTLER", "noop").to_lowercase();

        // Parse throttler configuration values
        let blueprint_throttler_authority_limit =
            std::env::var("BLUEPRINT_THROTTLER_AUTHORITY_LIMIT")
                .ok()
                .map(|v| v.parse::<i32>())
                .transpose()
                .map_err(|_| ConfigError::InvalidKeyFormat {
                    details: "BLUEPRINT_THROTTLER_AUTHORITY_LIMIT must be a valid integer"
                        .to_string(),
                })?;

        let blueprint_throttler_authority_window =
            std::env::var("BLUEPRINT_THROTTLER_AUTHORITY_WINDOW")
                .ok()
                .map(|v| v.parse::<i32>())
                .transpose()
                .map_err(|_| ConfigError::InvalidKeyFormat {
                    details: "BLUEPRINT_THROTTLER_AUTHORITY_WINDOW must be a valid integer"
                        .to_string(),
                })?;

        let blueprint_throttler_aturi_limit = std::env::var("BLUEPRINT_THROTTLER_ATURI_LIMIT")
            .ok()
            .map(|v| v.parse::<i32>())
            .transpose()
            .map_err(|_| ConfigError::InvalidKeyFormat {
                details: "BLUEPRINT_THROTTLER_ATURI_LIMIT must be a valid integer".to_string(),
            })?;

        let blueprint_throttler_aturi_window = std::env::var("BLUEPRINT_THROTTLER_ATURI_WINDOW")
            .ok()
            .map(|v| v.parse::<i32>())
            .transpose()
            .map_err(|_| ConfigError::InvalidKeyFormat {
                details: "BLUEPRINT_THROTTLER_ATURI_WINDOW must be a valid integer".to_string(),
            })?;

        // Per-identity throttler default fields (used when throttler is "redis-per-identity")
        let blueprint_throttler_authority_default_limit =
            std::env::var("BLUEPRINT_THROTTLER_AUTHORITY_DEFAULT_LIMIT")
                .ok()
                .map(|v| v.parse::<i32>())
                .transpose()
                .map_err(|_| ConfigError::InvalidKeyFormat {
                    details: "BLUEPRINT_THROTTLER_AUTHORITY_DEFAULT_LIMIT must be a valid integer"
                        .to_string(),
                })?;

        let blueprint_throttler_authority_default_window =
            std::env::var("BLUEPRINT_THROTTLER_AUTHORITY_DEFAULT_WINDOW")
                .ok()
                .map(|v| v.parse::<i32>())
                .transpose()
                .map_err(|_| ConfigError::InvalidKeyFormat {
                    details: "BLUEPRINT_THROTTLER_AUTHORITY_DEFAULT_WINDOW must be a valid integer"
                        .to_string(),
                })?;

        let blueprint_throttler_aturi_default_limit =
            std::env::var("BLUEPRINT_THROTTLER_ATURI_DEFAULT_LIMIT")
                .ok()
                .map(|v| v.parse::<i32>())
                .transpose()
                .map_err(|_| ConfigError::InvalidKeyFormat {
                    details: "BLUEPRINT_THROTTLER_ATURI_DEFAULT_LIMIT must be a valid integer"
                        .to_string(),
                })?;

        let blueprint_throttler_aturi_default_window =
            std::env::var("BLUEPRINT_THROTTLER_ATURI_DEFAULT_WINDOW")
                .ok()
                .map(|v| v.parse::<i32>())
                .transpose()
                .map_err(|_| ConfigError::InvalidKeyFormat {
                    details: "BLUEPRINT_THROTTLER_ATURI_DEFAULT_WINDOW must be a valid integer"
                        .to_string(),
                })?;

        // Validate blueprint throttler configuration
        match blueprint_throttler.as_str() {
            "noop" => {
                // No additional validation needed for noop
            }
            "redis" => {
                // At least one limit must be set for redis throttler
                if blueprint_throttler_authority_limit.is_none()
                    && blueprint_throttler_aturi_limit.is_none()
                {
                    return Err(ConfigError::InvalidKeyFormat {
                        details: "When BLUEPRINT_THROTTLER is 'redis', at least one of BLUEPRINT_THROTTLER_AUTHORITY_LIMIT or BLUEPRINT_THROTTLER_ATURI_LIMIT must be set".to_string(),
                    });
                }

                // Validate authority limit/window pairing
                match (
                    blueprint_throttler_authority_limit,
                    blueprint_throttler_authority_window,
                ) {
                    (None, None) => {
                        // Both None is valid
                    }
                    (Some(limit), Some(window)) => {
                        if limit <= 0 {
                            return Err(ConfigError::InvalidKeyFormat {
                                details:
                                    "BLUEPRINT_THROTTLER_AUTHORITY_LIMIT must be greater than 0"
                                        .to_string(),
                            });
                        }
                        if window <= 0 {
                            return Err(ConfigError::InvalidKeyFormat {
                                details:
                                    "BLUEPRINT_THROTTLER_AUTHORITY_WINDOW must be greater than 0"
                                        .to_string(),
                            });
                        }
                    }
                    _ => {
                        return Err(ConfigError::InvalidKeyFormat {
                            details: "When either BLUEPRINT_THROTTLER_AUTHORITY_LIMIT or BLUEPRINT_THROTTLER_AUTHORITY_WINDOW is set, both must be set".to_string(),
                        });
                    }
                }

                // Validate aturi limit/window pairing
                match (
                    blueprint_throttler_aturi_limit,
                    blueprint_throttler_aturi_window,
                ) {
                    (None, None) => {
                        // Both None is valid
                    }
                    (Some(limit), Some(window)) => {
                        if limit <= 0 {
                            return Err(ConfigError::InvalidKeyFormat {
                                details: "BLUEPRINT_THROTTLER_ATURI_LIMIT must be greater than 0"
                                    .to_string(),
                            });
                        }
                        if window <= 0 {
                            return Err(ConfigError::InvalidKeyFormat {
                                details: "BLUEPRINT_THROTTLER_ATURI_WINDOW must be greater than 0"
                                    .to_string(),
                            });
                        }
                    }
                    _ => {
                        return Err(ConfigError::InvalidKeyFormat {
                            details: "When either BLUEPRINT_THROTTLER_ATURI_LIMIT or BLUEPRINT_THROTTLER_ATURI_WINDOW is set, both must be set".to_string(),
                        });
                    }
                }

                // Redis must be configured if using redis throttler
                if redis_url.is_none() {
                    return Err(ConfigError::InvalidKeyFormat {
                        details:
                            "REDIS_URL must be configured when using 'redis' blueprint throttler"
                                .to_string(),
                    });
                }
            }
            "redis-per-identity" => {
                // All four default values must be set for redis-per-identity throttler
                if blueprint_throttler_authority_default_limit.is_none()
                    || blueprint_throttler_authority_default_window.is_none()
                    || blueprint_throttler_aturi_default_limit.is_none()
                    || blueprint_throttler_aturi_default_window.is_none()
                {
                    return Err(ConfigError::InvalidKeyFormat {
                        details: "When BLUEPRINT_THROTTLER is 'redis-per-identity', all four default values must be set: BLUEPRINT_THROTTLER_AUTHORITY_DEFAULT_LIMIT, BLUEPRINT_THROTTLER_AUTHORITY_DEFAULT_WINDOW, BLUEPRINT_THROTTLER_ATURI_DEFAULT_LIMIT, BLUEPRINT_THROTTLER_ATURI_DEFAULT_WINDOW".to_string(),
                    });
                }

                // Validate that all default values are greater than 0
                if let Some(limit) = blueprint_throttler_authority_default_limit {
                    if limit <= 0 {
                        return Err(ConfigError::InvalidKeyFormat {
                            details:
                                "BLUEPRINT_THROTTLER_AUTHORITY_DEFAULT_LIMIT must be greater than 0"
                                    .to_string(),
                        });
                    }
                }
                if let Some(window) = blueprint_throttler_authority_default_window {
                    if window <= 0 {
                        return Err(ConfigError::InvalidKeyFormat {
                            details: "BLUEPRINT_THROTTLER_AUTHORITY_DEFAULT_WINDOW must be greater than 0".to_string(),
                        });
                    }
                }
                if let Some(limit) = blueprint_throttler_aturi_default_limit {
                    if limit <= 0 {
                        return Err(ConfigError::InvalidKeyFormat {
                            details:
                                "BLUEPRINT_THROTTLER_ATURI_DEFAULT_LIMIT must be greater than 0"
                                    .to_string(),
                        });
                    }
                }
                if let Some(window) = blueprint_throttler_aturi_default_window {
                    if window <= 0 {
                        return Err(ConfigError::InvalidKeyFormat {
                            details:
                                "BLUEPRINT_THROTTLER_ATURI_DEFAULT_WINDOW must be greater than 0"
                                    .to_string(),
                        });
                    }
                }

                // Redis must be configured if using redis-per-identity throttler
                if redis_url.is_none() {
                    return Err(ConfigError::InvalidKeyFormat {
                        details:
                            "REDIS_URL must be configured when using 'redis-per-identity' blueprint throttler"
                                .to_string(),
                    });
                }
            }
            _ => {
                return Err(ConfigError::InvalidKeyFormat {
                    details: format!(
                        "Invalid BLUEPRINT_THROTTLER value '{}'. Must be 'noop', 'redis', or 'redis-per-identity'",
                        blueprint_throttler
                    ),
                });
            }
        }

        // Evaluation storage configuration
        let evaluation_storage = EvaluationStorageConfig::from_env()?;
        evaluation_storage.validate()?;

        // Denylist configuration - default to noop
        let denylist_type = default_env("DENYLIST_TYPE", "noop");

        // Validate denylist type
        match denylist_type.as_str() {
            "noop" => {
                println!("Denylist configured: type=noop (disabled)");
            }
            "postgres" => {
                println!("Denylist configured: type=postgres");
            }
            _ => {
                return Err(ConfigError::InvalidKeyFormat {
                    details: format!(
                        "Invalid DENYLIST_TYPE: '{}'. Must be 'noop' or 'postgres'",
                        denylist_type
                    ),
                });
            }
        }

        Ok(Self {
            version: version()?,
            http_port,
            http_cookie_key,
            http_static_path,
            external_base,
            database_url,
            user_agent,
            plc_hostname,
            resolve_handle_hostname,
            http_client_timeout,
            issuer_did,
            oauth,
            admin_dids,
            allowed_identities,
            identity_cache_size,
            identity_cache_ttl_minutes,
            jetstream,
            redis_url,
            redis_cursor_key,
            redis_cursor_ttl_seconds,
            blueprint_cache_reload_seconds,
            blueprint_cache_max_size,
            aip_base_url,
            aip_client_id,
            aip_client_secret,
            aip_oauth_scope,
            sentry_dsn,
            sentry_environment,
            sentry_traces_sample_rate,
            sentry_debug,
            webhook_queue,
            blueprint_queue,
            disabled_node_types,
            scheduler,
            allowed_publish_collections,
            blueprint_leadership,
            webhook_leadership,
            scheduler_leadership,
            blueprint_throttler,
            blueprint_throttler_authority_limit,
            blueprint_throttler_authority_window,
            blueprint_throttler_aturi_limit,
            blueprint_throttler_aturi_window,
            blueprint_throttler_authority_default_limit,
            blueprint_throttler_authority_default_window,
            blueprint_throttler_aturi_default_limit,
            blueprint_throttler_aturi_default_window,
            evaluation_storage,
            denylist_type,
        })
    }
}

/// Retrieves a required environment variable.
///
/// # Arguments
///
/// * `name` - The name of the environment variable to retrieve
///
/// # Returns
///
/// * `Ok(String)` - The environment variable value if present
/// * `Err(ConfigError::EnvVarRequired)` - If the environment variable is not set
fn require_env(name: &str) -> Result<String> {
    std::env::var(name).map_err(|_| ConfigError::EnvVarRequired {
        var_name: name.to_string(),
    })
}

/// Retrieves an optional environment variable, returning an empty string if not set.
///
/// # Arguments
///
/// * `name` - The name of the environment variable to retrieve
///
/// # Returns
///
/// The environment variable value, or an empty string if not set.
fn optional_env(name: &str) -> String {
    std::env::var(name).unwrap_or("".to_string())
}

/// Retrieves an environment variable with a default value if not set.
///
/// # Arguments
///
/// * `name` - The name of the environment variable to retrieve
/// * `default_value` - The default value to use if the environment variable is not set
///
/// # Returns
///
/// The environment variable value, or the default value if not set.
fn default_env(name: &str, default_value: &str) -> String {
    std::env::var(name).unwrap_or(default_value.to_string())
}

/// Retrieves the service version from compile-time environment variables.
///
/// This function attempts to get the version from either `GIT_HASH` or
/// `CARGO_PKG_VERSION` compile-time environment variables.
///
/// # Returns
///
/// * `Ok(String)` - The version string if available
/// * `Err(ConfigError::VersionNotAvailable)` - If no version information is available
pub fn version() -> Result<String> {
    option_env!("GIT_HASH")
        .or(option_env!("CARGO_PKG_VERSION"))
        .map(|val| val.to_string())
        .ok_or(ConfigError::VersionNotAvailable)
}

impl TryFrom<String> for HttpPort {
    type Error = ConfigError;
    fn try_from(value: String) -> Result<Self> {
        if value.is_empty() {
            Ok(Self(80))
        } else {
            value
                .parse::<u16>()
                .map(Self)
                .map_err(|_| ConfigError::InvalidPortNumber {
                    port: value.clone(),
                })
        }
    }
}

impl AsRef<u16> for HttpPort {
    fn as_ref(&self) -> &u16 {
        &self.0
    }
}

impl TryFrom<String> for HttpCookieKey {
    type Error = ConfigError;
    fn try_from(value: String) -> Result<Self> {
        let mut decoded_key: [u8; 66] = [0; 66];
        general_purpose::STANDARD_NO_PAD
            .decode_slice(value, &mut decoded_key)
            .map_err(|_| ConfigError::EnvVarRequired {
                var_name: "cookie".to_string(),
            })?;
        Key::try_from(&decoded_key[..64])
            .map_err(|_| ConfigError::EnvVarRequired {
                var_name: "cookie".to_string(),
            })
            .map(Self)
    }
}

impl AsRef<Key> for HttpCookieKey {
    fn as_ref(&self) -> &Key {
        &self.0
    }
}

impl AsRef<OrderMap<String, String>> for SigningKeys {
    fn as_ref(&self) -> &OrderMap<String, String> {
        &self.0
    }
}

impl TryFrom<String> for SigningKeys {
    type Error = ConfigError;
    fn try_from(value: String) -> Result<Self> {
        if value.is_empty() {
            return Ok(Self(OrderMap::new()));
        }

        let private_keys: Vec<&str> = value.split(';').filter(|s| !s.is_empty()).collect();

        if private_keys.is_empty() {
            return Ok(Self(OrderMap::new()));
        }

        let mut signing_keys = OrderMap::new();

        for private_key in private_keys {
            let key_data = match identify_key(private_key) {
                Ok(key_data) => key_data,
                Err(_) => {
                    continue;
                }
            };

            match key_data.0 {
                KeyType::P256Public | KeyType::K256Public => {
                    continue;
                }
                _ => {}
            }

            let public_key = match to_public(&key_data) {
                Ok(public_key) => public_key,
                Err(_) => {
                    continue;
                }
            };

            signing_keys.insert(public_key.to_string(), private_key.to_string());
        }

        if signing_keys.is_empty() {
            return Err(ConfigError::InvalidKeyFormat {
                details: "No valid signing keys found".to_string(),
            });
        }

        Ok(Self(signing_keys))
    }
}

impl AsRef<std::time::Duration> for HttpClientTimeout {
    fn as_ref(&self) -> &std::time::Duration {
        &self.0
    }
}

impl TryFrom<String> for HttpClientTimeout {
    type Error = ConfigError;
    fn try_from(value: String) -> Result<Self> {
        if value.is_empty() {
            return Ok(Self(std::time::Duration::from_secs(8)));
        }

        match value.parse::<u64>() {
            Ok(seconds) => Ok(Self(std::time::Duration::from_secs(seconds))),
            Err(_) => Err(ConfigError::InvalidTimeout {
                value: value.clone(),
            }),
        }
    }
}

impl AsRef<String> for IssuerDid {
    fn as_ref(&self) -> &String {
        &self.0
    }
}

impl IssuerDid {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for IssuerDid {
    type Error = ConfigError;
    fn try_from(value: String) -> Result<Self> {
        if value.is_empty() {
            return Err(ConfigError::InvalidDid {
                did: "(empty)".to_string(),
            });
        }
        if !value.starts_with("did:") {
            return Err(ConfigError::InvalidDid { did: value.clone() });
        }
        Ok(Self(value))
    }
}

impl TryFrom<String> for IssuerKey {
    type Error = ConfigError;
    fn try_from(value: String) -> Result<Self> {
        identify_key(&value)
            .map(IssuerKey)
            .map_err(|err| ConfigError::InvalidKeyFormat {
                details: format!("Failed to parse issuer key: {}", err),
            })
    }
}

impl AsRef<atproto_identity::key::KeyData> for IssuerKey {
    fn as_ref(&self) -> &atproto_identity::key::KeyData {
        &self.0
    }
}
