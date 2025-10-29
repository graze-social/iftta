//! Condition node for conditional flow control in blueprints.
//!
//! This module implements conditional logic that allows blueprints to filter
//! data based on complex conditions. Condition nodes act as gates that either
//! allow data to pass through or stop processing.
//!
//! # Usage
//!
//! Condition nodes are typically placed after entry nodes or transforms to
//! filter data based on specific criteria. They use DataLogic expressions
//! to evaluate conditions against the input data.
//!
//! # Example Blueprint
//!
//! ```json
//! {
//!   "nodes": [
//!     {
//!       "type": "jetstream_entry",
//!       "configuration": {"collection": ["app.bsky.feed.post"]},
//!       "payload": true
//!     },
//!     {
//!       "type": "condition",
//!       "configuration": {},
//!       "payload": true  // Boolean payload allows all data through
//!     },
//!     {
//!       "type": "condition",
//!       "configuration": {},
//!       "payload": {
//!         "and": [
//!           {"contains": [{"val": ["commit", "record", "text"]}, "#atproto"]},
//!           {">": [{"len": {"val": ["commit", "record", "text"]}}, 50]}
//!         ]
//!       }
//!     },
//!     {
//!       "type": "publish_webhook",
//!       "configuration": {"url": "https://example.com/webhook"},
//!       "payload": {"val": []}
//!     }
//!   ]
//! }
//! ```

use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;

use crate::storage::node::Node;

use super::common::with_cached_datalogic;
use super::evaluator::NodeEvaluator;

/// Evaluator for condition nodes.
///
/// Condition nodes act as gates in the pipeline using the matching payload node pattern.
/// They evaluate the payload to determine whether data should continue through the pipeline.
///
/// # Payload Evaluation
///
/// The payload field determines whether the blueprint should proceed:
/// - Boolean `true`: Data continues through pipeline unchanged
/// - Boolean `false`: Pipeline stops (returns `None`)
/// - Object: Evaluated using DataLogic against input data, must return a boolean result
///
/// # Behavior
///
/// 1. If payload is boolean, uses that value directly
/// 2. If payload is object, evaluates using DataLogic against the input
/// 3. The result must be a boolean:
///    - `true`: Returns the input data unchanged (continues pipeline)
///    - `false`: Returns `None` to stop the pipeline
/// 4. Non-boolean results from DataLogic evaluation cause an error
///
/// # Configuration
///
/// The configuration field is not used and should be an empty JSON object:
/// ```json
/// {}
/// ```
///
/// # DataLogic Payload Examples
///
/// ## Simple equality check:
/// ```json
/// {"==": [{"val": ["status"]}, "active"]}
/// ```
///
/// ## Range check:
/// ```json
/// {"and": [
///   {">": [{"val": ["count"]}, 0]},
///   {"<=": [{"val": ["count"]}, 100]}
/// ]}
/// ```
///
/// ## Text contains check:
/// ```json
/// {"contains": [{"val": ["text"]}, "keyword"]}
/// ```
///
/// ## Array membership check:
/// ```json
/// {"in": [{"val": ["type"]}, ["post", "reply", "quote"]]}
/// ```
///
/// ## Complex nested condition:
/// ```json
/// {
///   "or": [
///     {"==": [{"val": ["priority"]}, "high"]},
///     {"and": [
///       {"==": [{"val": ["priority"]}, "medium"]},
///       {">": [{"val": ["score"]}, 75]}
///     ]}
///   ]
/// }
/// ```
///
/// ## Check for field existence:
/// ```json
/// {"exists": [{"val": ["metadata", "tags"]}]}
/// ```
///
/// ## Pattern matching:
/// ```json
/// {"matches": [{"val": ["email"]}, "^[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\\.[a-zA-Z]{2,}$"]}
/// ```
///
/// # Example Flow
///
/// Input:
/// ```json
/// {"status": "active", "count": 5}
/// ```
///
/// Condition payload:
/// ```json
/// {
///   "and": [
///     {"==": [{"val": ["status"]}, "active"]},
///     {">": [{"val": ["count"]}, 0]}
///   ]
/// }
/// ```
///
/// Result: Since both conditions are true, the original input passes through unchanged
pub struct ConditionEvaluator;

impl ConditionEvaluator {
    /// Create a new condition evaluator
    pub fn new() -> Self {
        Self
    }
}

impl Default for ConditionEvaluator {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl NodeEvaluator for ConditionEvaluator {
    async fn evaluate(&self, node: &Node, input: &Value) -> Result<Option<Value>> {
        let result = if let Some(bool_value) = node.payload.as_bool() {
            Value::Bool(bool_value)
        } else if node.payload.is_object() {
            with_cached_datalogic(|datalogic| {
                datalogic.evaluate_json(&node.payload, input, None)
            })?
        } else {
            return Err(anyhow::anyhow!("invalid payload type"));
        };

        // Ensure the result is a boolean
        match result {
            Value::Bool(true) => Ok(Some(input.clone())),
            Value::Bool(false) => Ok(None),
            _ => Err(anyhow::anyhow!(
                "Node must evaluate to a boolean, got: {:?}",
                result
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[tokio::test]
    async fn test_condition_evaluator_pass() {
        let evaluator = ConditionEvaluator::new();

        let node = Node {
            aturi: "test_node".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "condition".to_string(),
            payload: serde_json::json!({
                "==": [
                    {"val": ["status"]},
                    "active"
                ]
            }),
            configuration: serde_json::json!({}),
            created_at: Utc::now(),
        };

        let input = serde_json::json!({
            "status": "active",
            "data": "preserved"
        });

        let result = evaluator.evaluate(&node, &input).await.unwrap();
        assert!(result.is_some());

        // When condition passes, input data is returned unchanged
        assert_eq!(result.unwrap(), input);
    }

    #[tokio::test]
    async fn test_condition_evaluator_fail() {
        let evaluator = ConditionEvaluator::new();

        let node = Node {
            aturi: "test_node".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "condition".to_string(),
            payload: serde_json::json!({
                "==": [
                    {"val": ["status"]},
                    "active"
                ]
            }),
            configuration: serde_json::json!({}),
            created_at: Utc::now(),
        };

        let input = serde_json::json!({
            "status": "inactive",
            "data": "should_not_pass"
        });

        let result = evaluator.evaluate(&node, &input).await.unwrap();
        // When condition fails, returns None to stop processing
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_condition_complex_expression() {
        let evaluator = ConditionEvaluator::new();

        let node = Node {
            aturi: "test_node".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "condition".to_string(),
            payload: serde_json::json!({
                "and": [
                    {"==": [{"val": ["type"]}, "post"]},
                    {">": [{"val": ["score"]}, 10]},
                    {"or": [
                        {"==": [{"val": ["author"]}, "alice"]},
                        {"==": [{"val": ["author"]}, "bob"]}
                    ]}
                ]
            }),
            configuration: serde_json::json!({}),
            created_at: Utc::now(),
        };

        // Test passing complex condition
        let input = serde_json::json!({
            "type": "post",
            "score": 15,
            "author": "alice",
            "content": "test"
        });

        let result = evaluator.evaluate(&node, &input).await.unwrap();
        assert_eq!(result.unwrap(), input);

        // Test failing complex condition (score too low)
        let input = serde_json::json!({
            "type": "post",
            "score": 5,
            "author": "alice"
        });

        let result = evaluator.evaluate(&node, &input).await.unwrap();
        // When condition fails, returns None to stop processing
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_condition_non_boolean_error() {
        let evaluator = ConditionEvaluator::new();

        let node = Node {
            aturi: "test_node".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "condition".to_string(),
            // This will evaluate to a string, not a boolean
            payload: serde_json::json!({
                "val": ["status"]
            }),
            configuration: serde_json::json!({}),
            created_at: Utc::now(),
        };

        let input = serde_json::json!({
            "status": "active"
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
    async fn test_condition_boolean_payloads() {
        let evaluator = ConditionEvaluator::new();
        let input = serde_json::json!({"status": "test", "data": "preserved"});

        // Test with boolean true payload
        let node_true = Node {
            aturi: "test_node".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "condition".to_string(),
            payload: serde_json::json!(true),
            configuration: serde_json::json!({}),
            created_at: Utc::now(),
        };

        let result = evaluator.evaluate(&node_true, &input).await.unwrap();
        assert_eq!(result, Some(input.clone()));

        // Test with boolean false payload
        let node_false = Node {
            aturi: "test_node".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "condition".to_string(),
            payload: serde_json::json!(false),
            configuration: serde_json::json!({}),
            created_at: Utc::now(),
        };

        let result = evaluator.evaluate(&node_false, &input).await.unwrap();
        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn test_condition_invalid_payload_types() {
        let evaluator = ConditionEvaluator::new();
        let input = serde_json::json!({"status": "test"});

        // Test with string payload
        let node_string = Node {
            aturi: "test_node".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "condition".to_string(),
            payload: serde_json::json!("invalid"),
            configuration: serde_json::json!({}),
            created_at: Utc::now(),
        };

        let result = evaluator.evaluate(&node_string, &input).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("invalid payload type")
        );

        // Test with array payload
        let node_array = Node {
            aturi: "test_node".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "condition".to_string(),
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
                .contains("invalid payload type")
        );

        // Test with number payload
        let node_number = Node {
            aturi: "test_node".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "condition".to_string(),
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
                .contains("invalid payload type")
        );
    }

    #[tokio::test]
    async fn test_condition_empty_input() {
        let evaluator = ConditionEvaluator::new();

        // Test with boolean true payload and empty input
        let node = Node {
            aturi: "test_node".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "condition".to_string(),
            payload: serde_json::json!(true),
            configuration: serde_json::json!({}),
            created_at: Utc::now(),
        };

        let empty_input = serde_json::json!({});
        let result = evaluator.evaluate(&node, &empty_input).await.unwrap();
        assert_eq!(result, Some(empty_input.clone()));

        // Test with conditional payload that checks non-existent field
        let node_conditional = Node {
            aturi: "test_node".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "condition".to_string(),
            payload: serde_json::json!({
                "!=": [{"val": ["status"]}, null]
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
    async fn test_condition_missing_expected_fields() {
        let evaluator = ConditionEvaluator::new();

        // Test checking for required fields when they're missing
        let node = Node {
            aturi: "test_node".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "condition".to_string(),
            payload: serde_json::json!({
                "and": [
                    {"!=": [{"val": ["user", "id"]}, null]},
                    {"!=": [{"val": ["metadata", "timestamp"]}, null]}
                ]
            }),
            configuration: serde_json::json!({}),
            created_at: Utc::now(),
        };

        // Input missing required fields
        let incomplete_input = serde_json::json!({
            "user": {
                "name": "test"
                // Missing id
            },
            "data": "test"
            // Missing metadata
        });

        let result = evaluator.evaluate(&node, &incomplete_input).await.unwrap();
        assert_eq!(result, None);

        // Input with all required fields
        let complete_input = serde_json::json!({
            "user": {
                "id": 123,
                "name": "test"
            },
            "metadata": {
                "timestamp": "2023-01-01T00:00:00Z"
            },
            "data": "test"
        });

        let result = evaluator.evaluate(&node, &complete_input).await.unwrap();
        assert_eq!(result, Some(complete_input));
    }

    #[tokio::test]
    async fn test_condition_or_conditions() {
        let evaluator = ConditionEvaluator::new();

        // Test with OR condition - accept different status values
        let node = Node {
            aturi: "test_node".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "condition".to_string(),
            payload: serde_json::json!({
                "or": [
                    {"==": [{"val": ["status"]}, "active"]},
                    {"==": [{"val": ["status"]}, "pending"]},
                    {"==": [{"val": ["priority"]}, "high"]}
                ]
            }),
            configuration: serde_json::json!({}),
            created_at: Utc::now(),
        };

        // Test with first condition matching
        let input1 = serde_json::json!({
            "status": "active",
            "data": "test"
        });

        let result = evaluator.evaluate(&node, &input1).await.unwrap();
        assert_eq!(result, Some(input1));

        // Test with second condition matching
        let input2 = serde_json::json!({
            "status": "pending",
            "data": "test"
        });

        let result = evaluator.evaluate(&node, &input2).await.unwrap();
        assert_eq!(result, Some(input2));

        // Test with third condition matching
        let input3 = serde_json::json!({
            "status": "inactive",
            "priority": "high",
            "data": "test"
        });

        let result = evaluator.evaluate(&node, &input3).await.unwrap();
        assert_eq!(result, Some(input3));

        // Test with no conditions matching
        let input_no_match = serde_json::json!({
            "status": "inactive",
            "priority": "low",
            "data": "test"
        });

        let result = evaluator.evaluate(&node, &input_no_match).await.unwrap();
        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn test_condition_nested_object_conditions() {
        let evaluator = ConditionEvaluator::new();

        // Test deeply nested object structure evaluation
        let node = Node {
            aturi: "test_node".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "condition".to_string(),
            payload: serde_json::json!({
                "and": [
                    {"==": [{"val": ["user", "profile", "tier"]}, "premium"]},
                    {">=": [{"val": ["user", "stats", "score"]}, 100]},
                    {"in": [{"val": ["action", "type"]}, ["create", "update", "publish"]]}
                ]
            }),
            configuration: serde_json::json!({}),
            created_at: Utc::now(),
        };

        // Test with all nested conditions met
        let input_success = serde_json::json!({
            "user": {
                "id": 123,
                "profile": {
                    "tier": "premium",
                    "status": "active"
                },
                "stats": {
                    "score": 150,
                    "level": 5
                }
            },
            "action": {
                "type": "create",
                "timestamp": "2023-01-01T00:00:00Z"
            }
        });

        let result = evaluator.evaluate(&node, &input_success).await.unwrap();
        assert_eq!(result, Some(input_success));

        // Test with one nested condition failing
        let input_fail = serde_json::json!({
            "user": {
                "id": 123,
                "profile": {
                    "tier": "basic", // Wrong tier
                    "status": "active"
                },
                "stats": {
                    "score": 150
                }
            },
            "action": {
                "type": "create"
            }
        });

        let result = evaluator.evaluate(&node, &input_fail).await.unwrap();
        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn test_condition_numeric_range_checks() {
        let evaluator = ConditionEvaluator::new();

        // Test range validation
        let node = Node {
            aturi: "test_node".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "condition".to_string(),
            payload: serde_json::json!({
                "and": [
                    {">=": [{"val": ["score"]}, 0]},
                    {"<=": [{"val": ["score"]}, 100]},
                    {">": [{"val": ["attempts"]}, 0]}
                ]
            }),
            configuration: serde_json::json!({}),
            created_at: Utc::now(),
        };

        // Test with valid range
        let input_valid = serde_json::json!({
            "score": 85,
            "attempts": 3,
            "user": "test"
        });

        let result = evaluator.evaluate(&node, &input_valid).await.unwrap();
        assert_eq!(result, Some(input_valid));

        // Test with score out of range (too high)
        let input_high = serde_json::json!({
            "score": 150,
            "attempts": 3
        });

        let result = evaluator.evaluate(&node, &input_high).await.unwrap();
        assert_eq!(result, None);

        // Test with zero attempts (fails > 0 check)
        let input_zero = serde_json::json!({
            "score": 85,
            "attempts": 0
        });

        let result = evaluator.evaluate(&node, &input_zero).await.unwrap();
        assert_eq!(result, None);
    }
}
