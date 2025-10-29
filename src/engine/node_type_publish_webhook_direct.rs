//! Direct webhook publishing node for synchronous HTTP requests.
//!
//! This module implements the direct webhook publishing evaluator that sends
//! HTTP POST requests immediately without queuing. It's used when the webhook
//! queue system is disabled or not needed.
//!
//! # Usage
//!
//! This evaluator is automatically selected when `WEBHOOK_QUEUE_ENABLED=false`
//! or when the node configuration specifies `use_async_queue: false`.
//!
//! # Example Blueprint
//!
//! ```json
//! {
//!   "type": "publish_webhook",
//!   "configuration": {
//!     "url": "https://api.example.com/webhook",
//!     "timeout_ms": 5000,
//!     "headers": {
//!       "Authorization": "Bearer token123",
//!       "X-Alert-Type": "urgent-post"
//!     }
//!   },
//!   "payload": {}
//! }
//! ```

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;
use tracing::{debug, error, info, instrument, warn};

use crate::storage::node::Node;

use super::evaluator::NodeEvaluator;
use super::node_type_common::{extract_headers_from_config, is_webhook_success_status};
use super::node_type_publish_webhook_common::{extract_webhook_payload, validate_webhook_config};

/// Direct webhook evaluator for synchronous HTTP requests.
///
/// This evaluator sends HTTP POST requests directly to configured webhook URLs
/// without using a queue. It's suitable for low-volume webhooks or when
/// immediate delivery is required.
///
/// # Configuration
///
/// The node's configuration field requires:
/// - `url`: The webhook URL to send the POST request to (required, HTTPS only)
/// - `headers`: Additional headers to include (optional, object of key-value pairs)
///
/// The node's payload field can be:
/// - A string: Used as field name to extract from input
/// - An object: Evaluated with DataLogic to produce the webhook data
///
/// # Response Handling
///
/// The node handles responses as follows:
/// - Success (200 or 204): Returns the cloned input
/// - Any other status: Returns an error
/// - Redirects are not followed
/// - Network Error: Returns an error with details
pub struct PublishWebhookDirectEvaluator {
    http_client: Arc<reqwest::Client>,
}

impl PublishWebhookDirectEvaluator {
    /// Create a new direct webhook evaluator
    pub fn new(http_client: Arc<reqwest::Client>) -> Self {
        Self { http_client }
    }
}

#[async_trait]
impl NodeEvaluator for PublishWebhookDirectEvaluator {
    #[instrument(skip(self, input), fields(
        node.type = %node.node_type,
        node.aturi = %node.aturi,
    ))]
    async fn evaluate(&self, node: &Node, input: &Value) -> Result<Option<Value>> {
        debug!("Evaluating publish_webhook node (direct mode)");

        // Validate configuration
        validate_webhook_config(&node.configuration)?;

        // Extract URL from configuration
        let url = node
            .configuration
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("publish_webhook node requires 'url' in configuration"))?;

        // Extract webhook payload using common utility
        let webhook_data = extract_webhook_payload(node, input)?;

        debug!(
            webhook.url = %url,
            webhook.mode = "direct",
            "Preparing webhook request"
        );

        // Extract headers from configuration
        let headers = extract_headers_from_config(&node.configuration)?;

        // Build the request
        let mut request = self.http_client.post(url).json(&webhook_data);

        // Add all headers
        for (key, value) in &headers {
            request = request.header(key.as_str(), value.as_str());
            if key != "Content-Type" {
                debug!(header.name = %key, header.value = %value, "Added custom header");
            }
        }

        // Send the request
        let response = request.send().await;

        // Handle the response
        match response {
            Ok(response) => {
                let status = response.status();
                let status_code = status.as_u16();

                // Only accept 200 or 204 as success
                if is_webhook_success_status(status_code) {
                    info!(
                        webhook.status = status_code,
                        webhook.mode = "direct",
                        "Webhook request successful"
                    );
                    // Return cloned input on success
                    Ok(Some(input.clone()))
                } else {
                    // Try to get error details from response
                    let error_body = response
                        .text()
                        .await
                        .unwrap_or_else(|_| String::from("Unable to read response body"));

                    warn!(
                        webhook.status = status_code,
                        webhook.error = %error_body,
                        webhook.mode = "direct",
                        "Webhook request failed with non-200/204 status"
                    );

                    Err(anyhow!(
                        "Webhook request failed with status {}: {}",
                        status,
                        error_body
                    ))
                }
            }
            Err(e) => {
                error!(
                    webhook.error = %e,
                    webhook.mode = "direct",
                    "Failed to send webhook request"
                );
                Err(anyhow!("Failed to send webhook request: {}", e))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use serde_json::json;

    #[tokio::test]
    async fn test_direct_webhook_missing_url() {
        let http_client = Arc::new(reqwest::Client::new());
        let evaluator = PublishWebhookDirectEvaluator::new(http_client);

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
    async fn test_direct_webhook_invalid_url() {
        let http_client = Arc::new(reqwest::Client::new());
        let evaluator = PublishWebhookDirectEvaluator::new(http_client);

        let node = Node {
            aturi: "test_node".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "publish_webhook".to_string(),
            payload: json!("data"), // Use string payload
            configuration: json!({
                "url": "not-a-valid-url"
            }),
            created_at: Utc::now(),
        };

        let input = json!({"data": {"test": "value"}});

        let result = evaluator.evaluate(&node, &input).await;
        assert!(result.is_err());

        let error = result.unwrap_err();
        // Will fail validation because URL is not HTTPS
        assert!(
            error
                .to_string()
                .contains("Webhook URL must use HTTPS protocol")
        );
    }

    #[tokio::test]
    async fn test_direct_webhook_non_https_url() {
        let http_client = Arc::new(reqwest::Client::new());
        let evaluator = PublishWebhookDirectEvaluator::new(http_client);

        let node = Node {
            aturi: "test_node".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "publish_webhook".to_string(),
            payload: json!("data"),
            configuration: json!({
                "url": "http://example.com/webhook" // Non-HTTPS URL
            }),
            created_at: Utc::now(),
        };

        let input = json!({"data": {"test": "value"}});

        let result = evaluator.evaluate(&node, &input).await;
        assert!(result.is_err());

        let error = result.unwrap_err();
        assert!(
            error
                .to_string()
                .contains("Webhook URL must use HTTPS protocol")
        );
    }

    #[tokio::test]
    async fn test_direct_webhook_string_payload_missing_field() {
        let http_client = Arc::new(reqwest::Client::new());
        let evaluator = PublishWebhookDirectEvaluator::new(http_client);

        let node = Node {
            aturi: "test_node".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "publish_webhook".to_string(),
            payload: json!("missing_field"),
            configuration: json!({
                "url": "https://example.com/webhook"
            }),
            created_at: Utc::now(),
        };

        let input = json!({"data": {"test": "value"}});

        let result = evaluator.evaluate(&node, &input).await;
        assert!(result.is_err());

        let error = result.unwrap_err();
        let err_msg = error.to_string();
        assert!(err_msg.contains("not found") || err_msg.contains("Field") || err_msg.contains("missing"));
    }

    #[tokio::test]
    async fn test_direct_webhook_object_payload() {
        let http_client = Arc::new(reqwest::Client::new());
        let evaluator = PublishWebhookDirectEvaluator::new(http_client);

        let node = Node {
            aturi: "test_node".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "publish_webhook".to_string(),
            payload: json!({"val": ["user", "profile"]}),
            configuration: json!({
                "url": "https://192.0.2.1/webhook", // Non-routable IP
                "headers": {
                    "X-Custom": "test"
                }
            }),
            created_at: Utc::now(),
        };

        let input = json!({
            "user": {
                "profile": {"name": "Alice"}
            }
        });

        // This will fail to connect but we're testing payload extraction
        let result = evaluator.evaluate(&node, &input).await;
        assert!(result.is_err());

        // Should fail on network error, not payload extraction
        let error = result.unwrap_err();
        let error_msg = error.to_string();
        // Could be timeout or connection error
        assert!(
            error_msg.contains("Failed to send webhook request")
                || error_msg.contains("Webhook request failed")
        );
    }
}
