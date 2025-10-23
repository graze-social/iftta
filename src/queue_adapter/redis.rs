//! Redis-based queue adapter for distributed, persistent work queuing.
//!
//! This module provides a reliable queue implementation using Redis with
//! the RPOPLPUSH pattern for at-least-once delivery guarantees. It's ideal
//! for multi-instance deployments requiring work distribution and persistence.
//!
//! # Characteristics
//!
//! - **Distributed**: Multiple workers can process from same queue
//! - **Persistent**: Data survives process restarts
//! - **Reliable**: At-least-once delivery with acknowledgment
//! - **Recoverable**: Failed work can be recovered from worker queues
//! - **Scalable**: Supports horizontal scaling of workers
//!
//! # Architecture
//!
//! The adapter uses two Redis lists per worker:
//!
//! 1. **Primary Queue** (`queue:prefix:primary`): Shared queue for all workers
//! 2. **Worker Queue** (`queue:prefix:worker-id`): Temporary queue for in-progress items
//!
//! Items flow: Primary Queue → Worker Queue → Acknowledged (removed)
//!
//! # When to Use
//!
//! Use `RedisQueueAdapter` when:
//! - Running multiple instances of the service
//! - Work needs to be distributed across workers
//! - Data persistence is required
//! - At-least-once delivery is needed
//! - Queue must survive process restarts
//!
//! # Reliability Pattern
//!
//! ```text
//! 1. RPOPLPUSH atomically moves item from primary to worker queue
//! 2. Worker processes the item
//! 3. On success: LREM removes item from worker queue
//! 4. On failure: Item remains in worker queue for recovery
//! 5. On startup: Worker can recover its queue back to primary
//! ```

use anyhow::Result;
use async_trait::async_trait;
use deadpool_redis::{Pool, redis::AsyncCommands};
use serde::{Deserialize, Serialize};
use std::marker::PhantomData;
use tracing::{debug, error, trace, warn};

use super::QueueAdapter;
use crate::errors::QueueError;

/// Redis-based queue adapter using reliable queue pattern.
///
/// This adapter implements a reliable message queue using Redis lists
/// with atomic operations for work distribution and failure recovery.
///
/// # Type Parameters
///
/// * `T` - The work item type. Must implement `Serialize` and `Deserialize`.
///
/// # Example
///
/// ```rust,ignore
/// use ifthisthenat::queue_adapter::RedisQueueAdapter;
/// use deadpool_redis::Config;
///
/// #[derive(Serialize, Deserialize)]
/// struct WorkItem {
///     id: String,
///     data: String,
/// }
///
/// #[tokio::main]
/// async fn main() -> Result<()> {
///     // Create Redis pool
///     let cfg = Config::from_url("redis://localhost");
///     let pool = cfg.create_pool(Some(Runtime::Tokio1))?;
///     
///     // Create queue adapter
///     let queue = RedisQueueAdapter::<WorkItem>::new(
///         pool,
///         Some("worker-1".to_string()),
///         Some("myapp:queue:".to_string()),
///     );
///     
///     // Recover any previous work on startup
///     queue.recover_worker_queue().await?;
///     
///     // Process work
///     while let Some(work) = queue.pull().await {
///         match process(work.clone()).await {
///             Ok(_) => queue.ack(&work).await?,
///             Err(e) => {
///                 // Work remains in worker queue for later recovery
///                 error!("Failed to process: {}", e);
///             }
///         }
///     }
///     
///     Ok(())
/// }
/// ```
pub struct RedisQueueAdapter<T>
where
    T: Send + Sync + Serialize + for<'de> Deserialize<'de> + 'static,
{
    /// Redis connection pool
    pool: Pool,
    /// Unique identifier for this worker
    worker_id: String,
    /// Queue name prefix
    prefix: String,
    /// Full name of the primary queue
    primary_queue_name: String,
    /// Full name of this worker's queue
    worker_queue_name: String,
    /// Phantom data for type parameter
    _phantom: PhantomData<T>,
}

impl<T> RedisQueueAdapter<T>
where
    T: Send + Sync + Serialize + for<'de> Deserialize<'de> + 'static,
{
    /// Create a new Redis queue adapter.
    ///
    /// # Arguments
    ///
    /// * `pool` - Redis connection pool
    /// * `worker_id` - Unique worker identifier (auto-generated if None)
    /// * `prefix` - Queue name prefix (defaults to "queue:work:")
    ///
    /// # Queue Names
    ///
    /// - Primary queue: `{prefix}primary`
    /// - Worker queue: `{prefix}{worker_id}`
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// // Auto-generate worker ID
    /// let queue = RedisQueueAdapter::<Work>::new(pool, None, None);
    ///
    /// // Specific worker ID for debugging
    /// let queue = RedisQueueAdapter::<Work>::new(
    ///     pool,
    ///     Some("worker-1".to_string()),
    ///     Some("myapp:queue:".to_string()),
    /// );
    /// ```
    pub fn new(pool: Pool, worker_id: Option<String>, prefix: Option<String>) -> Self {
        let worker_id = worker_id.unwrap_or_else(|| {
            // Generate random worker ID using UUID
            uuid::Uuid::new_v4().to_string()
        });

        let prefix = prefix.unwrap_or_else(|| "queue:work:".to_string());
        let primary_queue_name = format!("{}primary", prefix);
        let worker_queue_name = format!("{}{}", prefix, worker_id);

        debug!(
            worker_id = %worker_id,
            primary_queue = %primary_queue_name,
            worker_queue = %worker_queue_name,
            "Initializing Redis queue adapter"
        );

        Self {
            pool,
            worker_id,
            prefix,
            primary_queue_name,
            worker_queue_name,
            _phantom: PhantomData,
        }
    }

    /// Get the worker ID for this adapter.
    pub fn worker_id(&self) -> &str {
        &self.worker_id
    }

    /// Get the queue name prefix.
    pub fn prefix(&self) -> &str {
        &self.prefix
    }

    /// Get the primary queue name.
    pub fn primary_queue_name(&self) -> &str {
        &self.primary_queue_name
    }

    /// Get the worker queue name.
    pub fn worker_queue_name(&self) -> &str {
        &self.worker_queue_name
    }

    /// Recover items from worker queue back to primary queue.
    ///
    /// Call this on startup to recover any items that were being
    /// processed when the worker stopped. Items are moved back to
    /// the primary queue for reprocessing.
    ///
    /// # Returns
    ///
    /// Number of items recovered.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// // On startup, recover any previous work
    /// let recovered = queue.recover_worker_queue().await?;
    /// if recovered > 0 {
    ///     info!("Recovered {} items from previous run", recovered);
    /// }
    /// ```
    pub async fn recover_worker_queue(&self) -> Result<usize> {
        let mut conn = self
            .pool
            .get()
            .await
            .map_err(|e| QueueError::ConnectionFailed {
                queue_type: "redis".to_string(),
                details: e.to_string(),
            })?;

        let mut recovered = 0;
        loop {
            // Move items from worker queue back to primary queue
            let item: Option<String> = conn
                .rpoplpush(&self.worker_queue_name, &self.primary_queue_name)
                .await
                .map_err(|e| QueueError::RedisOperationFailed {
                    operation: "rpoplpush (recovery)".to_string(),
                    source: e,
                })?;

            if item.is_none() {
                break;
            }
            recovered += 1;
        }

        if recovered > 0 {
            debug!(
                worker_id = %self.worker_id,
                count = recovered,
                "Recovered items from worker queue to primary queue"
            );
        }

        Ok(recovered)
    }

    /// Get the number of items in the worker's temporary queue.
    ///
    /// Useful for monitoring how many items are currently being processed
    /// by this worker.
    ///
    /// # Returns
    ///
    /// Number of items in the worker queue.
    pub async fn worker_queue_depth(&self) -> Result<usize> {
        let mut conn = self
            .pool
            .get()
            .await
            .map_err(|e| QueueError::ConnectionFailed {
                queue_type: "redis".to_string(),
                details: e.to_string(),
            })?;

        let depth: usize = conn.llen(&self.worker_queue_name).await.map_err(|e| {
            QueueError::RedisOperationFailed {
                operation: "llen (worker queue depth)".to_string(),
                source: e,
            }
        })?;

        Ok(depth)
    }

    /// Clear all items from the worker queue.
    ///
    /// WARNING: This will lose any in-progress work. Only use for cleanup
    /// in development or when you're certain the items are invalid.
    pub async fn clear_worker_queue(&self) -> Result<()> {
        let mut conn = self
            .pool
            .get()
            .await
            .map_err(|e| QueueError::ConnectionFailed {
                queue_type: "redis".to_string(),
                details: e.to_string(),
            })?;

        conn.del::<_, ()>(&self.worker_queue_name)
            .await
            .map_err(|e| QueueError::RedisOperationFailed {
                operation: "del (clear worker queue)".to_string(),
                source: e,
            })?;

        warn!(
            worker_id = %self.worker_id,
            "Cleared worker queue"
        );

        Ok(())
    }

    /// Get statistics about the queue system.
    ///
    /// Returns counts for primary queue, worker queue, and total items.
    pub async fn stats(&self) -> Result<QueueStats> {
        let mut conn = self
            .pool
            .get()
            .await
            .map_err(|e| QueueError::ConnectionFailed {
                queue_type: "redis".to_string(),
                details: e.to_string(),
            })?;

        let primary_depth: usize = conn.llen(&self.primary_queue_name).await.unwrap_or(0);

        let worker_depth: usize = conn.llen(&self.worker_queue_name).await.unwrap_or(0);

        Ok(QueueStats {
            primary_queue_depth: primary_depth,
            worker_queue_depth: worker_depth,
            total_items: primary_depth + worker_depth,
            worker_id: self.worker_id.clone(),
        })
    }
}

#[async_trait]
impl<T> QueueAdapter<T> for RedisQueueAdapter<T>
where
    T: Send + Sync + Serialize + for<'de> Deserialize<'de> + 'static,
{
    async fn pull(&self) -> Option<T> {
        match self.pool.get().await {
            Ok(mut conn) => {
                // Atomically move item from primary queue to worker queue
                let result: Result<Option<String>, _> = conn
                    .rpoplpush(&self.primary_queue_name, &self.worker_queue_name)
                    .await;

                match result {
                    Ok(Some(data)) => {
                        // Deserialize the item
                        match serde_json::from_str(&data) {
                            Ok(item) => {
                                trace!(
                                    worker_id = %self.worker_id,
                                    "Pulled item from Redis queue"
                                );
                                Some(item)
                            }
                            Err(e) => {
                                error!(
                                    error = ?e,
                                    data = %data,
                                    "Failed to deserialize item from queue"
                                );
                                // Item is malformed, remove it from worker queue
                                let _ = conn
                                    .lrem::<_, _, ()>(&self.worker_queue_name, 1, &data)
                                    .await;
                                None
                            }
                        }
                    }
                    Ok(None) => {
                        // Queue is empty
                        None
                    }
                    Err(e) => {
                        error!(
                            error = ?e,
                            "Failed to pull item from Redis queue"
                        );
                        None
                    }
                }
            }
            Err(e) => {
                error!(
                    error = ?e,
                    "Failed to get Redis connection"
                );
                None
            }
        }
    }

    async fn push(&self, work: T) -> Result<()> {
        let mut conn = self
            .pool
            .get()
            .await
            .map_err(|e| QueueError::ConnectionFailed {
                queue_type: "redis".to_string(),
                details: e.to_string(),
            })?;

        let serialized = serde_json::to_string(&work)?;

        // Push to the left of primary queue (FIFO when using RPOPLPUSH)
        conn.lpush::<_, _, ()>(&self.primary_queue_name, &serialized)
            .await
            .map_err(|e| QueueError::RedisOperationFailed {
                operation: "lpush".to_string(),
                source: e,
            })?;

        trace!(
            queue = %self.primary_queue_name,
            "Pushed item to Redis queue"
        );

        Ok(())
    }

    async fn ack(&self, item: &T) -> Result<()> {
        let mut conn = self
            .pool
            .get()
            .await
            .map_err(|e| QueueError::ConnectionFailed {
                queue_type: "redis".to_string(),
                details: e.to_string(),
            })?;

        let serialized = serde_json::to_string(item)?;

        // Remove one instance of the item from worker queue
        let removed: i64 = conn
            .lrem(&self.worker_queue_name, 1, &serialized)
            .await
            .map_err(|e| QueueError::RedisOperationFailed {
                operation: "lrem (ack)".to_string(),
                source: e,
            })?;

        if removed == 0 {
            trace!("Item not found in worker queue for acknowledgment");
        } else {
            trace!(count = removed, "Acknowledged item(s) from worker queue");
        }

        Ok(())
    }

    async fn try_push(&self, work: T) -> Result<()> {
        // Redis LPUSH is non-blocking, so we can use the same implementation
        self.push(work).await
    }

    async fn depth(&self) -> Option<usize> {
        match self.pool.get().await {
            Ok(mut conn) => {
                // Get the length of the primary queue
                match conn.llen::<_, usize>(&self.primary_queue_name).await {
                    Ok(depth) => Some(depth),
                    Err(e) => {
                        error!(
                            error = ?e,
                            "Failed to get queue depth"
                        );
                        None
                    }
                }
            }
            Err(e) => {
                error!(
                    error = ?e,
                    "Failed to get Redis connection for depth check"
                );
                None
            }
        }
    }

    async fn is_healthy(&self) -> bool {
        // Try to ping Redis
        match self.pool.get().await {
            Ok(mut conn) => {
                match deadpool_redis::redis::cmd("PING")
                    .query_async::<String>(&mut conn)
                    .await
                {
                    Ok(response) => response == "PONG",
                    Err(_) => false,
                }
            }
            Err(_) => false,
        }
    }
}

/// Queue statistics for monitoring.
#[derive(Debug, Clone)]
pub struct QueueStats {
    /// Number of items in primary queue
    pub primary_queue_depth: usize,
    /// Number of items in worker queue
    pub worker_queue_depth: usize,
    /// Total items across both queues
    pub total_items: usize,
    /// Worker ID
    pub worker_id: String,
}
