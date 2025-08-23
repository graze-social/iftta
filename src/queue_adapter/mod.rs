//! Generic queue adapter system for work queue abstraction.
//!
//! This module provides a trait-based abstraction for different queue implementations,
//! allowing various queue backends (in-memory, Redis, PostgreSQL, etc.) to be used
//! interchangeably throughout the application.
//!
//! # Architecture
//!
//! The queue adapter system follows a strategy pattern where different implementations
//! of the `QueueAdapter` trait can be swapped based on deployment needs:
//!
//! - **Development**: Use `MpscQueueAdapter` for simple in-memory queuing
//! - **Production**: Use `RedisQueueAdapter` for distributed, persistent queuing
//! - **Future**: PostgreSQL, Kafka, or other backends can be added
//!
//! # Features
//!
//! - **Generic**: Works with any `Send + Sync` type
//! - **Async**: Full async/await support for non-blocking operations
//! - **Reliable**: Optional acknowledgment for at-least-once delivery
//! - **Observable**: Health checks and depth monitoring
//! - **Extensible**: Easy to add new implementations
//!
//! # Example
//!
//! ```rust,ignore
//! use ifthisthenat::queue_adapter::{QueueAdapter, MpscQueueAdapter};
//! use serde::{Serialize, Deserialize};
//!
//! #[derive(Debug, Clone, Serialize, Deserialize)]
//! struct WorkItem {
//!     id: String,
//!     data: String,
//! }
//!
//! #[tokio::main]
//! async fn main() -> Result<()> {
//!     // Create an in-memory queue
//!     let queue = Arc::new(MpscQueueAdapter::<WorkItem>::new(100));
//!     
//!     // Push work
//!     queue.push(WorkItem {
//!         id: "123".to_string(),
//!         data: "process this".to_string(),
//!     }).await?;
//!     
//!     // Pull and process work
//!     while let Some(work) = queue.pull().await {
//!         println!("Processing: {:?}", work);
//!         queue.ack(&work).await?;
//!     }
//!     
//!     Ok(())
//! }
//! ```

use anyhow::Result;
use async_trait::async_trait;

// Re-export implementations
mod mpsc;
mod redis;

pub use mpsc::MpscQueueAdapter;
pub use redis::RedisQueueAdapter;

/// Generic trait for queue adapters that can work with any work type.
///
/// This trait provides a common interface for different queue implementations
/// (MPSC, Redis, PostgreSQL, etc.) allowing them to be used interchangeably.
///
/// # Type Parameters
///
/// * `T` - The type of work items in the queue. Must be `Send + Sync` for thread safety.
///
/// # Implementation Guidelines
///
/// When implementing this trait:
///
/// 1. **Thread Safety**: Ensure the implementation is thread-safe (`Send + Sync`)
/// 2. **Error Handling**: Use descriptive error messages following error-iftta format
/// 3. **Cancellation**: Handle cancellation tokens appropriately
/// 4. **Monitoring**: Implement depth() and is_healthy() for observability
/// 5. **Performance**: Consider batching and connection pooling where applicable
///
/// # Reliability Patterns
///
/// Different implementations provide different reliability guarantees:
///
/// - **At-most-once**: Item is processed once or lost (MPSC default)
/// - **At-least-once**: Item is processed at least once (Redis with ack)
/// - **Exactly-once**: Item is processed exactly once (requires transactions)
#[async_trait]
pub trait QueueAdapter<T>: Send + Sync
where
    T: Send + Sync + 'static,
{
    /// Pull the next work item from the queue.
    ///
    /// This method blocks until an item is available or the queue is closed.
    /// Implementations may use different strategies:
    ///
    /// - **MPSC**: Returns None when channel is closed
    /// - **Redis**: Uses RPOPLPUSH for atomic move to worker queue
    /// - **PostgreSQL**: Could use SELECT ... FOR UPDATE SKIP LOCKED
    ///
    /// # Returns
    ///
    /// * `Some(T)` - The next work item
    /// * `None` - Queue is closed or empty (implementation-specific)
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// while let Some(work) = queue.pull().await {
    ///     process_work(work).await;
    ///     queue.ack(&work).await?;
    /// }
    /// ```
    async fn pull(&self) -> Option<T>;

    /// Push a work item to the queue.
    ///
    /// This method may block if the queue is full (implementation-specific).
    /// Use `try_push` for non-blocking behavior.
    ///
    /// # Arguments
    ///
    /// * `work` - The work item to enqueue
    ///
    /// # Returns
    ///
    /// * `Ok(())` - Item successfully queued
    /// * `Err` - Queue is full, closed, or backend error
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// queue.push(WorkItem { id: "123", data: "..." }).await?;
    /// ```
    async fn push(&self, work: T) -> Result<()>;

    /// Acknowledge successful processing of a work item.
    ///
    /// This method is used by reliable queue implementations to confirm
    /// that an item has been successfully processed and can be removed
    /// from any temporary processing queue.
    ///
    /// # Default Implementation
    ///
    /// The default implementation is a no-op for queues that don't
    /// require acknowledgment (like simple MPSC channels).
    ///
    /// # Arguments
    ///
    /// * `item` - The work item that was successfully processed
    ///
    /// # Returns
    ///
    /// * `Ok(())` - Acknowledgment succeeded or wasn't needed
    /// * `Err` - Acknowledgment failed (item not found, connection error, etc.)
    ///
    /// # Reliability Considerations
    ///
    /// For at-least-once delivery:
    /// 1. Item is moved to worker queue on pull
    /// 2. Item stays in worker queue during processing
    /// 3. Item is removed from worker queue on ack
    /// 4. If worker crashes, items in worker queue can be recovered
    async fn ack(&self, _item: &T) -> Result<()> {
        // Default no-op implementation for queues that don't need acknowledgment
        Ok(())
    }

    /// Try to push a work item without blocking.
    ///
    /// This method returns immediately if the queue is full rather than
    /// waiting for space to become available.
    ///
    /// # Arguments
    ///
    /// * `work` - The work item to enqueue
    ///
    /// # Returns
    ///
    /// * `Ok(())` - Item successfully queued
    /// * `Err` - Queue is full, closed, or backend error
    ///
    /// # Use Cases
    ///
    /// - Preventing backpressure in event processors
    /// - Work shedding under high load
    /// - Non-blocking producers
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// match queue.try_push(work).await {
    ///     Ok(()) => debug!("Work queued"),
    ///     Err(e) if e.to_string().contains("full") => {
    ///         warn!("Queue full, dropping work");
    ///     }
    ///     Err(e) => error!("Queue error: {}", e),
    /// }
    /// ```
    async fn try_push(&self, work: T) -> Result<()> {
        // Default implementation uses regular push
        self.push(work).await
    }

    /// Get the current queue depth if available.
    ///
    /// Returns the number of items currently in the queue.
    /// This is useful for monitoring and autoscaling decisions.
    ///
    /// # Returns
    ///
    /// * `Some(count)` - Number of items in queue
    /// * `None` - Queue doesn't support depth queries
    ///
    /// # Implementation Notes
    ///
    /// - May be approximate for some implementations
    /// - Should be efficient (O(1) if possible)
    /// - Used for metrics and health checks
    async fn depth(&self) -> Option<usize> {
        None
    }

    /// Check if the queue is healthy and operational.
    ///
    /// This method is used for health checks and circuit breaking.
    /// Implementations should verify connectivity and basic functionality.
    ///
    /// # Returns
    ///
    /// * `true` - Queue is operational
    /// * `false` - Queue has issues (connection lost, backend down, etc.)
    ///
    /// # Health Check Strategies
    ///
    /// - **MPSC**: Check if channel is closed
    /// - **Redis**: PING command
    /// - **PostgreSQL**: Simple SELECT 1 query
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// if !queue.is_healthy().await {
    ///     error!("Queue unhealthy, switching to fallback");
    ///     use_fallback_queue();
    /// }
    /// ```
    async fn is_healthy(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// Test that the trait is object-safe and can be used with dynamic dispatch
    #[test]
    fn test_trait_object_safety() {
        fn _assert_object_safe(_: &dyn QueueAdapter<String>) {}
    }

    /// Test that Arc<dyn QueueAdapter> can be created
    #[test]
    fn test_arc_dyn_queue_adapter() {
        fn _assert_sendable(_: Arc<dyn QueueAdapter<String>>) {}
    }
}
