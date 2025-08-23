//! Transform node for data transformation and manipulation.
//!
//! This module implements data transformation capabilities that allow blueprints
//! to reshape, extract, and modify data as it flows through the pipeline.
//! Transform nodes use DataLogic expressions to create new data structures
//! from input data.
//!
//! # Important Requirements
//!
//! - The payload field MUST be either:
//!   - An object containing the DataLogic template
//!   - An array of objects, each containing a DataLogic template (for chained transformations)
//! - The configuration field MUST be an object (currently empty `{}`)
//! - The evaluation result MUST be an object that replaces the input
//!
//! # Usage
//!
//! Transform nodes are placed in the pipeline to prepare data for action nodes
//! or to extract specific fields from complex structures. They're essential
//! for adapting data formats between different systems.
//!
//! # Example Blueprints
//!
//! ## Single transformation (object payload):
//! ```json
//! {
//!   "nodes": [
//!     {
//!       "type": "jetstream_entry",
//!       "configuration": {"collection": ["app.bsky.feed.post"]},
//!       "payload": true
//!     },
//!     {
//!       "type": "transform",
//!       "configuration": {},
//!       "payload": {
//!         "record": {
//!           "$type": "app.bsky.feed.like",
//!           "subject": {
//!             "uri": {"cat": ["at://", {"val": ["did"]}, "/app.bsky.feed.post/", {"val": ["commit", "rkey"]}]},
//!             "cid": {"val": ["commit", "cid"]}
//!           },
//!           "createdAt": {"now": []}
//!         }
//!       }
//!     },
//!     {
//!       "type": "publish_record",
//!       "configuration": {"did": "did:plc:...", "collection": "app.bsky.feed.like"},
//!       "payload": {"val": []}
//!     }
//!   ]
//! }
//! ```
//!
//! ## Chained transformations (array payload):
//! ```json
//! {
//!   "nodes": [
//!     {
//!       "type": "webhook_entry",
//!       "configuration": {},
//!       "payload": true
//!     },
//!     {
//!       "type": "transform",
//!       "configuration": {},
//!       "payload": [
//!         {
//!           "user": {"val": ["user"]},
//!           "is_premium": {"==": [{"val": ["user", "tier"]}, "premium"]},
//!           "timestamp": {"now": []}
//!         },
//!         {
//!           "message": {"cat": ["Welcome ", {"val": ["user", "name"]}, "!"]},
//!           "access_level": {"if": [{"val": ["is_premium"]}, "full", "basic"]},
//!           "processed_at": {"val": ["timestamp"]}
//!         }
//!       ]
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

use super::common::create_datalogic;
use super::evaluator::NodeEvaluator;

/// Evaluator for transform nodes.
///
/// Transform nodes take input data, apply a DataLogic template transformation,
/// and output the transformed result. The output replaces the input data
/// for subsequent nodes in the pipeline.
///
/// # Validation Rules
///
/// - The node's payload MUST be an object (not boolean, string, or other types)
/// - The configuration MUST be an object (currently empty)
/// - The DataLogic evaluation result MUST be an object
///
/// # Configuration
///
/// The configuration field MUST be an empty JSON object (required but unused):
/// ```json
/// {}
/// ```
///
/// # Payload Structure
///
/// The payload can be one of two formats:
///
/// 1. **Object Format**: A JSON object containing DataLogic expressions that
///    evaluate to produce a new object.
///
/// 2. **Array Format**: An array of JSON objects, where each object contains
///    DataLogic expressions. The transformations are applied sequentially,
///    with the output of each transformation becoming the input for the next.
///
/// ## Extract specific fields:
/// ```json
/// {
///   "id": {"val": ["user", "id"]},
///   "name": {"val": ["user", "profile", "displayName"]},
///   "timestamp": {"val": ["createdAt"]}
/// }
/// ```
///
/// ## Concatenate strings:
/// ```json
/// {
///   "message": {
///     "cat": [
///       "User ",
///       {"val": ["username"]},
///       " posted: ",
///       {"val": ["text"]}
///     ]
///   }
/// }
/// ```
///
/// ## Conditional transformation:
/// ```json
/// {
///   "status": {
///     "if": [
///       {">": [{"val": ["score"]}, 100]},
///       "excellent",
///       {"if": [
///         {">": [{"val": ["score"]}, 50]},
///         "good",
///         "needs improvement"
///       ]}
///     ]
///   }
/// }
/// ```
///
/// ## Build AT Protocol record for publish_record:
/// ```json
/// {
///   "record": {
///     "$type": "app.bsky.feed.post",
///     "text": {"val": ["content"]},
///     "createdAt": {"now": []},
///     "langs": ["en-US"]
///   }
/// }
/// ```
/// Note: The record must be wrapped in a "record" field for publish_record nodes.
///
/// ## Array operations:
/// ```json
/// {
///   "items": {
///     "map": [
///       {"val": ["products"]},
///       {
///         "name": {"val": ["title"]},
///         "price_with_tax": {"*": [{"val": ["price"]}, 1.1]}
///       }
///     ]
///   },
///   "total": {"sum": {"val": ["products", "*", "price"]}}
/// }
/// ```
///
/// ## Date/time operations:
/// ```json
/// {
///   "timestamp": {"now": []},
///   "expiry": {"add_days": [{"now": []}, 30]},
///   "formatted_date": {"format_date": [{"val": ["created_at"]}, "YYYY-MM-DD"]}
/// }
/// ```
///
/// ## Chained transformations (array format):
/// ```json
/// [
///   {
///     "user_greeting": {"cat": ["Hello, ", {"val": ["name"]}]},
///     "is_adult": {">=": [{"val": ["age"]}, 18]},
///     "location": {"val": ["city"]}
///   },
///   {
///     "final_message": {"val": ["user_greeting"]},
///     "can_access": {"and": [{"val": ["is_adult"]}, {"==": [{"val": ["location"]}, "New York"]}]}
///   }
/// ]
/// ```
/// First transformation creates intermediate fields, second transformation uses those to create final output.
///
/// # Example Flow
///
/// Input:
/// ```json
/// {"name": "Alice", "age": 25, "city": "New York"}
/// ```
///
/// Transform payload (MUST be an object):
/// ```json
/// {
///   "greeting": {"cat": ["Hello, ", {"val": ["name"]}, " from ", {"val": ["city"]}]},
///   "can_vote": {">=": [{"val": ["age"]}, 18]}
/// }
/// ```
///
/// Output (MUST be an object that replaces input):
/// ```json
/// {"greeting": "Hello, Alice from New York", "can_vote": true}
/// ```
pub struct TransformEvaluator;

impl TransformEvaluator {
    /// Create a new transform evaluator
    pub fn new() -> Self {
        Self
    }
}

impl Default for TransformEvaluator {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl NodeEvaluator for TransformEvaluator {
    async fn evaluate(&self, node: &Node, input: &Value) -> Result<Option<Value>> {
        let datalogic = create_datalogic();

        // Check if payload is an array (chained transformations) or object (single transformation)
        if node.payload.is_array() {
            // Process array of transformations sequentially
            let transformations = node
                .payload
                .as_array()
                .ok_or_else(|| anyhow::anyhow!("Expected array payload"))?;

            if transformations.is_empty() {
                return Err(anyhow::anyhow!("Transform array payload cannot be empty"));
            }

            // Start with the input as the current data
            let mut current_data = input.clone();

            // Apply each transformation in sequence
            for (index, transformation) in transformations.iter().enumerate() {
                // Each element must be an object
                if !transformation.is_object() {
                    return Err(anyhow::anyhow!(
                        "Transform array element {} must be an object, got: {:?}",
                        index,
                        transformation
                    ));
                }

                // Apply the transformation
                let result = datalogic.evaluate_json(transformation, &current_data, None)?;

                // Ensure the result is an object
                if !result.is_object() {
                    return Err(anyhow::anyhow!(
                        "Transform array element {} must evaluate to an object, got: {:?}",
                        index,
                        result
                    ));
                }

                // Use the result as input for the next transformation
                current_data = result;
            }

            // Return the final transformed data
            Ok(Some(current_data))
        } else if node.payload.is_object() {
            // Process single object transformation (existing behavior)
            let transformed = datalogic.evaluate_json(&node.payload, input, None)?;

            // Ensure the result is an object
            if !transformed.is_object() {
                return Err(anyhow::anyhow!(
                    "Transform node must evaluate to an object, got: {:?}",
                    transformed
                ));
            }

            // Return the transformed data as the new output
            Ok(Some(transformed))
        } else {
            // Invalid payload type
            Err(anyhow::anyhow!(
                "Transform payload must be an object or array of objects, got: {:?}",
                node.payload
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use serde_json::json;

    #[tokio::test]
    async fn test_transform_evaluator() {
        let evaluator = TransformEvaluator::new();

        let node = Node {
            aturi: "test_node".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "transform".to_string(),
            payload: serde_json::json!({
                "result": "transformed",
                "input": {"val": ["input"]},
                "doubled": {"*": [{"val": ["number"]}, 2]}
            }),
            configuration: serde_json::json!({}),
            created_at: Utc::now(),
        };

        let input = serde_json::json!({
            "input": "data",
            "number": 5
        });

        let result = evaluator.evaluate(&node, &input).await.unwrap();
        assert!(result.is_some());

        let output = result.unwrap();
        assert_eq!(output["result"], "transformed");
        assert_eq!(output["input"], "data");
        assert_eq!(output["doubled"], 10);

        // Verify the output replaces the input (doesn't contain original "number" field)
        assert!(output.get("number").is_none());
    }

    #[tokio::test]
    async fn test_transform_with_complex_template() {
        let evaluator = TransformEvaluator::new();

        let node = Node {
            aturi: "test_node".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "transform".to_string(),
            payload: serde_json::json!({
                "$type": "app.bsky.feed.like",
                "createdAt": {"datetime": [{"now": []}]},
                "subject": {
                    "uri": {
                        "cat": [
                            "at://",
                            {"val": ["did"]},
                            "/app.bsky.feed.post/",
                            {"val": ["rkey"]}
                        ]
                    }
                }
            }),
            configuration: serde_json::json!({}),
            created_at: Utc::now(),
        };

        let input = serde_json::json!({
            "did": "did:plc:example",
            "rkey": "abc123"
        });

        let result = evaluator.evaluate(&node, &input).await.unwrap();
        assert!(result.is_some());

        let output = result.unwrap();
        assert_eq!(output["$type"], "app.bsky.feed.like");
        assert_eq!(
            output["subject"]["uri"],
            "at://did:plc:example/app.bsky.feed.post/abc123"
        );
        assert!(output["createdAt"].is_string());
    }

    #[tokio::test]
    async fn test_transform_invalid_payload_boolean() {
        let evaluator = TransformEvaluator::new();

        let node = Node {
            aturi: "test_node".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "transform".to_string(),
            payload: json!(true), // Invalid: boolean payload
            configuration: json!({}),
            created_at: Utc::now(),
        };

        let input = json!({"test": "data"});

        // This should fail because payload must be object or array
        let result = evaluator.evaluate(&node, &input).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("must be an object or array of objects")
        );
    }

    #[tokio::test]
    async fn test_transform_invalid_payload_string() {
        let evaluator = TransformEvaluator::new();

        let node = Node {
            aturi: "test_node".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "transform".to_string(),
            payload: json!("string payload"), // Invalid: string payload
            configuration: json!({}),
            created_at: Utc::now(),
        };

        let input = json!({"test": "data"});

        let result = evaluator.evaluate(&node, &input).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("must be an object or array of objects")
        );
    }

    #[tokio::test]
    async fn test_transform_invalid_payload_array_of_primitives() {
        let evaluator = TransformEvaluator::new();

        let node = Node {
            aturi: "test_node".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "transform".to_string(),
            payload: json!([1, 2, 3]), // Invalid: array of primitives instead of objects
            configuration: json!({}),
            created_at: Utc::now(),
        };

        let input = json!({"test": "data"});

        let result = evaluator.evaluate(&node, &input).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("element 0 must be an object")
        );
    }

    #[tokio::test]
    async fn test_transform_result_not_object() {
        let evaluator = TransformEvaluator::new();

        // Payload that evaluates to a non-object (string)
        let node = Node {
            aturi: "test_node".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "transform".to_string(),
            payload: json!({
                "val": ["test"] // This will return the string value of "test" field
            }),
            configuration: json!({}),
            created_at: Utc::now(),
        };

        let input = json!({"test": "string value"});

        let result = evaluator.evaluate(&node, &input).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("must evaluate to an object")
        );
    }

    #[tokio::test]
    async fn test_transform_result_is_array() {
        let evaluator = TransformEvaluator::new();

        // Test that payload creates an object with array field (valid)
        let node = Node {
            aturi: "test_node".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "transform".to_string(),
            payload: json!({
                "items": [1, 2, 3, 4, 5] // This creates an object with an array field
            }),
            configuration: json!({}),
            created_at: Utc::now(),
        };

        let input = json!({});

        let result = evaluator.evaluate(&node, &input).await;
        // The payload creates an object with an "items" field containing an array
        // This is valid since the result is still an object
        assert!(result.is_ok());
        let output = result.unwrap().unwrap();
        assert!(output.is_object());
        assert!(output["items"].is_array());
        assert_eq!(output["items"], json!([1, 2, 3, 4, 5]));
    }

    #[tokio::test]
    async fn test_transform_payload_evaluates_to_primitive() {
        let evaluator = TransformEvaluator::new();

        // Payload that evaluates directly to a primitive value (not an object)
        let node = Node {
            aturi: "test_node".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "transform".to_string(),
            payload: json!({
                "val": ["items"] // This extracts the array directly, not wrapped in object
            }),
            configuration: json!({}),
            created_at: Utc::now(),
        };

        let input = json!({"items": [1, 2, 3]});

        let result = evaluator.evaluate(&node, &input).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("must evaluate to an object")
        );
    }

    #[tokio::test]
    async fn test_transform_empty_object() {
        let evaluator = TransformEvaluator::new();

        let node = Node {
            aturi: "test_node".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "transform".to_string(),
            payload: json!({}), // Empty object is valid
            configuration: json!({}),
            created_at: Utc::now(),
        };

        let input = json!({"ignored": "data"});

        let result = evaluator.evaluate(&node, &input).await.unwrap();
        assert!(result.is_some());
        assert_eq!(result.unwrap(), json!({}));
    }

    #[tokio::test]
    async fn test_transform_nested_objects() {
        let evaluator = TransformEvaluator::new();

        let node = Node {
            aturi: "test_node".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "transform".to_string(),
            payload: json!({
                "user": {
                    "id": {"val": ["userId"]},
                    "profile": {
                        "name": {"val": ["userName"]},
                        "settings": {
                            "theme": "dark",
                            "notifications": true
                        }
                    }
                },
                "timestamp": {"now": []}
            }),
            configuration: json!({}),
            created_at: Utc::now(),
        };

        let input = json!({
            "userId": "user123",
            "userName": "Alice"
        });

        let result = evaluator.evaluate(&node, &input).await.unwrap();
        assert!(result.is_some());

        let output = result.unwrap();
        assert!(output.is_object());
        assert_eq!(output["user"]["id"], "user123");
        assert_eq!(output["user"]["profile"]["name"], "Alice");
        assert_eq!(output["user"]["profile"]["settings"]["theme"], "dark");
        assert_eq!(output["user"]["profile"]["settings"]["notifications"], true);
        assert!(output["timestamp"].is_string());
    }

    #[tokio::test]
    async fn test_transform_conditional_with_object_result() {
        let evaluator = TransformEvaluator::new();

        let node = Node {
            aturi: "test_node".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "transform".to_string(),
            payload: json!({
                "status": {
                    "if": [
                        {"==": [{"val": ["active"]}, true]},
                        {"code": "ACTIVE", "message": "User is active"},
                        {"code": "INACTIVE", "message": "User is inactive"}
                    ]
                }
            }),
            configuration: json!({}),
            created_at: Utc::now(),
        };

        let input = json!({"active": true});

        let result = evaluator.evaluate(&node, &input).await.unwrap();
        assert!(result.is_some());

        let output = result.unwrap();
        assert!(output.is_object());
        assert_eq!(output["status"]["code"], "ACTIVE");
        assert_eq!(output["status"]["message"], "User is active");
    }

    #[tokio::test]
    async fn test_transform_array_simple_chain() {
        let evaluator = TransformEvaluator::new();

        // Test the example from the user's requirements
        let node = Node {
            aturi: "test_node".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "transform".to_string(),
            payload: json!([
                {
                    "greeting": {"cat": ["Hello, ", {"val": ["name"]}, " from ", {"val": ["city"]}]},
                    "is_newyorker": {"==": [{"val": ["city"]}, "New York"]},
                    "voting_age": {">=": [{"val": ["age"]}, 18]}
                },
                {
                    "greeting": {"val": ["greeting"]},
                    "can_vote": {"and": [{"==": [{"val": ["voting_age"]}, true]}, {"==": [{"val": ["is_newyorker"]}, true]}]}
                }
            ]),
            configuration: json!({}),
            created_at: Utc::now(),
        };

        let input = json!({"name": "Alice", "age": 25, "city": "New York"});

        let result = evaluator.evaluate(&node, &input).await.unwrap();
        assert!(result.is_some());

        let output = result.unwrap();
        assert_eq!(output["greeting"], "Hello, Alice from New York");
        assert_eq!(output["can_vote"], true);
    }

    #[tokio::test]
    async fn test_transform_array_three_steps() {
        let evaluator = TransformEvaluator::new();

        let node = Node {
            aturi: "test_node".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "transform".to_string(),
            payload: json!([
                // Step 1: Extract basic fields
                {
                    "user_id": {"val": ["id"]},
                    "user_name": {"val": ["name"]},
                    "user_score": {"val": ["score"]}
                },
                // Step 2: Calculate status based on score
                {
                    "user_id": {"val": ["user_id"]},
                    "user_name": {"val": ["user_name"]},
                    "status": {
                        "if": [
                            {">": [{"val": ["user_score"]}, 100]},
                            "excellent",
                            {
                                "if": [
                                    {">": [{"val": ["user_score"]}, 50]},
                                    "good",
                                    "needs improvement"
                                ]
                            }
                        ]
                    }
                },
                // Step 3: Format final message
                {
                    "message": {
                        "cat": [
                            "User ",
                            {"val": ["user_name"]},
                            " (ID: ",
                            {"val": ["user_id"]},
                            ") has status: ",
                            {"val": ["status"]}
                        ]
                    }
                }
            ]),
            configuration: json!({}),
            created_at: Utc::now(),
        };

        let input = json!({"id": "u123", "name": "Bob", "score": 75});

        let result = evaluator.evaluate(&node, &input).await.unwrap();
        assert!(result.is_some());

        let output = result.unwrap();
        assert_eq!(output["message"], "User Bob (ID: u123) has status: good");
    }

    #[tokio::test]
    async fn test_transform_array_empty() {
        let evaluator = TransformEvaluator::new();

        let node = Node {
            aturi: "test_node".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "transform".to_string(),
            payload: json!([]), // Empty array
            configuration: json!({}),
            created_at: Utc::now(),
        };

        let input = json!({"test": "data"});

        let result = evaluator.evaluate(&node, &input).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("cannot be empty"));
    }

    #[tokio::test]
    async fn test_transform_array_invalid_element() {
        let evaluator = TransformEvaluator::new();

        let node = Node {
            aturi: "test_node".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "transform".to_string(),
            payload: json!([
                {"valid": "object"},
                "invalid string", // Not an object
                {"another": "object"}
            ]),
            configuration: json!({}),
            created_at: Utc::now(),
        };

        let input = json!({"test": "data"});

        let result = evaluator.evaluate(&node, &input).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("element 1 must be an object")
        );
    }

    #[tokio::test]
    async fn test_transform_array_element_evaluates_to_non_object() {
        let evaluator = TransformEvaluator::new();

        let node = Node {
            aturi: "test_node".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "transform".to_string(),
            payload: json!([
                {"field1": "value1"},
                {"val": ["field1"]} // This evaluates to a string, not an object
            ]),
            configuration: json!({}),
            created_at: Utc::now(),
        };

        let input = json!({"test": "data"});

        let result = evaluator.evaluate(&node, &input).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("element 1 must evaluate to an object")
        );
    }

    #[tokio::test]
    async fn test_transform_array_data_flow() {
        let evaluator = TransformEvaluator::new();

        // Test that data flows correctly from one transformation to the next
        let node = Node {
            aturi: "test_node".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "transform".to_string(),
            payload: json!([
                // Step 1: Add field1
                {
                    "field1": "value1",
                    "original": {"val": ["data"]}
                },
                // Step 2: Add field2, preserve field1
                {
                    "field1": {"val": ["field1"]},
                    "field2": "value2"
                },
                // Step 3: Add field3, only preserve field2
                {
                    "field2": {"val": ["field2"]},
                    "field3": "value3"
                }
            ]),
            configuration: json!({}),
            created_at: Utc::now(),
        };

        let input = json!({"data": "input_data"});

        let result = evaluator.evaluate(&node, &input).await.unwrap();
        assert!(result.is_some());

        let output = result.unwrap();
        // field1 should NOT exist (dropped in step 3)
        assert!(output.get("field1").is_none());
        // original should NOT exist (dropped in step 2)
        assert!(output.get("original").is_none());
        // field2 and field3 should exist
        assert_eq!(output["field2"], "value2");
        assert_eq!(output["field3"], "value3");
    }

    #[tokio::test]
    async fn test_transform_single_object_still_works() {
        // Ensure backward compatibility - single object payload should still work
        let evaluator = TransformEvaluator::new();

        let node = Node {
            aturi: "test_node".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "transform".to_string(),
            payload: json!({
                "greeting": {"cat": ["Hello, ", {"val": ["name"]}, " from ", {"val": ["city"]}]},
                "can_vote": {">=": [{"val": ["age"]}, 18]}
            }),
            configuration: json!({}),
            created_at: Utc::now(),
        };

        let input = json!({"name": "Alice", "age": 25, "city": "New York"});

        let result = evaluator.evaluate(&node, &input).await.unwrap();
        assert!(result.is_some());

        let output = result.unwrap();
        assert_eq!(output["greeting"], "Hello, Alice from New York");
        assert_eq!(output["can_vote"], true);
    }

    #[tokio::test]
    async fn test_transform_datalogic_evaluation_error() {
        let evaluator = TransformEvaluator::new();

        let node = Node {
            aturi: "test_node".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "transform".to_string(),
            payload: json!({
                "result": {"val": ["non_existent", "deep", "path", "that", "does", "not", "exist"]}
            }),
            configuration: json!({}),
            created_at: Utc::now(),
        };

        let input = json!({"other": "data"});

        // Should still return an object, just with null for missing values
        let result = evaluator.evaluate(&node, &input).await.unwrap();
        assert!(result.is_some());
        let output = result.unwrap();
        assert!(output.is_object());
        assert_eq!(output["result"], Value::Null);
    }
}
