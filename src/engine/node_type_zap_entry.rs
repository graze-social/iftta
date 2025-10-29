//! Zap entry node for processing Zapier integration requests.
//!
//! This module implements the entry point for blueprints that process incoming
//! Zapier webhook requests. Zap entry nodes are specifically designed for
//! Zapier integrations and provide a dedicated endpoint for Zapier actions.
//!
//! # Usage
//!
//! Zap entry nodes must be the first node in a blueprint. They filter
//! incoming Zapier requests based on DataLogic expressions evaluated against
//! the request data.
//!
//! # Zap Request Structure
//!
//! Zapier requests are passed to the node with the following structure:
//! ```json
//! {
//!   "headers": {
//!     "content-type": "application/json",
//!     "x-zapier-trigger": "...",
//!     "x-zapier-user": "..."
//!   },
//!   "body": {
//!     // The parsed JSON body from Zapier
//!   },
//!   "query": {
//!     // Query parameters from the URL
//!   },
//!   "method": "POST",
//!   "path": "/api/zapier/action"
//! }
//! ```
//!
//! # Example Blueprint
//!
//! ```json
//! {
//!   "nodes": [
//!     {
//!       "type": "zap_entry",
//!       "configuration": {},
//!       "payload": true  // Boolean payload allows all Zapier requests
//!     },
//!     {
//!       "type": "zap_entry",
//!       "configuration": {},
//!       "payload": {
//!         "and": [
//!           {"==": [{"val": ["headers", "content-type"]}, "application/json"]},
//!           {"!=": [{"val": ["body", "trigger_id"]}, null]}
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

use super::common::evaluate_json_logic;
use super::evaluator::NodeEvaluator;

/// Evaluator for Zap entry nodes.
///
/// This evaluator handles Zapier inbound triggers by evaluating the payload
/// as a boolean expression using DataLogic to determine whether to continue
/// blueprint processing.
///
/// # Configuration
///
/// The configuration field is not used and should be an empty JSON object:
/// ```json
/// {}
/// ```
///
/// # Payload Evaluation
///
/// The payload field determines whether the blueprint should proceed:
/// - Boolean `true`: Blueprint continues processing with all Zapier requests
/// - Boolean `false`: Blueprint stops processing (rejects all requests)
/// - Object: Evaluated using DataLogic against the request data, must return a boolean result
///
/// ## Payload Examples
///
/// Accept all Zapier requests:
/// ```json
/// true
/// ```
///
/// Filter by Zapier trigger type:
/// ```json
/// {
///   "==": [
///     {"val": ["headers", "x-zapier-trigger"]},
///     "new_post"
///   ]
/// }
/// ```
///
/// Filter by Zapier user:
/// ```json
/// {
///   "exists": [{"val": ["headers", "x-zapier-user"]}]
/// }
/// ```
///
/// Filter by specific action in body:
/// ```json
/// {
///   "==": [
///     {"val": ["body", "action_type"]},
///     "create_post"
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
///     {"exists": [{"val": ["body", "zapier_meta", "account_id"]}]}
///   ]
/// }
/// ```
///
/// # Behavior
///
/// 1. The evaluator receives Zapier input data
/// 2. If payload is boolean, uses that value directly
/// 3. If payload is object, evaluates using DataLogic against the input
/// 4. The result must be a boolean:
///    - `true`: Continue blueprint evaluation (returns `Some(input)`)
///    - `false`: Stop blueprint evaluation (returns `None`)
/// 5. Non-boolean results from DataLogic evaluation cause an error
///
/// # Input Data
///
/// The Zapier input data typically contains:
/// - `method`: HTTP method (usually POST)
/// - `headers`: HTTP headers including Zapier-specific headers
/// - `body`: Request body with Zapier action data
/// - `query`: Query parameters
/// - `path`: Request path (/api/zapier/action)
/// - Any additional Zapier-specific fields
pub struct ZapEntryEvaluator;

impl ZapEntryEvaluator {
    /// Create a new Zap entry evaluator
    pub fn new() -> Self {
        Self
    }
}

impl Default for ZapEntryEvaluator {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl NodeEvaluator for ZapEntryEvaluator {
    async fn evaluate(&self, node: &Node, input: &Value) -> Result<Option<Value>> {
        // Zap entry nodes don't use configuration, just evaluate the payload
        let result = if let Some(bool_value) = node.payload.as_bool() {
            Value::Bool(bool_value)
        } else if node.payload.is_object() {
            evaluate_json_logic(false, &node.payload, input)?
        } else {
            return Err(EngineError::InvalidNodeConfiguration {
                node_type: "zap_entry".to_string(),
                details: "Payload must be a boolean or object".to_string(),
            }
            .into());
        };

        // Ensure the result is a boolean
        match result {
            Value::Bool(true) => Ok(Some(input.clone())),
            Value::Bool(false) => Ok(None),
            _ => Err(EngineError::NodeEvaluationFailed {
                node_type: "zap_entry".to_string(),
                node_id: "zap_entry".to_string(),
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
    async fn test_zap_entry_simple_boolean() {
        let evaluator = ZapEntryEvaluator::new();

        // Test with always true payload
        let node = Node {
            aturi: "test_zap".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "zap_entry".to_string(),
            payload: serde_json::json!(true),
            configuration: serde_json::json!({}),
            created_at: Utc::now(),
        };

        let input = serde_json::json!({
            "method": "POST",
            "path": "/api/zapier/action",
            "headers": {
                "content-type": "application/json",
                "x-zapier-trigger": "new_post"
            },
            "body": {
                "action": "create",
                "data": {"text": "Test post"}
            }
        });

        let result = evaluator.evaluate(&node, &input).await.unwrap();
        assert_eq!(result, Some(input.clone()));

        // Test with always false payload
        let node_false = Node {
            aturi: "test_zap".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "zap_entry".to_string(),
            payload: serde_json::json!(false),
            configuration: serde_json::json!({}),
            created_at: Utc::now(),
        };

        let result = evaluator.evaluate(&node_false, &input).await.unwrap();
        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn test_zap_entry_zapier_header_check() {
        let evaluator = ZapEntryEvaluator::new();

        // Test checking for specific Zapier trigger
        let node = Node {
            aturi: "test_zap".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "zap_entry".to_string(),
            payload: serde_json::json!({
                "==": [
                    {"val": ["headers", "x-zapier-trigger"]},
                    "new_post"
                ]
            }),
            configuration: serde_json::json!({}),
            created_at: Utc::now(),
        };

        // Test with matching trigger
        let input_match = serde_json::json!({
            "method": "POST",
            "path": "/api/zapier/action",
            "headers": {
                "x-zapier-trigger": "new_post",
                "content-type": "application/json"
            }
        });

        let result = evaluator.evaluate(&node, &input_match).await.unwrap();
        assert_eq!(result, Some(input_match.clone()));

        // Test with non-matching trigger
        let input_no_match = serde_json::json!({
            "method": "POST",
            "path": "/api/zapier/action",
            "headers": {
                "x-zapier-trigger": "update_post",
                "content-type": "application/json"
            }
        });

        let result = evaluator.evaluate(&node, &input_no_match).await.unwrap();
        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn test_zap_entry_complex_condition() {
        let evaluator = ZapEntryEvaluator::new();

        // Test with complex AND condition
        let node = Node {
            aturi: "test_zap".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "zap_entry".to_string(),
            payload: serde_json::json!({
                "and": [
                    {"==": [{"val": ["method"]}, "POST"]},
                    {"!=": [{"val": ["body", "zapier_meta", "account_id"]}, null]}
                ]
            }),
            configuration: serde_json::json!({}),
            created_at: Utc::now(),
        };

        // Test with both conditions true
        let input_both_true = serde_json::json!({
            "method": "POST",
            "body": {
                "zapier_meta": {
                    "account_id": "123456",
                    "zap_id": "zap_789"
                },
                "data": {"text": "Test"}
            }
        });

        let result = evaluator.evaluate(&node, &input_both_true).await.unwrap();
        assert_eq!(result, Some(input_both_true.clone()));

        // Test with one condition false
        let input_one_false = serde_json::json!({
            "method": "GET",
            "body": {
                "zapier_meta": {
                    "account_id": "123456"
                }
            }
        });

        let result = evaluator.evaluate(&node, &input_one_false).await.unwrap();
        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn test_zap_entry_non_boolean_payload() {
        let evaluator = ZapEntryEvaluator::new();

        // Test with payload that doesn't evaluate to boolean
        let node = Node {
            aturi: "test_zap".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "zap_entry".to_string(),
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
    async fn test_zap_entry_invalid_payload_types() {
        let evaluator = ZapEntryEvaluator::new();
        let input = serde_json::json!({"method": "POST"});

        // Test with string payload
        let node_string = Node {
            aturi: "test_zap".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "zap_entry".to_string(),
            payload: serde_json::json!("invalid"),
            configuration: serde_json::json!({}),
            created_at: Utc::now(),
        };

        let result = evaluator.evaluate(&node_string, &input).await;
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("Payload must be a boolean or object"));

        // Test with array payload
        let node_array = Node {
            aturi: "test_zap".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "zap_entry".to_string(),
            payload: serde_json::json!([1, 2, 3]),
            configuration: serde_json::json!({}),
            created_at: Utc::now(),
        };

        let result = evaluator.evaluate(&node_array, &input).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Payload must be a boolean or object")
        );

        // Test with number payload
        let node_number = Node {
            aturi: "test_zap".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "zap_entry".to_string(),
            payload: serde_json::json!(42),
            configuration: serde_json::json!({}),
            created_at: Utc::now(),
        };

        let result = evaluator.evaluate(&node_number, &input).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Payload must be a boolean or object")
        );
    }

    #[tokio::test]
    async fn test_zap_entry_empty_input() {
        let evaluator = ZapEntryEvaluator::new();

        // Test with boolean true payload and empty input
        let node = Node {
            aturi: "test_zap".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "zap_entry".to_string(),
            payload: serde_json::json!(true),
            configuration: serde_json::json!({}),
            created_at: Utc::now(),
        };

        let empty_input = serde_json::json!({});
        let result = evaluator.evaluate(&node, &empty_input).await.unwrap();
        assert_eq!(result, Some(empty_input.clone()));

        // Test with conditional payload that checks non-existent field
        let node_conditional = Node {
            aturi: "test_zap".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "zap_entry".to_string(),
            payload: serde_json::json!({
                "!=": [{"val": ["headers", "x-zapier-user"]}, null]
            }),
            configuration: serde_json::json!({}),
            created_at: Utc::now(),
        };

        let result = evaluator
            .evaluate(&node_conditional, &empty_input)
            .await
            .unwrap();
        assert_eq!(result, None); // Should be false since field doesn't exist
    }

    #[tokio::test]
    async fn test_zap_entry_missing_expected_fields() {
        let evaluator = ZapEntryEvaluator::new();

        // Test checking for required Zapier headers when they're missing
        let node = Node {
            aturi: "test_zap".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "zap_entry".to_string(),
            payload: serde_json::json!({
                "and": [
                    {"!=": [{"val": ["headers", "x-zapier-trigger"]}, null]},
                    {"!=": [{"val": ["body", "zapier_meta", "zap_id"]}, null]}
                ]
            }),
            configuration: serde_json::json!({}),
            created_at: Utc::now(),
        };

        // Input missing required Zapier fields
        let incomplete_input = serde_json::json!({
            "method": "POST",
            "headers": {
                "content-type": "application/json"
                // Missing x-zapier-trigger
            },
            "body": {
                "data": "test"
                // Missing zapier_meta
            }
        });

        let result = evaluator.evaluate(&node, &incomplete_input).await.unwrap();
        assert_eq!(result, None);

        // Input with all required fields
        let complete_input = serde_json::json!({
            "method": "POST",
            "headers": {
                "content-type": "application/json",
                "x-zapier-trigger": "new_record"
            },
            "body": {
                "zapier_meta": {
                    "zap_id": "123"
                },
                "data": "test"
            }
        });

        let result = evaluator.evaluate(&node, &complete_input).await.unwrap();
        assert_eq!(result, Some(complete_input));
    }

    #[tokio::test]
    async fn test_zap_entry_or_conditions() {
        let evaluator = ZapEntryEvaluator::new();

        // Test with OR condition - accept different trigger types
        let node = Node {
            aturi: "test_zap".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "zap_entry".to_string(),
            payload: serde_json::json!({
                "or": [
                    {"==": [{"val": ["headers", "x-zapier-trigger"]}, "new_record"]},
                    {"==": [{"val": ["headers", "x-zapier-trigger"]}, "updated_record"]},
                    {"==": [{"val": ["body", "action"]}, "test_action"]}
                ]
            }),
            configuration: serde_json::json!({}),
            created_at: Utc::now(),
        };

        // Test with first condition matching
        let input1 = serde_json::json!({
            "headers": {
                "x-zapier-trigger": "new_record"
            }
        });

        let result = evaluator.evaluate(&node, &input1).await.unwrap();
        assert_eq!(result, Some(input1));

        // Test with second condition matching
        let input2 = serde_json::json!({
            "headers": {
                "x-zapier-trigger": "updated_record"
            }
        });

        let result = evaluator.evaluate(&node, &input2).await.unwrap();
        assert_eq!(result, Some(input2));

        // Test with third condition matching
        let input3 = serde_json::json!({
            "headers": {
                "x-zapier-trigger": "other_trigger"
            },
            "body": {
                "action": "test_action"
            }
        });

        let result = evaluator.evaluate(&node, &input3).await.unwrap();
        assert_eq!(result, Some(input3));

        // Test with no conditions matching
        let input_no_match = serde_json::json!({
            "headers": {
                "x-zapier-trigger": "unmatched_trigger"
            },
            "body": {
                "action": "other_action"
            }
        });

        let result = evaluator.evaluate(&node, &input_no_match).await.unwrap();
        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn test_zap_entry_nested_body_conditions() {
        let evaluator = ZapEntryEvaluator::new();

        // Test deeply nested body structure evaluation
        let node = Node {
            aturi: "test_zap".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "zap_entry".to_string(),
            payload: serde_json::json!({
                "and": [
                    {"==": [{"val": ["body", "zapier_meta", "subscription", "plan"]}, "premium"]},
                    {">=": [{"val": ["body", "data", "user", "level"]}, 10]},
                    {"in": [{"val": ["body", "action_type"]}, ["create", "update", "publish"]]}
                ]
            }),
            configuration: serde_json::json!({}),
            created_at: Utc::now(),
        };

        // Test with all nested conditions met
        let input_success = serde_json::json!({
            "method": "POST",
            "body": {
                "zapier_meta": {
                    "subscription": {
                        "plan": "premium",
                        "status": "active"
                    }
                },
                "data": {
                    "user": {
                        "id": 123,
                        "level": 15,
                        "name": "Test User"
                    }
                },
                "action_type": "create"
            }
        });

        let result = evaluator.evaluate(&node, &input_success).await.unwrap();
        assert_eq!(result, Some(input_success));

        // Test with one nested condition failing
        let input_fail = serde_json::json!({
            "method": "POST",
            "body": {
                "zapier_meta": {
                    "subscription": {
                        "plan": "basic", // Wrong plan
                        "status": "active"
                    }
                },
                "data": {
                    "user": {
                        "id": 123,
                        "level": 15
                    }
                },
                "action_type": "create"
            }
        });

        let result = evaluator.evaluate(&node, &input_fail).await.unwrap();
        assert_eq!(result, None);
    }
}
