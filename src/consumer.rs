//! Jetstream consumer for processing AT Protocol events and evaluating blueprints.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use crate::errors::ConsumerError;
use anyhow::Result;
use async_trait::async_trait;
use atproto_jetstream::{EventHandler, JetstreamEvent};
use deadpool_redis::Pool;
use deadpool_redis::redis::AsyncCommands;
use tokio::sync::Mutex;
use tokio::sync::mpsc;

use crate::config::PartitionKeyStrategy;
use crate::denylist::DenyListManager;

/// Receiver for `BlueprintEvent` instances from the jetstream consumer.
pub type BlueprintEventReceiver = mpsc::Receiver<BlueprintEvent>;

/// ATProtocol events for blueprint evaluation.
#[derive(Debug, Clone)]
pub enum BlueprintEvent {
    /// A record was committed to the ATProtocol network.
    Commit {
        /// The DID of the record author
        did: String,
        /// The collection name
        collection: String,
        /// The record key
        rkey: String,
        /// The content identifier
        cid: String,
        /// The record data
        record: serde_json::Value,
        /// The operation type (create/update)
        operation: String,
        /// Timestamp in microseconds
        time_us: i64,
    },
    /// A record was deleted from the ATProtocol network.
    Delete {
        /// The DID of the record author
        did: String,
        /// The collection name
        collection: String,
        /// The record key
        rkey: String,
        /// Timestamp in microseconds
        time_us: i64,
    },
}

/// Configuration for event partitioning
#[derive(Debug, Clone)]
pub struct PartitionConfig {
    /// The instance ID (0-based)
    pub instance_id: usize,
    /// Total number of instances
    pub total_instances: usize,
    /// Partition key strategy
    pub strategy: PartitionKeyStrategy,
}

impl PartitionConfig {
    /// Create a new partition configuration
    pub fn new(instance_id: usize, total_instances: usize, strategy: PartitionKeyStrategy) -> Self {
        Self {
            instance_id,
            total_instances,
            strategy,
        }
    }

    /// Parse partition configuration from string format "index/total" (legacy support)
    pub fn parse(config: &str) -> Result<Self, ConsumerError> {
        if config.is_empty() || config == "0/1" || config == "1" {
            // Default to single partition
            return Ok(Self {
                instance_id: 0,
                total_instances: 1,
                strategy: PartitionKeyStrategy::DidBased,
            });
        }

        let parts: Vec<&str> = config.split('/').collect();
        if parts.len() != 2 {
            return Err(ConsumerError::InvalidPartition {
                details: format!(
                    "Invalid partition format. Expected 'index/total' (e.g., '0/3'), got: {}",
                    config
                ),
            });
        }

        let instance_id =
            parts[0]
                .parse::<usize>()
                .map_err(|_| ConsumerError::InvalidPartitionIndex {
                    value: parts[0].to_string(),
                })?;
        let total_instances =
            parts[1]
                .parse::<usize>()
                .map_err(|_| ConsumerError::InvalidPartitionTotal {
                    value: parts[1].to_string(),
                })?;

        if total_instances == 0 {
            return Err(ConsumerError::PartitionTotalZero);
        }
        if instance_id >= total_instances {
            return Err(ConsumerError::PartitionOutOfRange {
                index: instance_id as u32,
                total: total_instances as u32,
            });
        }

        Ok(Self {
            instance_id,
            total_instances,
            strategy: PartitionKeyStrategy::DidBased, // Default strategy for legacy format
        })
    }

    /// Check if an event should be processed by this instance based on partitioning strategy
    pub fn should_process_event(&self, event: &JetstreamEvent) -> bool {
        if self.total_instances <= 1 {
            // Single instance, process everything
            return true;
        }

        // Extract partition key based on strategy
        let partition_key = match (&self.strategy, event) {
            // DID-based partitioning
            (PartitionKeyStrategy::DidBased, JetstreamEvent::Commit { did, .. })
            | (PartitionKeyStrategy::DidBased, JetstreamEvent::Delete { did, .. })
            | (PartitionKeyStrategy::DidBased, JetstreamEvent::Identity { did, .. })
            | (PartitionKeyStrategy::DidBased, JetstreamEvent::Account { did, .. }) => did.clone(),

            // Collection-based partitioning
            (PartitionKeyStrategy::CollectionBased, JetstreamEvent::Commit { commit, .. }) => {
                commit.collection.clone()
            }
            (PartitionKeyStrategy::CollectionBased, JetstreamEvent::Delete { commit, .. }) => {
                commit.collection.clone()
            }
            (PartitionKeyStrategy::CollectionBased, _) => {
                // Identity and Account events don't have collections, use empty string
                String::new()
            }

            // Round-robin based on time_us
            (PartitionKeyStrategy::RoundRobin, event) => {
                let time_us = match event {
                    JetstreamEvent::Commit { time_us, .. }
                    | JetstreamEvent::Delete { time_us, .. }
                    | JetstreamEvent::Identity { time_us, .. }
                    | JetstreamEvent::Account { time_us, .. } => *time_us,
                };
                time_us.to_string()
            }

            // Custom field partitioning (would need additional logic)
            (PartitionKeyStrategy::CustomField(field), JetstreamEvent::Commit { commit, .. }) => {
                // Try to extract the custom field from the record
                commit
                    .record
                    .get(field)
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string()
            }
            (PartitionKeyStrategy::CustomField(_), _) => String::new(),
        };

        // Hash the partition key and determine target instance
        let hash = Self::hash_key(&partition_key);
        let target_instance = (hash as usize) % self.total_instances;
        target_instance == self.instance_id
    }

    /// Simple hash function for partition keys
    fn hash_key(key: &str) -> u64 {
        let mut hasher = DefaultHasher::new();
        key.hash(&mut hasher);
        hasher.finish()
    }
}

/// Handler for processing ATProtocol events and converting them to `BlueprintEvent` instances.
pub struct BlueprintEventHandler {
    id: String,
    event_sender: mpsc::Sender<BlueprintEvent>,
    /// Optional collection filters - if empty, accepts all collections
    collections: Vec<String>,
    /// Partition configuration for parallel processing
    partition: PartitionConfig,
    /// Metrics for partition distribution
    events_processed: Arc<AtomicU64>,
    events_skipped: Arc<AtomicU64>,
    /// Denylist manager for checking if DIDs should be blocked
    denylist: Arc<dyn DenyListManager>,
}

impl BlueprintEventHandler {
    pub fn new(
        id: String,
        event_sender: mpsc::Sender<BlueprintEvent>,
        collections: Vec<String>,
        denylist: Arc<dyn DenyListManager>,
    ) -> Self {
        Self::with_partition(
            id,
            event_sender,
            collections,
            PartitionConfig {
                instance_id: 0,
                total_instances: 1,
                strategy: PartitionKeyStrategy::DidBased,
            },
            denylist,
        )
    }

    pub fn with_partition(
        id: String,
        event_sender: mpsc::Sender<BlueprintEvent>,
        collections: Vec<String>,
        partition: PartitionConfig,
        denylist: Arc<dyn DenyListManager>,
    ) -> Self {
        tracing::info!(
            handler_id = %id,
            instance_id = partition.instance_id,
            total_instances = partition.total_instances,
            strategy = ?partition.strategy,
            "Created BlueprintEventHandler with partition configuration"
        );
        Self {
            id,
            event_sender,
            collections,
            partition,
            events_processed: Arc::new(AtomicU64::new(0)),
            events_skipped: Arc::new(AtomicU64::new(0)),
            denylist,
        }
    }

    /// Get partition distribution metrics
    pub fn get_metrics(&self) -> (u64, u64) {
        (
            self.events_processed.load(Ordering::Relaxed),
            self.events_skipped.load(Ordering::Relaxed),
        )
    }
}

#[async_trait]
impl EventHandler for BlueprintEventHandler {
    #[tracing::instrument(skip(self, event), fields(handler.id = %self.id))]
    async fn handle_event(&self, event: JetstreamEvent) -> anyhow::Result<()> {
        // Check if this instance should process this event based on partitioning strategy
        if !self.partition.should_process_event(&event) {
            self.events_skipped.fetch_add(1, Ordering::Relaxed);

            // Log partition metrics periodically
            let skipped = self.events_skipped.load(Ordering::Relaxed);
            if skipped % 10000 == 0 {
                let processed = self.events_processed.load(Ordering::Relaxed);
                let total = processed + skipped;
                let percentage = if total > 0 {
                    (processed as f64 / total as f64) * 100.0
                } else {
                    0.0
                };
                tracing::info!(
                    instance_id = self.partition.instance_id,
                    total_instances = self.partition.total_instances,
                    strategy = ?self.partition.strategy,
                    events_processed = processed,
                    events_skipped = skipped,
                    processing_percentage = percentage,
                    "Partition distribution metrics"
                );
            }

            tracing::trace!(
                instance_id = self.partition.instance_id,
                total_instances = self.partition.total_instances,
                strategy = ?self.partition.strategy,
                "Event filtered out by partition strategy - handled by different instance"
            );
            return Ok(());
        }

        // Increment processed counter
        self.events_processed.fetch_add(1, Ordering::Relaxed);

        // Check if the DID is in the denylist
        let event_did = match &event {
            JetstreamEvent::Commit { did, .. } => did,
            JetstreamEvent::Delete { did, .. } => did,
            JetstreamEvent::Identity { did, .. } => did,
            JetstreamEvent::Account { did, .. } => did,
        };

        if let Err(e) = self.denylist.exists(event_did).await {
            tracing::warn!(
                did = %event_did,
                error = %e,
                "Failed to check denylist for DID, allowing event to proceed"
            );
        } else if self.denylist.exists(event_did).await.unwrap_or(false) {
            tracing::debug!(
                did = %event_did,
                "DID is on denylist, skipping event"
            );
            return Ok(());
        }

        let blueprint_event = match event {
            JetstreamEvent::Commit {
                did,
                commit,
                time_us,
                ..
            } => {
                // Filter by collection if filters are configured
                if !self.collections.is_empty() && !self.collections.contains(&commit.collection) {
                    tracing::trace!(
                        did = %did,
                        collection = %commit.collection,
                        "Filtered out event - collection not in filter list"
                    );
                    return Ok(());
                }

                tracing::trace!(
                    did = %did,
                    collection = %commit.collection,
                    rkey = %commit.rkey,
                    operation = %commit.operation,
                    "Processing commit event"
                );

                BlueprintEvent::Commit {
                    did,
                    collection: commit.collection,
                    rkey: commit.rkey,
                    cid: commit.cid,
                    record: commit.record,
                    operation: commit.operation,
                    time_us: time_us as i64,
                }
            }
            JetstreamEvent::Delete {
                did,
                commit,
                time_us,
                ..
            } => {
                // Filter by collection if filters are configured
                if !self.collections.is_empty() && !self.collections.contains(&commit.collection) {
                    tracing::trace!(
                        did = %did,
                        collection = %commit.collection,
                        "Filtered out delete event - collection not in filter list"
                    );
                    return Ok(());
                }

                tracing::trace!(
                    did = %did,
                    collection = %commit.collection,
                    rkey = %commit.rkey,
                    "Processing delete event"
                );

                BlueprintEvent::Delete {
                    did,
                    collection: commit.collection,
                    rkey: commit.rkey,
                    time_us: time_us as i64,
                }
            }
            JetstreamEvent::Identity { .. } | JetstreamEvent::Account { .. } => {
                // We don't process identity or account events for blueprint evaluation
                return Ok(());
            }
        };

        if let Err(err) = self.event_sender.send(blueprint_event).await {
            tracing::error!(
                error = ?err,
                handler_id = %self.id,
                "Failed to send event to processor queue - channel may be full"
            );
        } else {
            tracing::trace!("Event successfully sent to processor queue");
        }

        Ok(())
    }

    fn handler_id(&self) -> String {
        self.id.clone()
    }
}

/// Cursor writer handler that periodically writes the latest time_us to a file
pub struct CursorWriterHandler {
    id: String,
    cursor_path: String,
    last_time_us: Arc<AtomicU64>,
    last_write: Arc<Mutex<Instant>>,
    write_interval: Duration,
}

impl CursorWriterHandler {
    pub fn new(id: String, cursor_path: String) -> Self {
        Self {
            id,
            cursor_path,
            last_time_us: Arc::new(AtomicU64::new(0)),
            last_write: Arc::new(Mutex::new(Instant::now())),
            write_interval: Duration::from_secs(30),
        }
    }

    async fn maybe_write_cursor(&self) -> anyhow::Result<()> {
        let current_time_us = self.last_time_us.load(Ordering::Relaxed);
        if current_time_us == 0 {
            return Ok(());
        }

        let mut last_write = self.last_write.lock().await;
        if last_write.elapsed() >= self.write_interval {
            tokio::fs::write(&self.cursor_path, current_time_us.to_string()).await?;
            *last_write = Instant::now();
        }
        Ok(())
    }
}

#[async_trait]
impl EventHandler for CursorWriterHandler {
    async fn handle_event(&self, event: JetstreamEvent) -> anyhow::Result<()> {
        // Extract time_us from any event type
        let time_us = match &event {
            JetstreamEvent::Commit { time_us, .. } => *time_us,
            JetstreamEvent::Delete { time_us, .. } => *time_us,
            JetstreamEvent::Identity { time_us, .. } => *time_us,
            JetstreamEvent::Account { time_us, .. } => *time_us,
        };

        // Update the latest time_us
        self.last_time_us.store(time_us, Ordering::Relaxed);

        // Try to write the cursor periodically
        if let Err(err) = self.maybe_write_cursor().await {
            tracing::warn!(error = ?err, "Failed to write cursor");
        }

        Ok(())
    }

    fn handler_id(&self) -> String {
        self.id.clone()
    }
}

/// Redis-based cursor handler that persists jetstream cursor position to Redis
pub struct RedisCursorHandler {
    id: String,
    redis_key: String,
    ttl_seconds: u64,
    last_time_us: Arc<AtomicU64>,
    last_write: Arc<Mutex<Instant>>,
    write_interval: Duration,
    redis_pool: Pool,
}

impl RedisCursorHandler {
    pub async fn new(
        id: String,
        redis_pool: Pool,
        redis_key: String,
        ttl_seconds: u64,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            id,
            redis_key,
            ttl_seconds,
            last_time_us: Arc::new(AtomicU64::new(0)),
            last_write: Arc::new(Mutex::new(Instant::now())),
            write_interval: Duration::from_secs(5), // Write every 5 seconds
            redis_pool,
        })
    }

    async fn maybe_write_cursor(&self) -> anyhow::Result<()> {
        let current_time_us = self.last_time_us.load(Ordering::Relaxed);
        if current_time_us == 0 {
            return Ok(());
        }

        let mut last_write = self.last_write.lock().await;
        if last_write.elapsed() >= self.write_interval {
            let mut conn = self.redis_pool.get().await?;

            // Set cursor with TTL
            let _: () = conn
                .set_ex(&self.redis_key, current_time_us, self.ttl_seconds)
                .await?;

            *last_write = Instant::now();
            tracing::debug!(
                cursor = current_time_us,
                redis_key = %self.redis_key,
                "Updated Redis cursor"
            );
        }
        Ok(())
    }

    /// Read cursor from Redis
    pub async fn read_cursor(redis_pool: &Pool, redis_key: &str) -> Option<i64> {
        match redis_pool.get().await {
            Ok(mut conn) => {
                match conn
                    .get::<String, Option<String>>(redis_key.to_string())
                    .await
                {
                    Ok(Some(value)) => value.parse::<i64>().ok(),
                    _ => None,
                }
            }
            Err(e) => {
                tracing::warn!(error = ?e, "Failed to get Redis connection for cursor read");
                None
            }
        }
    }
}

#[async_trait]
impl EventHandler for RedisCursorHandler {
    async fn handle_event(&self, event: JetstreamEvent) -> anyhow::Result<()> {
        // Extract time_us from any event type
        let time_us = match &event {
            JetstreamEvent::Commit { time_us, .. } => *time_us,
            JetstreamEvent::Delete { time_us, .. } => *time_us,
            JetstreamEvent::Identity { time_us, .. } => *time_us,
            JetstreamEvent::Account { time_us, .. } => *time_us,
        };

        // Update the latest time_us
        self.last_time_us.store(time_us, Ordering::Relaxed);

        // Try to write the cursor periodically
        if let Err(err) = self.maybe_write_cursor().await {
            tracing::warn!(error = ?err, "Failed to write cursor to Redis");
        }

        Ok(())
    }

    fn handler_id(&self) -> String {
        self.id.clone()
    }
}

/// Multi-consumer handler that distributes events across multiple channels
pub struct MultiConsumerBlueprintEventHandler {
    id: String,
    /// Multiple senders, one for each consumer
    event_senders: Vec<mpsc::Sender<BlueprintEvent>>,
    /// Strategy for distributing events (round-robin counter)
    next_consumer: Arc<AtomicU64>,
    /// Optional collection filters - if empty, accepts all collections
    collections: Vec<String>,
    /// Partition configuration for parallel processing
    partition: PartitionConfig,
    /// Metrics for partition distribution
    events_processed: Arc<AtomicU64>,
    events_skipped: Arc<AtomicU64>,
    /// Denylist manager for checking if DIDs should be blocked
    denylist: Arc<dyn DenyListManager>,
}

impl MultiConsumerBlueprintEventHandler {
    pub fn new(
        id: String,
        event_senders: Vec<mpsc::Sender<BlueprintEvent>>,
        collections: Vec<String>,
        denylist: Arc<dyn DenyListManager>,
    ) -> Self {
        Self::with_partition(
            id,
            event_senders,
            collections,
            PartitionConfig {
                instance_id: 0,
                total_instances: 1,
                strategy: PartitionKeyStrategy::DidBased,
            },
            denylist,
        )
    }

    pub fn with_partition(
        id: String,
        event_senders: Vec<mpsc::Sender<BlueprintEvent>>,
        collections: Vec<String>,
        partition: PartitionConfig,
        denylist: Arc<dyn DenyListManager>,
    ) -> Self {
        Self {
            id,
            event_senders,
            next_consumer: Arc::new(AtomicU64::new(0)),
            collections,
            partition,
            events_processed: Arc::new(AtomicU64::new(0)),
            events_skipped: Arc::new(AtomicU64::new(0)),
            denylist,
        }
    }

    /// Get partition distribution metrics
    pub fn get_metrics(&self) -> (u64, u64) {
        (
            self.events_processed.load(Ordering::Relaxed),
            self.events_skipped.load(Ordering::Relaxed),
        )
    }

    /// Get the next consumer index using round-robin distribution
    fn get_next_consumer_index(&self) -> usize {
        let num_consumers = self.event_senders.len();
        if num_consumers == 0 {
            return 0;
        }
        let index = self.next_consumer.fetch_add(1, Ordering::Relaxed);
        (index % num_consumers as u64) as usize
    }
}

#[async_trait]
impl EventHandler for MultiConsumerBlueprintEventHandler {
    #[tracing::instrument(skip(self, event), fields(handler.id = %self.id))]
    async fn handle_event(&self, event: JetstreamEvent) -> anyhow::Result<()> {
        // Extract time_us for partition filtering
        let _event_time_us = match &event {
            JetstreamEvent::Commit { time_us, .. } => *time_us,
            JetstreamEvent::Delete { time_us, .. } => *time_us,
            JetstreamEvent::Identity { time_us, .. } => *time_us,
            JetstreamEvent::Account { time_us, .. } => *time_us,
        };

        // Check if this instance should process this event based on partitioning strategy
        if !self.partition.should_process_event(&event) {
            self.events_skipped.fetch_add(1, Ordering::Relaxed);

            // Log partition metrics periodically
            let skipped = self.events_skipped.load(Ordering::Relaxed);
            if skipped % 10000 == 0 {
                let processed = self.events_processed.load(Ordering::Relaxed);
                let total = processed + skipped;
                let percentage = if total > 0 {
                    (processed as f64 / total as f64) * 100.0
                } else {
                    0.0
                };
                tracing::info!(
                    handler_id = %self.id,
                    instance_id = self.partition.instance_id,
                    total_instances = self.partition.total_instances,
                    strategy = ?self.partition.strategy,
                    events_processed = processed,
                    events_skipped = skipped,
                    processing_percentage = percentage,
                    "Multi-consumer partition distribution metrics"
                );
            }

            tracing::trace!(
                instance_id = self.partition.instance_id,
                total_instances = self.partition.total_instances,
                strategy = ?self.partition.strategy,
                "Event filtered out by partition - handled by different instance"
            );
            return Ok(());
        }

        // Increment processed counter
        self.events_processed.fetch_add(1, Ordering::Relaxed);

        // Check if the DID is in the denylist
        let event_did = match &event {
            JetstreamEvent::Commit { did, .. } => did,
            JetstreamEvent::Delete { did, .. } => did,
            JetstreamEvent::Identity { did, .. } => did,
            JetstreamEvent::Account { did, .. } => did,
        };

        if let Err(e) = self.denylist.exists(event_did).await {
            tracing::warn!(
                did = %event_did,
                error = %e,
                "Failed to check denylist for DID, allowing event to proceed"
            );
        } else if self.denylist.exists(event_did).await.unwrap_or(false) {
            tracing::debug!(
                did = %event_did,
                "DID is on denylist, skipping event"
            );
            return Ok(());
        }

        let blueprint_event = match event {
            JetstreamEvent::Commit {
                did,
                commit,
                time_us,
                ..
            } => {
                // Filter by collection if filters are configured
                if !self.collections.is_empty() && !self.collections.contains(&commit.collection) {
                    tracing::trace!(
                        did = %did,
                        collection = %commit.collection,
                        "Filtered out event - collection not in filter list"
                    );
                    return Ok(());
                }

                tracing::trace!(
                    did = %did,
                    collection = %commit.collection,
                    rkey = %commit.rkey,
                    operation = %commit.operation,
                    "Processing commit event"
                );

                BlueprintEvent::Commit {
                    did,
                    collection: commit.collection,
                    rkey: commit.rkey,
                    cid: commit.cid,
                    record: commit.record,
                    operation: commit.operation,
                    time_us: time_us as i64,
                }
            }
            JetstreamEvent::Delete {
                did,
                commit,
                time_us,
                ..
            } => {
                // Filter by collection if filters are configured
                if !self.collections.is_empty() && !self.collections.contains(&commit.collection) {
                    tracing::trace!(
                        did = %did,
                        collection = %commit.collection,
                        "Filtered out delete event - collection not in filter list"
                    );
                    return Ok(());
                }

                tracing::trace!(
                    did = %did,
                    collection = %commit.collection,
                    rkey = %commit.rkey,
                    "Processing delete event"
                );

                BlueprintEvent::Delete {
                    did,
                    collection: commit.collection,
                    rkey: commit.rkey,
                    time_us: time_us as i64,
                }
            }
            JetstreamEvent::Identity { .. } | JetstreamEvent::Account { .. } => {
                // We don't process identity or account events for blueprint evaluation
                return Ok(());
            }
        };

        // Distribute to next consumer using round-robin
        let consumer_index = self.get_next_consumer_index();

        if consumer_index < self.event_senders.len() {
            if let Err(err) = self.event_senders[consumer_index]
                .send(blueprint_event)
                .await
            {
                tracing::error!(
                    error = ?err,
                    handler_id = %self.id,
                    consumer_index = consumer_index,
                    "Failed to send event to consumer {} - channel may be full",
                    consumer_index
                );
            } else {
                tracing::trace!(
                    consumer_index = consumer_index,
                    "Event successfully sent to consumer {}",
                    consumer_index
                );
            }
        }

        Ok(())
    }

    fn handler_id(&self) -> String {
        self.id.clone()
    }
}

/// Consumer for creating ATProtocol event handlers.
pub struct Consumer {}

impl Consumer {
    /// Create a new blueprint event handler that processes ATProtocol events.
    pub fn create_blueprint_handler(
        &self,
        collections: Vec<String>,
        denylist: Arc<dyn DenyListManager>,
    ) -> (Arc<BlueprintEventHandler>, BlueprintEventReceiver) {
        let (sender, receiver) = mpsc::channel(500);  // Reduced from 1000 to limit memory usage
        let handler = Arc::new(BlueprintEventHandler::new(
            "blueprint-processor".to_string(),
            sender,
            collections,
            denylist,
        ));
        (handler, receiver)
    }

    /// Create a new blueprint event handler with partition configuration for parallel processing.
    pub fn create_blueprint_handler_with_partition(
        &self,
        collections: Vec<String>,
        partition_config: &str,
        denylist: Arc<dyn DenyListManager>,
    ) -> Result<(Arc<BlueprintEventHandler>, BlueprintEventReceiver), ConsumerError> {
        let partition = PartitionConfig::parse(partition_config)?;
        let (sender, receiver) = mpsc::channel(500);  // Reduced from 1000 to limit memory usage
        let handler = Arc::new(BlueprintEventHandler::with_partition(
            "blueprint-processor".to_string(),
            sender,
            collections,
            partition.clone(),
            denylist,
        ));

        tracing::info!(
            instance_id = partition.instance_id,
            total_instances = partition.total_instances,
            strategy = ?partition.strategy,
            "Created blueprint handler with partition configuration"
        );

        Ok((handler, receiver))
    }

    /// Create a new blueprint event handler with explicit partition configuration.
    pub fn create_blueprint_handler_with_partition_config(
        &self,
        collections: Vec<String>,
        partition: PartitionConfig,
        denylist: Arc<dyn DenyListManager>,
    ) -> Result<(Arc<BlueprintEventHandler>, BlueprintEventReceiver), ConsumerError> {
        let (sender, receiver) = mpsc::channel(500);  // Reduced from 1000 to limit memory usage
        let handler = Arc::new(BlueprintEventHandler::with_partition(
            "blueprint-processor".to_string(),
            sender,
            collections,
            partition.clone(),
            denylist,
        ));

        tracing::info!(
            instance_id = partition.instance_id,
            total_instances = partition.total_instances,
            strategy = ?partition.strategy,
            "Created blueprint handler with explicit partition configuration"
        );

        Ok((handler, receiver))
    }

    /// Create a new Redis-based cursor handler that persists jetstream cursor position to Redis.
    pub async fn create_redis_cursor_handler(
        &self,
        redis_pool: Pool,
        redis_key: String,
        ttl_seconds: u64,
    ) -> anyhow::Result<Arc<RedisCursorHandler>> {
        let handler = RedisCursorHandler::new(
            "redis-cursor-writer".to_string(),
            redis_pool,
            redis_key,
            ttl_seconds,
        )
        .await?;
        Ok(Arc::new(handler))
    }

    /// Read the last cursor from file if it exists
    pub async fn read_cursor(cursor_path: &str) -> Option<i64> {
        match tokio::fs::read_to_string(cursor_path).await {
            Ok(content) => content.trim().parse::<i64>().ok(),
            Err(_) => None,
        }
    }

    /// Read the last cursor from Redis if it exists
    pub async fn read_cursor_from_redis(redis_pool: &Pool, redis_key: &str) -> Option<i64> {
        RedisCursorHandler::read_cursor(redis_pool, redis_key).await
    }

    /// Create a multi-consumer blueprint event handler with multiple receivers
    pub fn create_multi_consumer_handler(
        &self,
        collections: Vec<String>,
        consumer_count: usize,
        denylist: Arc<dyn DenyListManager>,
    ) -> (
        Arc<MultiConsumerBlueprintEventHandler>,
        Vec<BlueprintEventReceiver>,
    ) {
        let consumer_count = consumer_count.max(1); // Ensure at least 1 consumer

        let mut senders = Vec::with_capacity(consumer_count);
        let mut receivers = Vec::with_capacity(consumer_count);

        for _ in 0..consumer_count {
            let (sender, receiver) = mpsc::channel(500);  // Reduced from 1000 to limit memory usage
            senders.push(sender);
            receivers.push(receiver);
        }

        let handler = Arc::new(MultiConsumerBlueprintEventHandler::new(
            "multi-blueprint-processor".to_string(),
            senders,
            collections,
            denylist,
        ));

        tracing::info!(
            consumer_count = consumer_count,
            "Created multi-consumer handler with {} consumers",
            consumer_count
        );

        (handler, receivers)
    }

    /// Create a multi-consumer blueprint event handler with partition configuration
    pub fn create_multi_consumer_handler_with_partition(
        &self,
        collections: Vec<String>,
        consumer_count: usize,
        partition_config: &str,
        denylist: Arc<dyn DenyListManager>,
    ) -> Result<
        (
            Arc<MultiConsumerBlueprintEventHandler>,
            Vec<BlueprintEventReceiver>,
        ),
        ConsumerError,
    > {
        let partition = PartitionConfig::parse(partition_config)?;
        let consumer_count = consumer_count.max(1); // Ensure at least 1 consumer

        let mut senders = Vec::with_capacity(consumer_count);
        let mut receivers = Vec::with_capacity(consumer_count);

        for _ in 0..consumer_count {
            let (sender, receiver) = mpsc::channel(500);  // Reduced from 1000 to limit memory usage
            senders.push(sender);
            receivers.push(receiver);
        }

        let handler = Arc::new(MultiConsumerBlueprintEventHandler::with_partition(
            "multi-blueprint-processor".to_string(),
            senders,
            collections,
            partition.clone(),
            denylist,
        ));

        tracing::info!(
            consumer_count = consumer_count,
            instance_id = partition.instance_id,
            total_instances = partition.total_instances,
            strategy = ?partition.strategy,
            "Created multi-consumer handler with {} consumers and partition configuration",
            consumer_count
        );

        Ok((handler, receivers))
    }

    /// Create a multi-consumer blueprint event handler with explicit partition configuration
    pub fn create_multi_consumer_handler_with_partition_config(
        &self,
        collections: Vec<String>,
        consumer_count: usize,
        partition: PartitionConfig,
        denylist: Arc<dyn DenyListManager>,
    ) -> Result<
        (
            Arc<MultiConsumerBlueprintEventHandler>,
            Vec<BlueprintEventReceiver>,
        ),
        ConsumerError,
    > {
        let consumer_count = consumer_count.max(1); // Ensure at least 1 consumer

        let mut senders = Vec::with_capacity(consumer_count);
        let mut receivers = Vec::with_capacity(consumer_count);

        for _ in 0..consumer_count {
            let (sender, receiver) = mpsc::channel(500);  // Reduced from 1000 to limit memory usage
            senders.push(sender);
            receivers.push(receiver);
        }

        let handler = Arc::new(MultiConsumerBlueprintEventHandler::with_partition(
            "multi-blueprint-processor".to_string(),
            senders,
            collections,
            partition.clone(),
            denylist,
        ));

        tracing::info!(
            consumer_count = consumer_count,
            instance_id = partition.instance_id,
            total_instances = partition.total_instances,
            strategy = ?partition.strategy,
            "Created multi-consumer handler with {} consumers with explicit partition configuration",
            consumer_count
        );

        Ok((handler, receivers))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_partition_config_parse() {
        // Test default single partition
        let config = PartitionConfig::parse("0/1").unwrap();
        assert_eq!(config.instance_id, 0);
        assert_eq!(config.total_instances, 1);
        assert!(matches!(config.strategy, PartitionKeyStrategy::DidBased));

        // Test empty string defaults to single partition
        let config = PartitionConfig::parse("").unwrap();
        assert_eq!(config.instance_id, 0);
        assert_eq!(config.total_instances, 1);
        assert!(matches!(config.strategy, PartitionKeyStrategy::DidBased));

        // Test simple "1" defaults to single partition
        let config = PartitionConfig::parse("1").unwrap();
        assert_eq!(config.instance_id, 0);
        assert_eq!(config.total_instances, 1);
        assert!(matches!(config.strategy, PartitionKeyStrategy::DidBased));

        // Test multiple partitions
        let config = PartitionConfig::parse("0/3").unwrap();
        assert_eq!(config.instance_id, 0);
        assert_eq!(config.total_instances, 3);

        let config = PartitionConfig::parse("2/5").unwrap();
        assert_eq!(config.instance_id, 2);
        assert_eq!(config.total_instances, 5);

        // Test error cases
        assert!(PartitionConfig::parse("3/3").is_err()); // index >= total
        assert!(PartitionConfig::parse("1/0").is_err()); // total is 0
        assert!(PartitionConfig::parse("invalid").is_err()); // invalid format
        assert!(PartitionConfig::parse("a/b").is_err()); // non-numeric
    }

    #[test]
    fn test_partition_should_process() {
        use atproto_jetstream::{JetstreamEvent, JetstreamEventCommit};
        use serde_json::json;

        // Single partition should process everything
        let config = PartitionConfig {
            instance_id: 0,
            total_instances: 1,
            strategy: PartitionKeyStrategy::DidBased,
        };

        // Create a test event
        let event = JetstreamEvent::Commit {
            did: "did:plc:test123".to_string(),
            time_us: 1000,
            kind: "commit".to_string(),
            commit: JetstreamEventCommit {
                rev: "rev123".to_string(),
                operation: "create".to_string(),
                collection: "app.bsky.feed.post".to_string(),
                rkey: "rkey123".to_string(),
                record: json!({}),
                cid: "cid123".to_string(),
            },
        };

        assert!(config.should_process_event(&event));

        // Test 3-partition setup
        let partition0 = PartitionConfig {
            instance_id: 0,
            total_instances: 3,
            strategy: PartitionKeyStrategy::DidBased,
        };
        let partition1 = PartitionConfig {
            instance_id: 1,
            total_instances: 3,
            strategy: PartitionKeyStrategy::DidBased,
        };
        let partition2 = PartitionConfig {
            instance_id: 2,
            total_instances: 3,
            strategy: PartitionKeyStrategy::DidBased,
        };

        // Events should be distributed based on DID hash - exactly one partition should process it
        let processed_count = [
            partition0.should_process_event(&event) as u32,
            partition1.should_process_event(&event) as u32,
            partition2.should_process_event(&event) as u32,
        ]
        .iter()
        .sum::<u32>();

        assert_eq!(
            processed_count, 1,
            "Exactly one partition should process each event"
        );

        // Test with different DIDs to ensure distribution
        let events_with_dids = vec![
            "did:plc:test1",
            "did:plc:test2",
            "did:plc:test3",
            "did:plc:test4",
            "did:plc:test5",
            "did:plc:test6",
        ];

        for did in events_with_dids {
            let test_event = JetstreamEvent::Commit {
                did: did.to_string(),
                time_us: 1000,
                kind: "commit".to_string(),
                commit: JetstreamEventCommit {
                    rev: "rev123".to_string(),
                    operation: "create".to_string(),
                    collection: "app.bsky.feed.post".to_string(),
                    rkey: "rkey123".to_string(),
                    record: json!({}),
                    cid: "cid123".to_string(),
                },
            };

            let count = [
                partition0.should_process_event(&test_event) as u32,
                partition1.should_process_event(&test_event) as u32,
                partition2.should_process_event(&test_event) as u32,
            ]
            .iter()
            .sum::<u32>();

            assert_eq!(
                count, 1,
                "Exactly one partition should process event for DID: {}",
                did
            );
        }
    }

    #[tokio::test]
    async fn test_event_handler_with_partition() {
        use crate::denylist::noop::NoopDenyListManager;
        use atproto_jetstream::JetstreamEventCommit;

        // Create a handler for partition 1 of 3
        let partition = PartitionConfig {
            instance_id: 1,
            total_instances: 3,
            strategy: PartitionKeyStrategy::RoundRobin,
        };
        let (sender, mut receiver) = mpsc::channel(500);
        let denylist = Arc::new(NoopDenyListManager::new());
        let handler = BlueprintEventHandler::with_partition(
            "test-handler".to_string(),
            sender,
            vec![], // No collection filters
            partition,
            denylist,
        );

        // Create events with different time_us values
        let commit = JetstreamEventCommit {
            cid: "test-cid".to_string(),
            rev: "test-rev".to_string(),
            operation: "create".to_string(),
            collection: "test.collection".to_string(),
            rkey: "test-rkey".to_string(),
            record: serde_json::json!({"test": "data"}),
        };

        // For RoundRobin strategy with instance_id=1, should process events where time_us % 3 == 1
        let events = vec![
            ("did:plc:test1", 1), // 1 % 3 = 1 - should be processed by instance 1
            ("did:plc:test2", 4), // 4 % 3 = 1 - should be processed by instance 1
            ("did:plc:test3", 2), // 2 % 3 = 2 - should NOT be processed by instance 1
            ("did:plc:test4", 7), // 7 % 3 = 1 - should be processed by instance 1
            ("did:plc:test5", 0), // 0 % 3 = 0 - should NOT be processed by instance 1
        ];

        let mut expected_processed = 0;
        let mut events_sent = vec![];

        for (did, time_us_val) in events {
            let event = JetstreamEvent::Commit {
                did: did.to_string(),
                commit: commit.clone(),
                time_us: time_us_val,
                kind: "commit".to_string(),
            };

            // Test if this event should be processed by this partition
            if handler.partition.should_process_event(&event) {
                expected_processed += 1;
            }

            events_sent.push((did, time_us_val));
            handler.handle_event(event).await.unwrap();
        }

        // Verify we received the expected number of events
        let mut actual_count = 0;
        while let Ok(_) = receiver.try_recv() {
            actual_count += 1;
        }

        assert_eq!(
            actual_count, expected_processed,
            "Should have received {} events based on partition logic, got {}",
            expected_processed, actual_count
        );
    }

    #[tokio::test]
    async fn test_multi_consumer_event_distribution() {
        use crate::denylist::noop::NoopDenyListManager;
        use atproto_jetstream::JetstreamEventCommit;

        // Create multiple consumers
        let consumer = Consumer {};
        let denylist = Arc::new(NoopDenyListManager::new());
        let (handler, mut receivers) = consumer.create_multi_consumer_handler(
            vec![], // No collection filters
            3,      // 3 consumers
            denylist,
        );

        // Create test events
        let commit = JetstreamEventCommit {
            cid: "test-cid".to_string(),
            rev: "test-rev".to_string(),
            operation: "create".to_string(),
            collection: "test.collection".to_string(),
            rkey: "test-rkey".to_string(),
            record: serde_json::json!({"test": "data"}),
        };

        // Send 6 events (should be distributed round-robin)
        for i in 0..6 {
            let event = JetstreamEvent::Commit {
                did: format!("did:plc:test{}", i),
                commit: commit.clone(),
                time_us: i as u64,
                kind: "commit".to_string(),
            };
            handler.handle_event(event).await.unwrap();
        }

        // Check that each consumer received 2 events
        for (consumer_idx, receiver) in receivers.iter_mut().enumerate() {
            let mut event_count = 0;
            while let Ok(_) = receiver.try_recv() {
                event_count += 1;
            }
            assert_eq!(
                event_count, 2,
                "Consumer {} should have received 2 events",
                consumer_idx
            );
        }
    }

    #[test]
    fn test_multi_consumer_round_robin_distribution() {
        // Create handler with 3 senders
        let (sender1, _) = mpsc::channel(1);
        let (sender2, _) = mpsc::channel(1);
        let (sender3, _) = mpsc::channel(1);
        let denylist = Arc::new(crate::denylist::noop::NoopDenyListManager::new());

        let handler = MultiConsumerBlueprintEventHandler::new(
            "test".to_string(),
            vec![sender1, sender2, sender3],
            vec![],
            denylist,
        );

        // Test round-robin distribution
        assert_eq!(handler.get_next_consumer_index(), 0);
        assert_eq!(handler.get_next_consumer_index(), 1);
        assert_eq!(handler.get_next_consumer_index(), 2);
        assert_eq!(handler.get_next_consumer_index(), 0); // Back to 0
        assert_eq!(handler.get_next_consumer_index(), 1);
        assert_eq!(handler.get_next_consumer_index(), 2);
    }

    #[tokio::test]
    async fn test_denylist_blocks_events() {
        use crate::denylist::DenyListManager;
        use atproto_jetstream::{JetstreamEvent, JetstreamEventCommit};

        // Create a mock denylist that denies a specific DID
        struct TestDenyList {
            denied_did: String,
        }

        #[async_trait]
        impl DenyListManager for TestDenyList {
            async fn exists(&self, member: &str) -> anyhow::Result<bool> {
                Ok(member == self.denied_did)
            }
        }

        let denied_did = "did:plc:blocked123";
        let allowed_did = "did:plc:allowed456";

        let denylist = Arc::new(TestDenyList {
            denied_did: denied_did.to_string(),
        });

        let (sender, mut receiver) = mpsc::channel(100);
        let handler = BlueprintEventHandler::new(
            "test-handler".to_string(),
            sender,
            vec![], // No collection filters
            denylist,
        );

        // Create test events
        let denied_event = JetstreamEvent::Commit {
            kind: "commit".to_string(),
            did: denied_did.to_string(),
            time_us: 1000,
            commit: JetstreamEventCommit {
                cid: "test-cid-1".to_string(),
                rev: "test-rev-1".to_string(),
                operation: "create".to_string(),
                collection: "app.bsky.feed.post".to_string(),
                rkey: "test-rkey-1".to_string(),
                record: serde_json::json!({"text": "blocked post"}),
            },
        };

        let allowed_event = JetstreamEvent::Commit {
            kind: "commit".to_string(),
            did: allowed_did.to_string(),
            time_us: 2000,
            commit: JetstreamEventCommit {
                cid: "test-cid-2".to_string(),
                rev: "test-rev-2".to_string(),
                operation: "create".to_string(),
                collection: "app.bsky.feed.post".to_string(),
                rkey: "test-rkey-2".to_string(),
                record: serde_json::json!({"text": "allowed post"}),
            },
        };

        // Handle the denied event - should be blocked
        handler.handle_event(denied_event).await.unwrap();

        // Handle the allowed event - should pass through
        handler.handle_event(allowed_event).await.unwrap();

        // Only one event should have made it through the denylist
        assert!(
            receiver.try_recv().is_ok(),
            "Allowed event should be received"
        );
        assert!(
            receiver.try_recv().is_err(),
            "No more events should be received (denied event was blocked)"
        );
    }
}
