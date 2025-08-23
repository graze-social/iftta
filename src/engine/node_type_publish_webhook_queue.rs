//! Queue-based webhook publishing node for asynchronous processing.
//!
//! This module implements the webhook publishing evaluator that enqueues
//! webhook requests for background processing. It's used when the webhook
//! queue system is enabled for high-throughput or fault-tolerant delivery.
//!
//! # Usage
//!
//! This evaluator is automatically selected when `WEBHOOK_QUEUE_ENABLED=true`.
//! Webhooks are enqueued and processed by a background task with retry logic.
//!
//! # Example Blueprint
//!
//! ```json
//! {
//!   "type": "publish_webhook",
//!   "configuration": {
//!     "url": "https://api.example.com/webhook",
//!     "max_retries": 5,
//!     "timeout_ms": 10000,
//!     "correlation_id": "order-12345",
//!     "headers": {
//!       "Authorization": "Bearer token",
//!       "X-Custom-Header": "value"
//!     }
//!   },
//!   "payload": {}
//! }
//! ```

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use tokio::sync::mpsc;
use tracing::{debug, info, instrument};

use crate::storage::node::Node;
use crate::tasks::webhook::{WebhookMetadata, WebhookWork};

use super::evaluator::NodeEvaluator;
use super::node_type_common::extract_headers_from_config;
use super::node_type_publish_webhook_common::{extract_webhook_payload, validate_webhook_config};

/// Queue-based webhook evaluator for asynchronous processing.
///
/// This evaluator enqueues webhook requests to be processed by a background
/// task. It provides fault tolerance, retry logic, and high throughput for
/// webhook delivery.
///
/// # Configuration
///
/// The node's configuration field supports:
/// - `url`: The webhook URL to send the POST request to (required, HTTPS only)
/// - `headers`: Additional headers to include (optional, object of key-value pairs)
/// - `max_retries`: Maximum retry attempts (optional, default: 3)
/// - `correlation_id`: Correlation ID for tracking (optional)
/// - `tags`: Additional metadata tags (optional, object of key-value pairs)
///
/// The node's payload field can be:
/// - A string: Used as field name to extract from input
/// - An object: Evaluated with DataLogic to produce the webhook data
///
/// # Queue Behavior
///
/// - Webhooks are enqueued immediately and processed asynchronously
/// - Failed webhooks are retried with exponential backoff
/// - The queue has a configurable size limit
/// - Overflow behavior depends on the queue implementation
pub struct PublishWebhookQueueEvaluator {
    webhook_sender: mpsc::Sender<WebhookWork>,
}

impl PublishWebhookQueueEvaluator {
    /// Create a new queue-based webhook evaluator
    pub fn new(webhook_sender: mpsc::Sender<WebhookWork>) -> Self {
        Self { webhook_sender }
    }
}

#[async_trait]
impl NodeEvaluator for PublishWebhookQueueEvaluator {
    #[instrument(skip(self, input), fields(
        node.type = %node.node_type,
        node.aturi = %node.aturi,
    ))]
    async fn evaluate(&self, node: &Node, input: &Value) -> Result<Option<Value>> {
        debug!("Evaluating publish_webhook node (queue mode)");

        // Validate configuration
        validate_webhook_config(&node.configuration)?;

        // Extract URL from configuration
        let url = node
            .configuration
            .get("url")
            .and_then(|v| v.as_str())
            .unwrap(); // Safe because validate_webhook_config ensures this exists

        // Extract webhook payload using common utility
        let webhook_data = extract_webhook_payload(node, input)?;

        // Serialize webhook data to JSON bytes
        let body = serde_json::to_vec(&webhook_data)
            .map_err(|e| anyhow!("Failed to serialize webhook body: {}", e))?;

        // Extract headers from configuration
        let headers = extract_headers_from_config(&node.configuration)?;
        for (key, value) in &headers {
            if key != "Content-Type" {
                debug!(header.name = %key, header.value = %value, "Added custom header");
            }
        }

        // Get optional parameters
        let timeout_ms = None; // Will use default timeout in task

        let max_retries = node
            .configuration
            .get("max_retries")
            .and_then(|v| v.as_u64())
            .unwrap_or(3) as u32;

        let correlation_id = node
            .configuration
            .get("correlation_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        // Extract tags from configuration
        let mut tags = HashMap::new();
        if let Some(tags_value) = node.configuration.get("tags")
            && let Some(tags_obj) = tags_value.as_object()
        {
            for (key, value) in tags_obj {
                if let Some(tag_value) = value.as_str() {
                    tags.insert(key.clone(), tag_value.to_string());
                }
            }
        }

        // Create webhook work item
        let work = WebhookWork {
            id: ulid::Ulid::new().to_string(),
            url: url.to_string(),
            headers,
            body,
            timeout_ms,
            retry_count: 0,
            max_retries,
            metadata: WebhookMetadata {
                source_blueprint: node.blueprint.clone(),
                source_node: node.aturi.clone(),
                created_at: chrono::Utc::now(),
                correlation_id,
                tags,
            },
        };

        debug!(
            webhook.id = %work.id,
            webhook.url = %url,
            webhook.max_retries = max_retries,
            webhook.mode = "queue",
            "Enqueuing webhook for async processing"
        );

        // Send to queue
        self.webhook_sender
            .send(work.clone())
            .await
            .map_err(|e| anyhow!("Failed to enqueue webhook: {}", e))?;

        info!(
            webhook.id = %work.id,
            webhook.url = %url,
            webhook.mode = "queue",
            "Webhook enqueued for async processing"
        );

        // Return cloned input on success
        Ok(Some(input.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use serde_json::json;
    use tokio::sync::mpsc;

    #[tokio::test]
    async fn test_queue_webhook_missing_url() {
        let (_sender, receiver) = mpsc::channel(10);
        drop(receiver); // Drop receiver to simulate closed channel
        let evaluator = PublishWebhookQueueEvaluator::new(_sender);

        let node = Node {
            aturi: "test_node".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "publish_webhook".to_string(),
            payload: json!({}),
            configuration: json!({}), // Missing URL
            created_at: Utc::now(),
        };

        let input = json!({"test": "data"});

        let result = evaluator.evaluate(&node, &input).await;
        assert!(result.is_err());

        let error = result.unwrap_err();
        assert!(
            error
                .to_string()
                .contains("Configuration must contain 'url' field")
        );
    }

    #[tokio::test]
    async fn test_queue_webhook_successful_enqueue() {
        let (sender, mut receiver) = mpsc::channel(10);
        let evaluator = PublishWebhookQueueEvaluator::new(sender);

        let node = Node {
            aturi: "test_node".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "publish_webhook".to_string(),
            payload: json!({}),
            configuration: json!({
                "url": "https://api.example.com/webhook",
                "max_retries": 5,
                "correlation_id": "test-123",
                "headers": {
                    "Authorization": "Bearer token"
                },
                "tags": {
                    "environment": "test"
                }
            }),
            created_at: Utc::now(),
        };

        let input = json!({"test": "data"});

        let result = evaluator.evaluate(&node, &input).await;
        assert!(result.is_ok());

        let response = result.unwrap();
        assert!(response.is_some());

        let response_value = response.unwrap();
        // Should return cloned input
        assert_eq!(response_value, input);

        // Verify work was enqueued
        let work = receiver.try_recv();
        assert!(work.is_ok());

        let work = work.unwrap();
        assert_eq!(work.url, "https://api.example.com/webhook");
        assert_eq!(work.max_retries, 5);
        assert_eq!(work.metadata.correlation_id, Some("test-123".to_string()));
        assert_eq!(
            work.headers.get("Authorization"),
            Some(&"Bearer token".to_string())
        );
        assert_eq!(
            work.metadata.tags.get("environment"),
            Some(&"test".to_string())
        );
    }

    #[tokio::test]
    async fn test_queue_webhook_channel_full() {
        let (sender, _receiver) = mpsc::channel(1);
        let evaluator = PublishWebhookQueueEvaluator::new(sender.clone());

        // Fill the channel
        let work = WebhookWork {
            id: "block".to_string(),
            url: "https://example.com".to_string(),
            headers: HashMap::new(),
            body: vec![],
            timeout_ms: None,
            retry_count: 0,
            max_retries: 0,
            metadata: WebhookMetadata {
                source_blueprint: "test".to_string(),
                source_node: "test".to_string(),
                created_at: Utc::now(),
                correlation_id: None,
                tags: HashMap::new(),
            },
        };
        sender.send(work).await.unwrap();

        let node = Node {
            aturi: "test_node".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "publish_webhook".to_string(),
            payload: json!({}),
            configuration: json!({
                "url": "https://api.example.com/webhook"
            }),
            created_at: Utc::now(),
        };

        let input = json!({"test": "data"});

        // This should block until timeout or channel has space
        // In a real scenario, the background task would consume items
        let result = tokio::time::timeout(
            std::time::Duration::from_millis(100),
            evaluator.evaluate(&node, &input),
        )
        .await;

        // Should timeout since channel is full and no consumer
        assert!(result.is_err());
    }
}
