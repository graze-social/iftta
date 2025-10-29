//! Queue adapter for blueprint evaluation tasks.
//!
//! This module provides an abstraction layer for blueprint work queuing,
//! allowing different queue implementations (mpsc, Redis, PostgreSQL, etc.)
//! to be used interchangeably.

use anyhow::Result;
use deadpool_redis::Pool as RedisPool;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use std::sync::Arc;

use crate::{
    config::BlueprintQueueConfig,
    errors::TaskError,
    queue_adapter::{MpscQueueAdapter as GenericMpscQueueAdapter, QueueAdapter, RedisQueueAdapter},
};

// Custom serde module for Arc<Value> to enable serialization
mod arc_value_serde {
    use super::*;

    pub fn serialize<S>(value: &Arc<Value>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        value.as_ref().serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Arc<Value>, D::Error>
    where
        D: Deserializer<'de>,
    {
        Value::deserialize(deserializer).map(Arc::new)
    }
}

/// Work item for blueprint evaluation that supports both traced and non-traced modes.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BlueprintWork {
    /// The AT-URI of the blueprint being processed
    pub blueprint: String,
    /// The index of the node to be processed
    pub node_index: usize,
    /// The output of the last node processed as input to the next node
    /// Using Arc to avoid cloning large payloads when distributing to multiple blueprints
    #[serde(with = "arc_value_serde")]
    pub payload: Arc<Value>,
    /// Optional trace ID for correlation
    pub trace_id: Option<String>,
    /// Start time of the blueprint evaluation (for duration tracking)
    pub blueprint_start: Option<chrono::DateTime<chrono::Utc>>,
    /// Number of nodes evaluated so far
    pub nodes_evaluated: usize,
}

/// Type alias for blueprint queue adapter using the generic adapter
pub type BlueprintQueueAdapter = dyn QueueAdapter<BlueprintWork>;

/// MPSC channel-based queue adapter for BlueprintWork - uses generic implementation
pub type MpscBlueprintQueueAdapter = GenericMpscQueueAdapter<BlueprintWork>;

/// Redis-based queue adapter for BlueprintWork
pub type RedisBlueprintQueueAdapter = RedisQueueAdapter<BlueprintWork>;

/// Create a blueprint queue adapter based on configuration.
/// Returns the adapter and an mpsc::Sender for submitting work.
pub fn create_blueprint_queue_adapter(
    config: &BlueprintQueueConfig,
    redis_pool: Option<RedisPool>,
) -> Result<(
    Arc<dyn QueueAdapter<BlueprintWork>>,
    tokio::sync::mpsc::Sender<BlueprintWork>,
)> {
    match config.adapter_type.as_str() {
        "mpsc" => {
            tracing::info!(
                "Creating MPSC blueprint queue adapter with buffer size: {}",
                config.mpsc_buffer_size
            );
            let adapter = MpscBlueprintQueueAdapter::new(config.mpsc_buffer_size);
            let sender = adapter.sender();
            Ok((Arc::new(adapter), sender))
        }
        "redis" => {
            let pool = redis_pool.ok_or_else(|| TaskError::BlueprintAdapterInitFailed {
                details: "Redis pool required for Redis queue adapter".to_string(),
            })?;

            // Use configured worker ID or generate a random one
            let worker_id = config.redis_worker_id.clone().unwrap_or_else(|| {
                let id = uuid::Uuid::new_v4().to_string();
                tracing::info!("Generated worker ID for Redis queue: {}", id);
                id
            });

            tracing::info!(
                "Creating Redis blueprint queue adapter with prefix: {}, worker_id: {}",
                config.redis_queue_prefix,
                worker_id
            );

            let adapter = Arc::new(RedisBlueprintQueueAdapter::new(
                pool,
                Some(worker_id),
                Some(config.redis_queue_prefix.clone()),
            ));

            // For Redis (and other non-MPSC adapters), create a channel that forwards to the adapter
            let (sender, mut receiver) = tokio::sync::mpsc::channel::<BlueprintWork>(500);  // Reduced from 1000 to limit memory usage
            let adapter_clone = adapter.clone();

            // Spawn a task to forward from channel to Redis adapter
            tokio::spawn(async move {
                while let Some(work) = receiver.recv().await {
                    if let Err(e) = adapter_clone.push(work).await {
                        tracing::error!(error = ?e, "Failed to push blueprint work to Redis adapter");
                    }
                }
                tracing::info!("Redis blueprint queue forwarder stopped");
            });

            Ok((adapter, sender))
        }
        _ => Err(TaskError::BlueprintAdapterInitFailed {
            details: format!(
                "Unsupported blueprint queue adapter type: {}. Supported types: mpsc, redis",
                config.adapter_type
            ),
        }
        .into()),
    }
}

// Future implementations can be added here:
// - PostgresBlueprintQueueAdapter
// - KafkaBlueprintQueueAdapter
// - SQS/RabbitMQ adapters
// etc.

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Arc;

    #[tokio::test]
    async fn test_mpsc_blueprint_queue_adapter() {
        let adapter = Arc::new(MpscBlueprintQueueAdapter::new(10));

        // Test push and pull with minimal fields
        let work = BlueprintWork {
            blueprint: "test_blueprint".to_string(),
            node_index: 0,
            payload: Arc::new(json!({"test": "data"})),
            trace_id: None,
            blueprint_start: None,
            nodes_evaluated: 0,
        };

        adapter.push(work.clone()).await.unwrap();

        let pulled = adapter.pull().await;
        assert!(pulled.is_some());
        let pulled_work = pulled.unwrap();
        assert_eq!(pulled_work.blueprint, work.blueprint);
        assert_eq!(pulled_work.node_index, work.node_index);

        // Test ack (should succeed for MPSC adapter)
        assert!(adapter.ack(&pulled_work).await.is_ok());

        // Test health check
        assert!(adapter.is_healthy().await);
    }

    #[tokio::test]
    async fn test_mpsc_blueprint_queue_adapter_with_tracing() {
        let adapter = Arc::new(MpscBlueprintQueueAdapter::new(10));

        // Test push and pull with tracing fields
        let work = BlueprintWork {
            blueprint: "test_blueprint".to_string(),
            node_index: 0,
            payload: Arc::new(json!({"test": "data"})),
            trace_id: Some("trace-123".to_string()),
            blueprint_start: Some(chrono::Utc::now()),
            nodes_evaluated: 0,
        };

        adapter.push(work.clone()).await.unwrap();

        let pulled = adapter.pull().await;
        assert!(pulled.is_some());
        let pulled_work = pulled.unwrap();
        assert_eq!(pulled_work.blueprint, work.blueprint);
        assert_eq!(pulled_work.trace_id, work.trace_id);

        // Test ack (should succeed for MPSC adapter)
        assert!(adapter.ack(&pulled_work).await.is_ok());

        // Test health check
        assert!(adapter.is_healthy().await);
    }

    #[tokio::test]
    async fn test_try_push_when_full() {
        let adapter = Arc::new(MpscBlueprintQueueAdapter::new(1));

        let work = BlueprintWork {
            blueprint: "test".to_string(),
            node_index: 0,
            payload: Arc::new(json!({})),
            trace_id: None,
            blueprint_start: None,
            nodes_evaluated: 0,
        };

        // Fill the queue
        adapter.push(work.clone()).await.unwrap();

        // Try to push when full
        let result = adapter.try_push(work).await;
        assert!(result.is_err());
        let error_msg = result.unwrap_err().to_string();
        assert!(error_msg.contains("full") || error_msg.contains("Queue"));
    }
}
