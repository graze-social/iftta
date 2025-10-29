//! MPSC channel-based queue adapter for in-memory work queuing.
//!
//! This module provides a simple, efficient queue implementation using
//! Tokio's multi-producer, single-consumer channels. It's ideal for
//! single-instance deployments and development environments.
//!
//! # Characteristics
//!
//! - **In-Memory**: No persistence, data lost on restart
//! - **Bounded**: Configurable buffer size for backpressure
//! - **Fast**: No network overhead, minimal latency
//! - **Simple**: No external dependencies
//! - **At-Most-Once**: No delivery guarantees after crash
//!
//! # When to Use
//!
//! Use `MpscQueueAdapter` when:
//! - Running a single instance of the service
//! - Data loss on restart is acceptable
//! - Low latency is critical
//! - Simplicity is preferred over reliability
//!
//! # When NOT to Use
//!
//! Avoid `MpscQueueAdapter` when:
//! - Running multiple instances (no work distribution)
//! - Data persistence is required
//! - At-least-once delivery is needed
//! - Queue needs to survive process restarts

use anyhow::Result;
use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::{Mutex, mpsc};
use tracing::trace;

use super::QueueAdapter;
use crate::errors::QueueError;

/// MPSC channel-based queue adapter implementation.
///
/// This adapter wraps Tokio's MPSC channel to provide a queue interface
/// compatible with the `QueueAdapter` trait. The receiver is wrapped in
/// an Arc<Mutex> to allow sharing across async tasks.
///
/// # Type Parameters
///
/// * `T` - The type of work items. Must be `Send + Sync + 'static`.
///
/// # Example
///
/// ```rust,ignore
/// use ifthisthenat::queue_adapter::MpscQueueAdapter;
///
/// // Create a queue with buffer for 100 items
/// let queue = MpscQueueAdapter::<WorkItem>::new(100);
///
/// // Get a sender for producers
/// let sender = queue.sender();
/// tokio::spawn(async move {
///     sender.send(WorkItem::new()).await.unwrap();
/// });
///
/// // Consumer pulls from queue
/// while let Some(work) = queue.pull().await {
///     process(work).await;
/// }
/// ```
pub struct MpscQueueAdapter<T>
where
    T: Send + Sync + 'static,
{
    /// The receiver wrapped for shared access
    receiver: Arc<Mutex<mpsc::Receiver<T>>>,
    /// The sender for pushing items
    sender: mpsc::Sender<T>,
}

impl<T> MpscQueueAdapter<T>
where
    T: Send + Sync + 'static,
{
    /// Create a new MPSC queue adapter with the specified buffer size.
    ///
    /// # Arguments
    ///
    /// * `buffer` - Maximum number of items that can be buffered.
    ///   When full, senders will block on `push()`.
    ///
    /// # Returns
    ///
    /// A new `MpscQueueAdapter` instance.
    ///
    /// # Buffer Size Guidelines
    ///
    /// - **Small (1-10)**: Tight backpressure, minimal memory
    /// - **Medium (100-1000)**: Good for burst handling
    /// - **Large (10000+)**: Handle traffic spikes, more memory
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// // Development: small buffer
    /// let dev_queue = MpscQueueAdapter::<Work>::new(10);
    ///
    /// // Production: larger buffer for bursts
    /// let prod_queue = MpscQueueAdapter::<Work>::new(1000);
    /// ```
    pub fn new(buffer: usize) -> Self {
        let (sender, receiver) = mpsc::channel(buffer);
        Self {
            receiver: Arc::new(Mutex::new(receiver)),
            sender,
        }
    }

    /// Create an adapter from existing MPSC channels.
    ///
    /// This method is provided for backward compatibility and testing
    /// scenarios where you need direct control over channel creation.
    ///
    /// # Arguments
    ///
    /// * `sender` - Existing sender channel
    /// * `receiver` - Existing receiver channel
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let (tx, rx) = mpsc::channel(100);
    /// let queue = MpscQueueAdapter::from_channel(tx.clone(), rx);
    /// ```
    pub fn from_channel(sender: mpsc::Sender<T>, receiver: mpsc::Receiver<T>) -> Self {
        Self {
            receiver: Arc::new(Mutex::new(receiver)),
            sender,
        }
    }

    /// Get a clone of the sender for producer use.
    ///
    /// This allows multiple producers to send to the same queue.
    /// The sender can be safely cloned and shared across threads.
    ///
    /// # Returns
    ///
    /// A cloned `mpsc::Sender<T>` that can be moved to other tasks.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let queue = MpscQueueAdapter::<Work>::new(100);
    ///
    /// // Spawn multiple producers
    /// for i in 0..10 {
    ///     let sender = queue.sender();
    ///     tokio::spawn(async move {
    ///         sender.send(Work::new(i)).await.unwrap();
    ///     });
    /// }
    /// ```
    pub fn sender(&self) -> mpsc::Sender<T> {
        self.sender.clone()
    }

    /// Get the maximum capacity of the channel.
    ///
    /// Returns the maximum number of items that can be buffered.
    pub fn max_capacity(&self) -> usize {
        self.sender.max_capacity()
    }

    /// Get the current available capacity.
    ///
    /// Returns the number of items that can be sent without blocking.
    /// Note: This is a snapshot and may change immediately after reading.
    pub fn available_capacity(&self) -> usize {
        self.sender.capacity()
    }
}

#[async_trait]
impl<T> QueueAdapter<T> for MpscQueueAdapter<T>
where
    T: Send + Sync + 'static,
{
    async fn pull(&self) -> Option<T> {
        let mut receiver = self.receiver.lock().await;
        let result = receiver.recv().await;
        trace!(has_item = result.is_some(), "Pulled item from MPSC queue");
        result
    }

    async fn push(&self, work: T) -> Result<()> {
        self.sender
            .send(work)
            .await
            .map_err(|e| QueueError::MpscOperationFailed {
                operation: "send".to_string(),
                details: e.to_string(),
            })?;
        trace!("Pushed item to MPSC queue");
        Ok(())
    }

    async fn try_push(&self, work: T) -> Result<()> {
        self.sender.try_send(work).map_err(|e| match e {
            mpsc::error::TrySendError::Full(_) => QueueError::CapacityExceeded {
                queue_type: "mpsc".to_string(),
                capacity: self.sender.max_capacity(),
            },
            mpsc::error::TrySendError::Closed(_) => QueueError::MpscOperationFailed {
                operation: "try_send".to_string(),
                details: "Channel closed".to_string(),
            },
        })?;
        trace!("Try-pushed item to MPSC queue");
        Ok(())
    }

    async fn depth(&self) -> Option<usize> {
        // Calculate approximate depth: max_capacity - available_capacity
        // This is an approximation as MPSC doesn't provide exact depth
        let depth = self.sender.max_capacity() - self.sender.capacity();
        Some(depth)
    }

    async fn is_healthy(&self) -> bool {
        // Channel is healthy if not closed
        !self.sender.is_closed()
    }

    // Note: ack() uses default no-op implementation since MPSC doesn't need acknowledgment
}

// Clone implementation for sharing the adapter
impl<T> Clone for MpscQueueAdapter<T>
where
    T: Send + Sync + 'static,
{
    fn clone(&self) -> Self {
        Self {
            receiver: self.receiver.clone(),
            sender: self.sender.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq)]
    struct TestWork {
        id: String,
        data: String,
    }

    #[tokio::test]
    async fn test_push_pull_basic() {
        let adapter = Arc::new(MpscQueueAdapter::<TestWork>::new(10));

        let work = TestWork {
            id: "test-1".to_string(),
            data: "test data".to_string(),
        };

        // Push work
        adapter.push(work.clone()).await.unwrap();

        // Pull work
        let pulled = adapter.pull().await;
        assert!(pulled.is_some());
        assert_eq!(pulled.unwrap(), work);
    }

    #[tokio::test]
    async fn test_multiple_items() {
        let adapter = Arc::new(MpscQueueAdapter::<i32>::new(10));

        // Push multiple items
        for i in 0..5 {
            adapter.push(i).await.unwrap();
        }

        // Pull items in FIFO order
        for expected in 0..5 {
            let pulled = adapter.pull().await;
            assert_eq!(pulled, Some(expected));
        }
    }

    #[tokio::test]
    async fn test_try_push_when_full() {
        let adapter = Arc::new(MpscQueueAdapter::<i32>::new(1));

        // Fill the queue
        adapter.try_push(1).await.unwrap();

        // Try to push when full
        let result = adapter.try_push(2).await;
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("full") || err_msg.contains("Queue"));
    }

    #[tokio::test]
    async fn test_concurrent_producers() {
        let adapter = Arc::new(MpscQueueAdapter::<i32>::new(100));
        let mut handles = vec![];

        // Spawn multiple producers
        for i in 0..10 {
            let queue = adapter.clone();
            let handle = tokio::spawn(async move {
                queue.push(i).await.unwrap();
            });
            handles.push(handle);
        }

        // Wait for all producers
        for handle in handles {
            handle.await.unwrap();
        }

        // Verify all items were queued
        let mut items = vec![];
        for _ in 0..10 {
            items.push(adapter.pull().await.unwrap());
        }
        items.sort();
        assert_eq!(items, (0..10).collect::<Vec<_>>());
    }

    #[tokio::test]
    async fn test_sender_clone() {
        let adapter = MpscQueueAdapter::<String>::new(10);
        let sender1 = adapter.sender();
        let sender2 = adapter.sender();

        // Both senders can send
        sender1.send("from-1".to_string()).await.unwrap();
        sender2.send("from-2".to_string()).await.unwrap();

        // Both items received
        assert_eq!(adapter.pull().await, Some("from-1".to_string()));
        assert_eq!(adapter.pull().await, Some("from-2".to_string()));
    }

    #[tokio::test]
    async fn test_from_channel() {
        let (sender, receiver) = mpsc::channel(5);
        let adapter = Arc::new(MpscQueueAdapter::from_channel(sender.clone(), receiver));

        // Send via original sender
        sender.send("direct".to_string()).await.unwrap();

        // Pull via adapter
        assert_eq!(adapter.pull().await, Some("direct".to_string()));
    }

    #[tokio::test]
    async fn test_depth_tracking() {
        let adapter = Arc::new(MpscQueueAdapter::<i32>::new(10));

        // Initially empty
        assert_eq!(adapter.depth().await, Some(0));

        // Add items
        for i in 0..5 {
            adapter.push(i).await.unwrap();
        }

        // Check depth
        assert_eq!(adapter.depth().await, Some(5));

        // Remove an item
        adapter.pull().await;

        // Depth should decrease
        assert_eq!(adapter.depth().await, Some(4));
    }

    #[tokio::test]
    async fn test_health_check() {
        let adapter = Arc::new(MpscQueueAdapter::<i32>::new(10));

        // Should be healthy initially
        assert!(adapter.is_healthy().await);

        // Even after operations
        adapter.push(42).await.unwrap();
        assert!(adapter.is_healthy().await);
    }

    #[tokio::test]
    async fn test_ack_is_noop() {
        let adapter = Arc::new(MpscQueueAdapter::<String>::new(10));

        adapter.push("test".to_string()).await.unwrap();
        let item = adapter.pull().await.unwrap();

        // Ack should succeed (no-op)
        assert!(adapter.ack(&item).await.is_ok());
    }

    #[tokio::test]
    async fn test_pull_from_empty_queue() {
        let adapter = Arc::new(MpscQueueAdapter::<i32>::new(10));

        // Start pulling before any items exist
        let adapter_clone = adapter.clone();
        let pull_handle = tokio::spawn(async move { adapter_clone.pull().await });

        // Give pull time to start waiting
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

        // Push an item
        adapter.push(42).await.unwrap();

        // Pull should receive it
        let result = pull_handle.await.unwrap();
        assert_eq!(result, Some(42));
    }

    #[tokio::test]
    async fn test_capacity_methods() {
        let adapter = MpscQueueAdapter::<i32>::new(10);

        assert_eq!(adapter.max_capacity(), 10);
        assert!(adapter.available_capacity() <= 10);

        // Fill partially
        for i in 0..5 {
            adapter.push(i).await.unwrap();
        }

        // Available capacity should be reduced
        assert!(adapter.available_capacity() <= 5);
    }
}
