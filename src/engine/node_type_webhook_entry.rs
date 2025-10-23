//! Webhook entry node for processing HTTP webhook requests.
//!
//! This module implements the entry point for blueprints that process incoming
//! webhook requests. Webhooks provide a way to trigger blueprints from external
//! systems via HTTP POST requests.
//!
//! # Usage
//!
//! Webhook entry nodes must be the first node in a blueprint. They evaluate
//! incoming webhook requests using either a simple boolean or a DataLogic
//! expression to determine whether to process the blueprint.
//!
//! # Configuration
//!
//! The configuration field must be an empty object `{}`. No configuration
//! options are currently supported for webhook_entry nodes.
//!
//! # Payload
//!
//! The payload can be either:
//! - A boolean value (`true` or `false`) for simple accept/reject logic
//! - A DataLogic expression object that evaluates to a boolean
//!
//! # Webhook Request Structure
//!
//! Webhook requests are passed to the node with the following structure:
//! ```json
//! {
//!   "headers": {
//!     "content-type": "application/json",
//!     "x-webhook-signature": "..."
//!   },
//!   "body": {
//!     // The parsed JSON body of the webhook request
//!   },
//!   "query": {
//!     // Query parameters from the URL
//!   },
//!   "method": "POST",
//!   "path": "/webhooks/blueprint-id"
//! }
//! ```
//!
//! # Example Blueprints
//!
//! Accept all webhook requests:
//! ```json
//! {
//!   "nodes": [
//!     {
//!       "type": "webhook_entry",
//!       "configuration": {},
//!       "payload": true
//!     }
//!   ]
//! }
//! ```
//!
//! Filter by event type and headers:
//! ```json
//! {
//!   "nodes": [
//!     {
//!       "type": "webhook_entry",
//!       "configuration": {},
//!       "payload": {
//!         "and": [
//!           {"==": [{"val": ["headers", "content-type"]}, "application/json"]},
//!           {"exists": [{"val": ["body", "event_type"]}]}
//!         ]
//!       }
//!     }
//!   ]
//! }
//! ```

use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;

use crate::errors::EngineError;
use crate::storage::node::Node;

use super::common::create_datalogic;
use super::evaluator::NodeEvaluator;

/// Evaluator for Webhook entry nodes.
///
/// This evaluator handles webhook inbound triggers by evaluating the payload
/// to determine whether to continue blueprint processing. The payload can be
/// either a simple boolean or a DataLogic expression that evaluates to boolean.
///
/// # Configuration
///
/// The configuration field must be an empty JSON object. No configuration
/// options are currently supported:
/// ```json
/// {}
/// ```
///
/// # Payload
///
/// The payload field can be either:
/// - A boolean value (`true` or `false`) for simple accept/reject logic
/// - A DataLogic expression object that must evaluate to a boolean
///
/// The payload is evaluated against the webhook request data to determine
/// whether to process the blueprint.
///
/// ## Payload Examples
///
/// Accept all webhook requests:
/// ```json
/// true
/// ```
///
/// Reject all webhook requests (useful for temporarily disabling):
/// ```json
/// false
/// ```
///
/// Filter by content type:
/// ```json
/// {
///   "==": [
///     {"val": ["headers", "content-type"]},
///     "application/json"
///   ]
/// }
/// ```
///
/// Filter by webhook signature presence:
/// ```json
/// {
///   "exists": [{"val": ["headers", "x-webhook-signature"]}]
/// }
/// ```
///
/// Filter by specific event type in body:
/// ```json
/// {
///   "==": [
///     {"val": ["body", "event_type"]},
///     "user.created"
///   ]
/// }
/// ```
///
/// Complex filter with multiple conditions:
/// ```json
/// {
///   "and": [
///     {"==": [{"val": ["method"]}, "POST"]},
///     {"==": [{"val": ["headers", "content-type"]}, "application/json"]},
///     {"in": [{"val": ["body", "action"]}, ["create", "update"]]},
///     {">": [{"val": ["body", "priority"]}, 5]}
///   ]
/// }
/// ```
///
/// # Behavior
///
/// 1. The evaluator receives webhook input data
/// 2. If payload is a boolean, uses that value directly
/// 3. If payload is an object, evaluates it as DataLogic expression
/// 4. The result must be a boolean:
///    - `true`: Continue blueprint evaluation (returns input data)
///    - `false`: Stop blueprint evaluation (returns None)
/// 5. Non-boolean results cause an error
///
/// # Input Data
///
/// The webhook input data typically contains:
/// - `method`: HTTP method (usually "POST")
/// - `headers`: HTTP headers as key-value pairs
/// - `body`: Request body (parsed JSON)
/// - `query`: Query parameters (if any)
/// - `path`: Request path
/// - Any additional webhook-specific fields
pub struct WebhookEntryEvaluator;

impl WebhookEntryEvaluator {
    /// Create a new Webhook entry evaluator
    pub fn new() -> Self {
        Self
    }
}

impl Default for WebhookEntryEvaluator {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl NodeEvaluator for WebhookEntryEvaluator {
    async fn evaluate(&self, node: &Node, input: &Value) -> Result<Option<Value>> {
        // Webhook entry nodes don't use configuration, just evaluate the payload
        let result = if let Some(bool_value) = node.payload.as_bool() {
            Value::Bool(bool_value)
        } else if node.payload.is_object() {
            let datalogic = create_datalogic();
            datalogic.evaluate_json(&node.payload, input, None)?
        } else {
            return Err(EngineError::InvalidNodeConfiguration {
                node_type: "webhook_entry".to_string(),
                details: "Payload must be a boolean or object".to_string(),
            }
            .into());
        };

        // Ensure the result is a boolean
        match result {
            Value::Bool(true) => Ok(Some(input.clone())),
            Value::Bool(false) => Ok(None),
            _ => Err(EngineError::NodeEvaluationFailed {
                node_type: "webhook_entry".to_string(),
                node_id: "webhook_entry".to_string(),
                details: format!("Node must evaluate to a boolean, got: {:?}", result),
            }
            .into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[tokio::test]
    async fn test_webhook_entry_simple_boolean() {
        let evaluator = WebhookEntryEvaluator::new();

        // Test with always true payload
        let node = Node {
            aturi: "test_webhook".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "webhook_entry".to_string(),
            payload: serde_json::json!(true),
            configuration: serde_json::json!({}),
            created_at: Utc::now(),
        };

        let input = serde_json::json!({
            "method": "POST",
            "path": "/webhook",
            "headers": {
                "content-type": "application/json"
            },
            "body": {
                "test": "data"
            }
        });

        let result = evaluator.evaluate(&node, &input).await.unwrap();
        assert_eq!(result, Some(input.clone()));

        // Test with always false payload
        let node_false = Node {
            aturi: "test_webhook".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "webhook_entry".to_string(),
            payload: serde_json::json!(false),
            configuration: serde_json::json!({}),
            created_at: Utc::now(),
        };

        let result = evaluator.evaluate(&node_false, &input).await.unwrap();
        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn test_webhook_entry_conditional_evaluation() {
        let evaluator = WebhookEntryEvaluator::new();

        // Test with conditional payload checking method
        let node = Node {
            aturi: "test_webhook".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "webhook_entry".to_string(),
            payload: serde_json::json!({
                "==": [
                    {"val": ["method"]},
                    "POST"
                ]
            }),
            configuration: serde_json::json!({}),
            created_at: Utc::now(),
        };

        // Test with matching method
        let input_post = serde_json::json!({
            "method": "POST",
            "path": "/webhook"
        });

        let result = evaluator.evaluate(&node, &input_post).await.unwrap();
        assert_eq!(result, Some(input_post.clone()));

        // Test with non-matching method
        let input_get = serde_json::json!({
            "method": "GET",
            "path": "/webhook"
        });

        let result = evaluator.evaluate(&node, &input_get).await.unwrap();
        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn test_webhook_entry_header_check() {
        let evaluator = WebhookEntryEvaluator::new();

        // Test checking for specific header value
        let node = Node {
            aturi: "test_webhook".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "webhook_entry".to_string(),
            payload: serde_json::json!({
                "==": [
                    {"val": ["headers", "x-webhook-type"]},
                    "user.created"
                ]
            }),
            configuration: serde_json::json!({}),
            created_at: Utc::now(),
        };

        // Test with matching header
        let input_match = serde_json::json!({
            "method": "POST",
            "headers": {
                "x-webhook-type": "user.created",
                "content-type": "application/json"
            }
        });

        let result = evaluator.evaluate(&node, &input_match).await.unwrap();
        assert_eq!(result, Some(input_match.clone()));

        // Test with non-matching header
        let input_no_match = serde_json::json!({
            "method": "POST",
            "headers": {
                "x-webhook-type": "user.deleted",
                "content-type": "application/json"
            }
        });

        let result = evaluator.evaluate(&node, &input_no_match).await.unwrap();
        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn test_webhook_entry_complex_condition() {
        let evaluator = WebhookEntryEvaluator::new();

        // Test with complex AND condition
        let node = Node {
            aturi: "test_webhook".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "webhook_entry".to_string(),
            payload: serde_json::json!({
                "and": [
                    {"==": [{"val": ["method"]}, "POST"]},
                    {"val": ["body", "active"]}
                ]
            }),
            configuration: serde_json::json!({}),
            created_at: Utc::now(),
        };

        // Test with both conditions true
        let input_both_true = serde_json::json!({
            "method": "POST",
            "body": {
                "active": true,
                "name": "test"
            }
        });

        let result = evaluator.evaluate(&node, &input_both_true).await.unwrap();
        assert_eq!(result, Some(input_both_true.clone()));

        // Test with one condition false
        let input_one_false = serde_json::json!({
            "method": "GET",
            "body": {
                "active": true,
                "name": "test"
            }
        });

        let result = evaluator.evaluate(&node, &input_one_false).await.unwrap();
        assert_eq!(result, None);

        // Test with both conditions false
        let input_both_false = serde_json::json!({
            "method": "GET",
            "body": {
                "active": false,
                "name": "test"
            }
        });

        let result = evaluator.evaluate(&node, &input_both_false).await.unwrap();
        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn test_webhook_entry_non_boolean_payload() {
        let evaluator = WebhookEntryEvaluator::new();

        // Test with payload that doesn't evaluate to boolean
        let node = Node {
            aturi: "test_webhook".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "webhook_entry".to_string(),
            payload: serde_json::json!({
                "val": ["method"]
            }),
            configuration: serde_json::json!({}),
            created_at: Utc::now(),
        };

        let input = serde_json::json!({
            "method": "POST"
        });

        let result = evaluator.evaluate(&node, &input).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("must evaluate to a boolean")
        );
    }

    #[tokio::test]
    async fn test_webhook_entry_invalid_payload_types() {
        let evaluator = WebhookEntryEvaluator::new();

        // Test with string payload (invalid)
        let node = Node {
            aturi: "test_webhook".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "webhook_entry".to_string(),
            payload: serde_json::json!("string payload"),
            configuration: serde_json::json!({}),
            created_at: Utc::now(),
        };

        let input = serde_json::json!({
            "method": "POST"
        });

        let result = evaluator.evaluate(&node, &input).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("invalid payload type")
        );

        // Test with number payload (invalid)
        let node = Node {
            aturi: "test_webhook".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "webhook_entry".to_string(),
            payload: serde_json::json!(123),
            configuration: serde_json::json!({}),
            created_at: Utc::now(),
        };

        let result = evaluator.evaluate(&node, &input).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("invalid payload type")
        );

        // Test with array payload (invalid)
        let node = Node {
            aturi: "test_webhook".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "webhook_entry".to_string(),
            payload: serde_json::json!([1, 2, 3]),
            configuration: serde_json::json!({}),
            created_at: Utc::now(),
        };

        let result = evaluator.evaluate(&node, &input).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("invalid payload type")
        );

        // Test with null payload (invalid)
        let node = Node {
            aturi: "test_webhook".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "webhook_entry".to_string(),
            payload: serde_json::json!(null),
            configuration: serde_json::json!({}),
            created_at: Utc::now(),
        };

        let result = evaluator.evaluate(&node, &input).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("invalid payload type")
        );
    }

    #[tokio::test]
    async fn test_webhook_entry_missing_expected_fields() {
        let evaluator = WebhookEntryEvaluator::new();

        // Test with payload expecting fields that don't exist in input
        let node = Node {
            aturi: "test_webhook".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "webhook_entry".to_string(),
            payload: serde_json::json!({
                "==": [
                    {"val": ["headers", "x-custom-header"]},
                    "expected-value"
                ]
            }),
            configuration: serde_json::json!({}),
            created_at: Utc::now(),
        };

        // Input missing the expected header
        let input = serde_json::json!({
            "method": "POST",
            "headers": {
                "content-type": "application/json"
            }
        });

        // Should evaluate to false (not match) when field is missing
        let result = evaluator.evaluate(&node, &input).await.unwrap();
        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn test_webhook_entry_nested_body_conditions() {
        let evaluator = WebhookEntryEvaluator::new();

        // Test with complex nested body structure
        let node = Node {
            aturi: "test_webhook".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "webhook_entry".to_string(),
            payload: serde_json::json!({
                "and": [
                    {"==": [{"val": ["body", "user", "status"]}, "active"]},
                    {">": [{"val": ["body", "user", "score"]}, 100]},
                    {"in": [{"val": ["body", "action"]}, ["create", "update"]]}
                ]
            }),
            configuration: serde_json::json!({}),
            created_at: Utc::now(),
        };

        // Test with all conditions met
        let input = serde_json::json!({
            "method": "POST",
            "body": {
                "user": {
                    "status": "active",
                    "score": 150
                },
                "action": "create"
            }
        });

        let result = evaluator.evaluate(&node, &input).await.unwrap();
        assert_eq!(result, Some(input.clone()));

        // Test with one condition not met (score too low)
        let input = serde_json::json!({
            "method": "POST",
            "body": {
                "user": {
                    "status": "active",
                    "score": 50
                },
                "action": "create"
            }
        });

        let result = evaluator.evaluate(&node, &input).await.unwrap();
        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn test_webhook_entry_or_conditions() {
        let evaluator = WebhookEntryEvaluator::new();

        // Test with OR condition
        let node = Node {
            aturi: "test_webhook".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "webhook_entry".to_string(),
            payload: serde_json::json!({
                "or": [
                    {"==": [{"val": ["method"]}, "POST"]},
                    {"==": [{"val": ["method"]}, "PUT"]},
                    {"==": [{"val": ["method"]}, "PATCH"]}
                ]
            }),
            configuration: serde_json::json!({}),
            created_at: Utc::now(),
        };

        // Test with POST (first condition matches)
        let input = serde_json::json!({
            "method": "POST",
            "path": "/webhook"
        });

        let result = evaluator.evaluate(&node, &input).await.unwrap();
        assert_eq!(result, Some(input.clone()));

        // Test with PUT (second condition matches)
        let input = serde_json::json!({
            "method": "PUT",
            "path": "/webhook"
        });

        let result = evaluator.evaluate(&node, &input).await.unwrap();
        assert_eq!(result, Some(input.clone()));

        // Test with GET (no condition matches)
        let input = serde_json::json!({
            "method": "GET",
            "path": "/webhook"
        });

        let result = evaluator.evaluate(&node, &input).await.unwrap();
        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn test_webhook_entry_auth_check() {
        let evaluator = WebhookEntryEvaluator::new();

        // Test checking for specific auth header pattern
        let node = Node {
            aturi: "test_webhook".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "webhook_entry".to_string(),
            payload: serde_json::json!({
                "and": [
                    {"!=": [{"val": ["headers", "authorization"]}, null]},
                    {"!=": [{"val": ["body", "signature"]}, null]}
                ]
            }),
            configuration: serde_json::json!({}),
            created_at: Utc::now(),
        };

        // Test with both fields present
        let input = serde_json::json!({
            "method": "POST",
            "headers": {
                "authorization": "Bearer token123",
                "content-type": "application/json"
            },
            "body": {
                "data": "test",
                "signature": "sig123"
            }
        });

        let result = evaluator.evaluate(&node, &input).await.unwrap();
        assert_eq!(result, Some(input.clone()));

        // Test with authorization missing
        let input = serde_json::json!({
            "method": "POST",
            "headers": {
                "content-type": "application/json"
            },
            "body": {
                "data": "test",
                "signature": "sig123"
            }
        });

        let result = evaluator.evaluate(&node, &input).await.unwrap();
        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn test_webhook_entry_query_params() {
        let evaluator = WebhookEntryEvaluator::new();

        // Test with query parameter conditions
        let node = Node {
            aturi: "test_webhook".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "webhook_entry".to_string(),
            payload: serde_json::json!({
                "and": [
                    {"==": [{"val": ["query", "token"]}, "secret123"]},
                    {"==": [{"val": ["query", "version"]}, "v2"]}
                ]
            }),
            configuration: serde_json::json!({}),
            created_at: Utc::now(),
        };

        // Test with matching query params
        let input = serde_json::json!({
            "method": "POST",
            "path": "/webhook",
            "query": {
                "token": "secret123",
                "version": "v2"
            },
            "body": {}
        });

        let result = evaluator.evaluate(&node, &input).await.unwrap();
        assert_eq!(result, Some(input.clone()));

        // Test with wrong token
        let input = serde_json::json!({
            "method": "POST",
            "path": "/webhook",
            "query": {
                "token": "wrong",
                "version": "v2"
            },
            "body": {}
        });

        let result = evaluator.evaluate(&node, &input).await.unwrap();
        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn test_webhook_entry_empty_input() {
        let evaluator = WebhookEntryEvaluator::new();

        // Test with empty input object
        let node = Node {
            aturi: "test_webhook".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "webhook_entry".to_string(),
            payload: serde_json::json!(true),
            configuration: serde_json::json!({}),
            created_at: Utc::now(),
        };

        let input = serde_json::json!({});

        let result = evaluator.evaluate(&node, &input).await.unwrap();
        assert_eq!(result, Some(input.clone()));

        // Test with condition that expects fields (should fail with empty input)
        let node = Node {
            aturi: "test_webhook".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "webhook_entry".to_string(),
            payload: serde_json::json!({
                "==": [{"val": ["method"]}, "POST"]
            }),
            configuration: serde_json::json!({}),
            created_at: Utc::now(),
        };

        let result = evaluator.evaluate(&node, &input).await.unwrap();
        assert_eq!(result, None);
    }
}
