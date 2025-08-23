//! Jetstream entry node for filtering AT Protocol firehose events.
//!
//! This module implements the entry point for blueprints that process Jetstream
//! events. Jetstream is a real-time firehose of AT Protocol events including
//! posts, likes, follows, and other activities on the network.
//!
//! # Usage
//!
//! Jetstream entry nodes must be the first node in a blueprint. They filter
//! incoming events based on DID, collection, and custom DataLogic expressions.
//!
//! # Configuration
//!
//! The configuration object supports:
//! - `did`: Optional string or array of strings - filters events by author DID
//! - `collection`: Optional string or array of strings - filters events by record collection (NSID)
//!
//! # Example Blueprints
//!
//! Filter posts from a specific user:
//! ```json
//! {
//!   "nodes": [
//!     {
//!       "type": "jetstream_entry",
//!       "configuration": {
//!         "did": "did:plc:xyz123",
//!         "collection": "app.bsky.feed.post"
//!       },
//!       "payload": true
//!     }
//!   ]
//! }
//! ```
//!
//! Filter posts containing "hello" from multiple users:
//! ```json
//! {
//!   "nodes": [
//!     {
//!       "type": "jetstream_entry",
//!       "configuration": {
//!         "collection": ["app.bsky.feed.post"],
//!         "did": ["did:plc:xyz123", "did:web:example.com"]
//!       },
//!       "payload": {
//!         "contains": [{
//!           "val": ["commit", "record", "text"]
//!         }, "hello"]
//!       }
//!     }
//!   ]
//! }
//! ```

use anyhow::{Context, Result};
use async_trait::async_trait;
use atproto_identity::validation::{
    is_valid_did_method_plc, is_valid_did_method_web, is_valid_did_method_webvh,
};
use serde_json::Value;

use crate::errors::{EngineError, ValidationError};
use crate::storage::node::Node;

use super::common::create_datalogic;
use super::evaluator::NodeEvaluator;

/// Validate that a DID string is properly formatted
pub fn validate_did(did: &str) -> Result<()> {
    // Check if it's a valid DID format
    if !did.starts_with("did:") {
        return Err(ValidationError::InvalidNodeConfiguration {
            node_type: "jetstream_entry".to_string(),
            details: format!("Invalid DID format '{}': must start with 'did:'", did),
        }.into());
    }

    // Validate based on the DID method
    if did.starts_with("did:plc:") {
        if !is_valid_did_method_plc(did) {
            return Err(ValidationError::InvalidNodeConfiguration {
                node_type: "jetstream_entry".to_string(),
                details: format!("Invalid PLC DID format '{}': must be 'did:plc:' followed by exactly 24 lowercase alphanumeric characters", did),
            }.into());
        }
    } else if did.starts_with("did:web:") {
        if !is_valid_did_method_web(did, false) {
            return Err(ValidationError::InvalidNodeConfiguration {
                node_type: "jetstream_entry".to_string(),
                details: format!("Invalid Web DID format '{}': must be 'did:web:' followed by a valid hostname and optional path segments", did),
            }.into());
        }
    } else if did.starts_with("did:webvh:") {
        if !is_valid_did_method_webvh(did, false) {
            return Err(ValidationError::InvalidNodeConfiguration {
                node_type: "jetstream_entry".to_string(),
                details: format!("Invalid WebVH DID format '{}'", did),
            }.into());
        }
    } else {
        // For other DID methods, just ensure basic format
        let parts: Vec<&str> = did.split(':').collect();
        if parts.len() < 3 || parts[1].is_empty() || parts[2].is_empty() {
            return Err(ValidationError::InvalidNodeConfiguration {
                node_type: "jetstream_entry".to_string(),
                details: format!("Invalid DID format '{}': must have format 'did:method:identifier'", did),
            }.into());
        }
    }

    Ok(())
}

/// Validate that a collection string follows AT Protocol collection naming conventions
pub fn validate_collection(collection: &str) -> Result<()> {
    // Collection names should follow reverse-DNS format
    // Examples: app.bsky.feed.post, com.atproto.sync.subscribeRepos

    if collection.is_empty() {
        return Err(ValidationError::InvalidNodeConfiguration {
            node_type: "jetstream_entry".to_string(),
            details: "Collection name cannot be empty".to_string(),
        }.into());
    }

    // Check for valid characters (alphanumeric, dots, and hyphens)
    if !collection
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
    {
        return Err(ValidationError::InvalidNodeConfiguration {
            node_type: "jetstream_entry".to_string(),
            details: format!("Invalid collection name '{}': must contain only alphanumeric characters, dots, and hyphens", collection),
        }.into());
    }

    // Split by dots and validate segments
    let segments: Vec<&str> = collection.split('.').collect();

    if segments.len() < 2 {
        return Err(ValidationError::InvalidNodeConfiguration {
            node_type: "jetstream_entry".to_string(),
            details: format!("Invalid collection name '{}': must have at least 2 segments separated by dots (e.g., 'app.bsky.feed.post')", collection),
        }.into());
    }

    // Each segment should be non-empty and start with a letter
    for (i, segment) in segments.iter().enumerate() {
        if segment.is_empty() {
            return Err(ValidationError::InvalidNodeConfiguration {
                node_type: "jetstream_entry".to_string(),
                details: format!("Invalid collection name '{}': segment {} is empty", collection, i + 1),
            }.into());
        }

        // First character should be alphabetic
        if !segment.chars().next().unwrap().is_ascii_alphabetic() {
            return Err(ValidationError::InvalidNodeConfiguration {
                node_type: "jetstream_entry".to_string(),
                details: format!("Invalid collection name '{}': segment '{}' must start with a letter", collection, segment),
            }.into());
        }

        // Segment shouldn't start or end with hyphen
        if segment.starts_with('-') || segment.ends_with('-') {
            return Err(ValidationError::InvalidNodeConfiguration {
                node_type: "jetstream_entry".to_string(),
                details: format!("Invalid collection name '{}': segment '{}' cannot start or end with a hyphen", collection, segment),
            }.into());
        }
    }

    Ok(())
}

/// Validate a jetstream_entry node configuration
pub fn validate_jetstream_entry_config(config: &Value) -> Result<()> {
    // Validate DID filter if present
    if let Some(did_filter) = config.get("did") {
        // Support both string and array formats
        let dids = if let Some(did_str) = did_filter.as_str() {
            vec![Value::String(did_str.to_string())]
        } else if let Some(did_array) = did_filter.as_array() {
            did_array.clone()
        } else {
            return Err(ValidationError::InvalidFieldType {
                field_name: "did".to_string(),
                context: "jetstream_entry configuration".to_string(),
                expected_type: "string or array".to_string(),
            }.into());
        };

        if dids.is_empty() {
            return Err(ValidationError::InvalidNodeConfiguration {
                node_type: "jetstream_entry".to_string(),
                details: "DID list must contain at least one valid DID".to_string(),
            }.into());
        }

        for (i, value) in dids.iter().enumerate() {
            let did = value
                .as_str()
                .ok_or_else(|| ValidationError::InvalidFieldType {
                    field_name: format!("did[{}]", i),
                    context: "jetstream_entry configuration".to_string(),
                    expected_type: "string".to_string(),
                })?;

            if did.is_empty() {
                return Err(ValidationError::InvalidNodeConfiguration {
                    node_type: "jetstream_entry".to_string(),
                    details: format!("Empty DID string at index {}", i),
                }.into());
            }

            validate_did(did).with_context(|| format!("Invalid DID at index {}", i))?;
        }
    }

    // Validate collection filter if present
    if let Some(collection_filter) = config.get("collection") {
        // Support both string and array formats
        let collections = if let Some(coll_str) = collection_filter.as_str() {
            vec![Value::String(coll_str.to_string())]
        } else if let Some(coll_array) = collection_filter.as_array() {
            coll_array.clone()
        } else {
            return Err(ValidationError::InvalidFieldType {
                field_name: "collection".to_string(),
                context: "jetstream_entry configuration".to_string(),
                expected_type: "string or array".to_string(),
            }.into());
        };

        if collections.is_empty() {
            return Err(ValidationError::InvalidNodeConfiguration {
                node_type: "jetstream_entry".to_string(),
                details: "Collection list must contain at least one valid collection".to_string(),
            }.into());
        }

        for (i, value) in collections.iter().enumerate() {
            let collection = value
                .as_str()
                .ok_or_else(|| ValidationError::InvalidFieldType {
                    field_name: format!("collection[{}]", i),
                    context: "jetstream_entry configuration".to_string(),
                    expected_type: "string".to_string(),
                })?;

            if collection.is_empty() {
                return Err(ValidationError::InvalidNodeConfiguration {
                    node_type: "jetstream_entry".to_string(),
                    details: format!("Empty collection string at index {}", i),
                }.into());
            }

            validate_collection(collection)
                .with_context(|| format!("Invalid collection at index {}", i))?;
        }
    }

    Ok(())
}

/// Evaluator for Jetstream entry nodes.
///
/// This evaluator handles Jetstream inbound triggers that filter events based on
/// DID and collection criteria, then evaluate payload expressions using DataLogic.
///
/// # Event Structure
///
/// Jetstream events have the following structure:
/// ```json
/// {
///   "kind": "commit",
///   "did": "did:plc:...",
///   "commit": {
///     "collection": "app.bsky.feed.post",
///     "rkey": "...",
///     "cid": "...",
///     "operation": "create",
///     "record": {
///       "$type": "app.bsky.feed.post",
///       "text": "Hello world!",
///       "createdAt": "2024-01-01T00:00:00.000Z"
///     }
///   },
///   "time_us": 1234567890
/// }
/// ```
///
/// # Filter Matching Behavior
///
/// - **DID Filter**: If configured, the input's `did` field must match one of the configured DIDs
/// - **Collection Filter**: If configured, the input's `commit.collection` field must match one of the configured collections
/// - **Both Filters**: If both are configured, BOTH must match (AND logic)
/// - **No Filters**: If neither is configured, all events pass through to payload evaluation
///
/// # Configuration
///
/// The node's configuration field is a JSON object that can contain:
/// - `did`: An optional string or array of DID strings to filter incoming events
/// - `collection`: An optional string or array of collection strings to filter incoming events
///
/// ## Configuration Examples
///
/// Empty configuration (accepts all events):
/// ```json
/// {}
/// ```
///
/// Filter by DID only (single user):
/// ```json
/// {"did": "did:plc:alice"}
/// ```
///
/// Filter by DID only (multiple users):
/// ```json
/// {"did": ["did:plc:alice", "did:web:example.com"]}
/// ```
///
/// Filter by collection only (single type):
/// ```json
/// {"collection": "app.bsky.feed.post"}
/// ```
///
/// Filter by collection only (multiple types):
/// ```json
/// {"collection": ["app.bsky.feed.post", "app.bsky.feed.like"]}
/// ```
///
/// Filter by both DID and collection (specific users' specific actions):
/// ```json
/// {
///   "collection": "app.bsky.feed.post",
///   "did": "did:plc:alice"
/// }
/// ```
///
/// # Payload
///
/// The payload field can be either:
/// - A boolean (`true` or `false`) for simple accept/reject logic
/// - A DataLogic expression object that evaluates to a boolean
///
/// The payload is evaluated AFTER configuration filters pass. If configuration
/// filters don't match, the payload is not evaluated.
///
/// ## Payload Examples
///
/// Accept all events that pass configuration filters:
/// ```json
/// true
/// ```
///
/// Reject all events (useful for temporarily disabling a blueprint):
/// ```json
/// false
/// ```
///
/// Filter posts containing specific text:
/// ```json
/// {
///   "contains": [
///     {"val": ["commit", "record", "text"]},
///     "hello"
///   ]
/// }
/// ```
///
/// Filter posts with mentions:
/// ```json
/// {
///   ">": [
///     {"len": {"val": ["commit", "record", "facets"]}},
///     0
///   ]
/// }
/// ```
///
/// Complex filter with multiple conditions:
/// ```json
/// {
///   "and": [
///     {"==": [{"val": ["kind"]}, "commit"]},
///     {"==": [{"val": ["commit", "operation"]}, "create"]},
///     {"contains": [{"val": ["commit", "record", "text"]}, "#atproto"]}
///   ]
/// }
/// ```
///
/// # Behavior
///
/// 1. First, the evaluator checks if the input matches the configuration filters
/// 2. If filters don't match, returns `None` to stop blueprint evaluation
/// 3. If filters match, evaluates the payload using DataLogic
/// 4. The payload must evaluate to a boolean:
///    - `true`: Continue blueprint evaluation with input data
///    - `false`: Stop blueprint evaluation
///
/// # Input Requirements
///
/// The input data must contain:
/// - `did`: The DID of the event source (required if `did` filter is set)
/// - `commit.collection`: The collection type (required if `collection` filter is set)
/// - Any additional fields required by the payload evaluation
///
/// Note: The collection is extracted from `input.commit.collection`, not directly from `input.collection`
pub struct JetstreamEntryEvaluator;

impl JetstreamEntryEvaluator {
    /// Create a new Jetstream entry evaluator
    pub fn new() -> Self {
        Self
    }

    /// Check if the input matches the configuration filters
    /// Assumes configuration has already been validated at blueprint creation time
    fn matches_filters(&self, config: &Value, input: &Value) -> Result<bool> {
        // Check DID filter if present
        if let Some(did_filter) = config.get("did") {
            // Support both string and array formats
            let allowed_dids = if let Some(did_str) = did_filter.as_str() {
                vec![did_str]
            } else if let Some(did_array) = did_filter.as_array() {
                did_array
                    .iter()
                    .filter_map(|v| v.as_str())
                    .collect::<Vec<_>>()
            } else {
                return Err(ValidationError::InvalidFieldType {
                field_name: "did".to_string(),
                context: "jetstream_entry configuration".to_string(),
                expected_type: "string or array".to_string(),
            }.into());
            };

            // Get the DID from input - only check if we have a DID filter configured
            if let Some(input_did) = input.get("did").and_then(|d| d.as_str()) {
                // Check if input DID matches any allowed DID
                let matches = allowed_dids.iter().any(|&d| d == input_did);
                if !matches {
                    return Ok(false);
                }
            } else {
                // If DID filter is configured but input has no DID, don't match
                return Ok(false);
            }
        }

        // Check collection filter if present
        if let Some(collection_filter) = config.get("collection") {
            // Support both string and array formats
            let allowed_collections = if let Some(coll_str) = collection_filter.as_str() {
                vec![coll_str]
            } else if let Some(coll_array) = collection_filter.as_array() {
                coll_array
                    .iter()
                    .filter_map(|v| v.as_str())
                    .collect::<Vec<_>>()
            } else {
                return Err(EngineError::InvalidNodeConfiguration {
                    node_type: "jetstream_entry".to_string(),
                    details: "Collection field must be a string or an array".to_string(),
                }.into());
            };

            // Get the collection from input.commit.collection
            if let Some(input_collection) = input
                .get("commit")
                .and_then(|c| c.get("collection"))
                .and_then(|c| c.as_str())
            {
                // Check if input collection matches any allowed collection
                let matches = allowed_collections.iter().any(|&c| c == input_collection);
                if !matches {
                    return Ok(false);
                }
            } else {
                // If collection filter is configured but input has no commit.collection, don't match
                return Ok(false);
            }
        }

        Ok(true)
    }
}

impl Default for JetstreamEntryEvaluator {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl NodeEvaluator for JetstreamEntryEvaluator {
    async fn evaluate(&self, node: &Node, input: &Value) -> Result<Option<Value>> {
        // First, check if the input matches the configuration filters
        if !self.matches_filters(&node.configuration, input)? {
            // If filters don't match, stop blueprint evaluation
            return Ok(None);
        }

        // If filters match, evaluate the payload
        let result = if let Some(bool_value) = node.payload.as_bool() {
            Value::Bool(bool_value)
        } else if node.payload.is_object() {
            let datalogic = create_datalogic();
            datalogic.evaluate_json(&node.payload, input, None)?
        } else {
            return Err(EngineError::InvalidFieldType {
                field_name: "payload".to_string(),
                node_type: "jetstream_entry".to_string(),
                expected_type: "boolean or object".to_string(),
            }.into());
        };

        // Ensure the result is a boolean
        match result {
            Value::Bool(true) => Ok(Some(input.clone())),
            Value::Bool(false) => Ok(None),
            _ => Err(EngineError::DataLogicFailed {
                expression: format!("{:?}", node.payload),
                details: format!("Node must evaluate to a boolean, got: {:?}", result),
            }.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[tokio::test]
    async fn test_jetstream_entry_no_filters() {
        let evaluator = JetstreamEntryEvaluator::new();

        let node = Node {
            aturi: "test_jetstream".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "jetstream_entry".to_string(),
            payload: serde_json::json!(true),
            configuration: serde_json::json!({}),
            created_at: Utc::now(),
        };

        let input = serde_json::json!({
            "did": "did:web:example.com",
            "collection": "app.bsky.feed.post"
        });

        let result = evaluator.evaluate(&node, &input).await.unwrap();
        assert_eq!(result, Some(input.clone()));
    }

    #[tokio::test]
    async fn test_jetstream_entry_with_did_filter() {
        let evaluator = JetstreamEntryEvaluator::new();

        let node = Node {
            aturi: "test_jetstream".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "jetstream_entry".to_string(),
            payload: serde_json::json!(true),
            configuration: serde_json::json!({
                "did": ["did:web:ngerakines.me", "did:web:example.com"]
            }),
            created_at: Utc::now(),
        };

        // Test matching DID
        let input = serde_json::json!({
            "did": "did:web:ngerakines.me",
            "collection": "app.bsky.feed.post"
        });

        let result = evaluator.evaluate(&node, &input).await.unwrap();
        assert_eq!(result, Some(input.clone()));

        // Test non-matching DID
        let input = serde_json::json!({
            "did": "did:web:other.com",
            "collection": "app.bsky.feed.post"
        });

        let result = evaluator.evaluate(&node, &input).await.unwrap();
        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn test_jetstream_entry_with_collection_filter() {
        let evaluator = JetstreamEntryEvaluator::new();

        let node = Node {
            aturi: "test_jetstream".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "jetstream_entry".to_string(),
            payload: serde_json::json!(true),
            configuration: serde_json::json!({
                "collection": ["app.bsky.feed.post", "app.bsky.feed.like"]
            }),
            created_at: Utc::now(),
        };

        // Test matching collection
        let input = serde_json::json!({
            "did": "did:web:example.com",
            "commit": {
                "collection": "app.bsky.feed.post"
            }
        });

        let result = evaluator.evaluate(&node, &input).await.unwrap();
        assert_eq!(result, Some(input.clone()));

        // Test non-matching collection
        let input = serde_json::json!({
            "did": "did:web:example.com",
            "commit": {
                "collection": "app.bsky.graph.follow"
            }
        });

        let result = evaluator.evaluate(&node, &input).await.unwrap();
        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn test_jetstream_entry_with_both_filters() {
        let evaluator = JetstreamEntryEvaluator::new();

        let node = Node {
            aturi: "test_jetstream".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "jetstream_entry".to_string(),
            payload: serde_json::json!(true),
            configuration: serde_json::json!({
                "did": ["did:web:ngerakines.me"],
                "collection": ["app.bsky.feed.post", "app.bsky.feed.like"]
            }),
            created_at: Utc::now(),
        };

        // Test matching both filters
        let input = serde_json::json!({
            "did": "did:web:ngerakines.me",
            "commit": {
                "collection": "app.bsky.feed.post"
            }
        });

        let result = evaluator.evaluate(&node, &input).await.unwrap();
        assert_eq!(result, Some(input.clone()));

        // Test non-matching DID
        let input = serde_json::json!({
            "did": "did:web:other.com",
            "commit": {
                "collection": "app.bsky.feed.post"
            }
        });

        let result = evaluator.evaluate(&node, &input).await.unwrap();
        assert_eq!(result, None);

        // Test non-matching collection
        let input = serde_json::json!({
            "did": "did:web:ngerakines.me",
            "commit": {
                "collection": "app.bsky.graph.follow"
            }
        });

        let result = evaluator.evaluate(&node, &input).await.unwrap();
        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn test_jetstream_entry_with_boolean_payload() {
        let evaluator = JetstreamEntryEvaluator::new();

        // Test with boolean true payload
        let node = Node {
            aturi: "test_jetstream".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "jetstream_entry".to_string(),
            payload: serde_json::json!(true),
            configuration: serde_json::json!({}),
            created_at: Utc::now(),
        };

        let input = serde_json::json!({
            "did": "did:web:example.com",
            "collection": "app.bsky.feed.post"
        });

        let result = evaluator.evaluate(&node, &input).await.unwrap();
        assert_eq!(result, Some(input.clone()));

        // Test with boolean false payload
        let node = Node {
            aturi: "test_jetstream".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "jetstream_entry".to_string(),
            payload: serde_json::json!(false),
            configuration: serde_json::json!({}),
            created_at: Utc::now(),
        };

        let result = evaluator.evaluate(&node, &input).await.unwrap();
        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn test_jetstream_entry_with_payload_evaluation() {
        let evaluator = JetstreamEntryEvaluator::new();

        let node = Node {
            aturi: "test_jetstream".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "jetstream_entry".to_string(),
            payload: serde_json::json!({
                "==": [
                    {"val": ["text"]},
                    "Hello, world!"
                ]
            }),
            configuration: serde_json::json!({}),
            created_at: Utc::now(),
        };

        // Test matching payload condition
        let input = serde_json::json!({
            "did": "did:web:example.com",
            "collection": "app.bsky.feed.post",
            "text": "Hello, world!"
        });

        let result = evaluator.evaluate(&node, &input).await.unwrap();
        assert_eq!(result, Some(input.clone()));

        // Test non-matching payload condition
        let input = serde_json::json!({
            "did": "did:web:example.com",
            "collection": "app.bsky.feed.post",
            "text": "Different text"
        });

        let result = evaluator.evaluate(&node, &input).await.unwrap();
        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn test_jetstream_entry_with_valid_plc_did() {
        let evaluator = JetstreamEntryEvaluator::new();

        // Test with valid PLC DID
        let node = Node {
            aturi: "test_jetstream".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "jetstream_entry".to_string(),
            payload: serde_json::json!(true),
            configuration: serde_json::json!({
                "did": ["did:plc:abcdefghijklmnopqrstuvwx"]
            }),
            created_at: Utc::now(),
        };

        let input = serde_json::json!({
            "did": "did:plc:abcdefghijklmnopqrstuvwx",
            "collection": "app.bsky.feed.post"
        });

        let result = evaluator.evaluate(&node, &input).await.unwrap();
        assert_eq!(result, Some(input.clone()));
    }

    #[tokio::test]
    async fn test_jetstream_entry_with_string_config() {
        let evaluator = JetstreamEntryEvaluator::new();

        // Test with string DID configuration
        let node = Node {
            aturi: "test_jetstream".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "jetstream_entry".to_string(),
            payload: serde_json::json!(true),
            configuration: serde_json::json!({
                "did": "did:web:ngerakines.me"
            }),
            created_at: Utc::now(),
        };

        let input = serde_json::json!({
            "did": "did:web:ngerakines.me",
            "commit": {
                "collection": "app.bsky.feed.post"
            }
        });

        let result = evaluator.evaluate(&node, &input).await.unwrap();
        assert_eq!(result, Some(input.clone()));

        // Test with string collection configuration
        let node = Node {
            aturi: "test_jetstream".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "jetstream_entry".to_string(),
            payload: serde_json::json!(true),
            configuration: serde_json::json!({
                "collection": "app.bsky.feed.post"
            }),
            created_at: Utc::now(),
        };

        let result = evaluator.evaluate(&node, &input).await.unwrap();
        assert_eq!(result, Some(input.clone()));

        // Test with both string configurations
        let node = Node {
            aturi: "test_jetstream".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "jetstream_entry".to_string(),
            payload: serde_json::json!(true),
            configuration: serde_json::json!({
                "did": "did:web:ngerakines.me",
                "collection": "app.bsky.feed.post"
            }),
            created_at: Utc::now(),
        };

        let result = evaluator.evaluate(&node, &input).await.unwrap();
        assert_eq!(result, Some(input.clone()));
    }

    #[tokio::test]
    async fn test_jetstream_entry_with_valid_collections() {
        let evaluator = JetstreamEntryEvaluator::new();

        // Test with various valid collection formats
        let node = Node {
            aturi: "test_jetstream".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "jetstream_entry".to_string(),
            payload: serde_json::json!(true),
            configuration: serde_json::json!({
                "collection": [
                    "app.bsky.feed.post",
                    "com.atproto.sync.subscribeRepos",
                    "app.bsky-social.feed.like"
                ]
            }),
            created_at: Utc::now(),
        };

        let input = serde_json::json!({
            "did": "did:web:example.com",
            "commit": {
                "collection": "app.bsky.feed.post"
            }
        });

        let result = evaluator.evaluate(&node, &input).await.unwrap();
        assert_eq!(result, Some(input.clone()));
    }

    #[test]
    fn test_did_validation() {
        // Valid DIDs
        assert!(validate_did("did:plc:abcdefghijklmnopqrstuvwx").is_ok());
        assert!(validate_did("did:web:example.com").is_ok());
        assert!(validate_did("did:web:example.com:path").is_ok());
        assert!(validate_did("did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK").is_ok());

        // Invalid DIDs
        assert!(validate_did("").is_err());
        assert!(validate_did("not-a-did").is_err());
        assert!(validate_did("did:").is_err());
        assert!(validate_did("did::").is_err());
        assert!(validate_did("did:plc:").is_err());
        assert!(validate_did("did:plc:tooshort").is_err());
        assert!(validate_did("did:plc:UPPERCASE123456789012345").is_err());
    }

    #[test]
    fn test_validate_jetstream_entry_config_string_formats() {
        // Test string DID configuration
        let config = serde_json::json!({
            "did": "did:web:example.com"
        });
        assert!(validate_jetstream_entry_config(&config).is_ok());

        // Test string collection configuration
        let config = serde_json::json!({
            "collection": "app.bsky.feed.post"
        });
        assert!(validate_jetstream_entry_config(&config).is_ok());

        // Test both string configurations
        let config = serde_json::json!({
            "did": "did:plc:abcdefghijklmnopqrstuvwx",
            "collection": "app.bsky.feed.like"
        });
        assert!(validate_jetstream_entry_config(&config).is_ok());

        // Test array configurations (still supported)
        let config = serde_json::json!({
            "did": ["did:web:example.com", "did:plc:abcdefghijklmnopqrstuvwx"],
            "collection": ["app.bsky.feed.post", "app.bsky.feed.like"]
        });
        assert!(validate_jetstream_entry_config(&config).is_ok());

        // Test invalid DID type
        let config = serde_json::json!({
            "did": 123
        });
        assert!(validate_jetstream_entry_config(&config).is_err());

        // Test invalid collection type
        let config = serde_json::json!({
            "collection": true
        });
        assert!(validate_jetstream_entry_config(&config).is_err());
    }

    #[test]
    fn test_collection_validation() {
        // Valid collections
        assert!(validate_collection("app.bsky.feed.post").is_ok());
        assert!(validate_collection("com.atproto.sync.subscribeRepos").is_ok());
        assert!(validate_collection("app.bsky-social.feed.like").is_ok());
        assert!(validate_collection("org.example.custom.action").is_ok());

        // Invalid collections
        assert!(validate_collection("").is_err());
        assert!(validate_collection("no-dots").is_err());
        assert!(validate_collection("app").is_err());
        assert!(validate_collection(".app.bsky").is_err());
        assert!(validate_collection("app..bsky").is_err());
        assert!(validate_collection("app.bsky.").is_err());
        assert!(validate_collection("app.bsky.feed.post!").is_err());
        assert!(validate_collection("app.123.feed").is_err());
        assert!(validate_collection("app.-bsky.feed").is_err());
        assert!(validate_collection("app.bsky-.feed").is_err());
    }

    // Comprehensive DID filtering tests
    #[tokio::test]
    async fn test_jetstream_entry_did_filter_missing_input_did() {
        let evaluator = JetstreamEntryEvaluator::new();

        let node = Node {
            aturi: "test_jetstream".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "jetstream_entry".to_string(),
            payload: serde_json::json!(true),
            configuration: serde_json::json!({
                "did": "did:web:example.com"
            }),
            created_at: Utc::now(),
        };

        // Input missing 'did' field
        let input = serde_json::json!({
            "commit": {
                "collection": "app.bsky.feed.post"
            }
        });

        let result = evaluator.evaluate(&node, &input).await.unwrap();
        // Should not match because DID filter is configured but input has no DID
        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn test_jetstream_entry_collection_filter_missing_commit() {
        let evaluator = JetstreamEntryEvaluator::new();

        let node = Node {
            aturi: "test_jetstream".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "jetstream_entry".to_string(),
            payload: serde_json::json!(true),
            configuration: serde_json::json!({
                "collection": "app.bsky.feed.post"
            }),
            created_at: Utc::now(),
        };

        // Input missing 'commit' field entirely
        let input = serde_json::json!({
            "did": "did:web:example.com"
        });

        let result = evaluator.evaluate(&node, &input).await.unwrap();
        // Should not match because collection filter is configured but input has no commit.collection
        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn test_jetstream_entry_collection_filter_missing_collection_in_commit() {
        let evaluator = JetstreamEntryEvaluator::new();

        let node = Node {
            aturi: "test_jetstream".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "jetstream_entry".to_string(),
            payload: serde_json::json!(true),
            configuration: serde_json::json!({
                "collection": "app.bsky.feed.post"
            }),
            created_at: Utc::now(),
        };

        // Input has commit but no collection field in it
        let input = serde_json::json!({
            "did": "did:web:example.com",
            "commit": {
                "operation": "create"
            }
        });

        let result = evaluator.evaluate(&node, &input).await.unwrap();
        // Should not match because collection filter is configured but commit has no collection
        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn test_jetstream_entry_both_filters_one_missing() {
        let evaluator = JetstreamEntryEvaluator::new();

        let node = Node {
            aturi: "test_jetstream".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "jetstream_entry".to_string(),
            payload: serde_json::json!(true),
            configuration: serde_json::json!({
                "did": "did:web:example.com",
                "collection": "app.bsky.feed.post"
            }),
            created_at: Utc::now(),
        };

        // Test with matching DID but missing collection
        let input = serde_json::json!({
            "did": "did:web:example.com"
        });

        let result = evaluator.evaluate(&node, &input).await.unwrap();
        assert_eq!(result, None);

        // Test with matching collection but missing DID
        let input = serde_json::json!({
            "commit": {
                "collection": "app.bsky.feed.post"
            }
        });

        let result = evaluator.evaluate(&node, &input).await.unwrap();
        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn test_jetstream_entry_array_filters_multiple_matches() {
        let evaluator = JetstreamEntryEvaluator::new();

        let node = Node {
            aturi: "test_jetstream".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "jetstream_entry".to_string(),
            payload: serde_json::json!(true),
            configuration: serde_json::json!({
                "did": ["did:web:example.com", "did:plc:abcd1234", "did:web:test.org"],
                "collection": ["app.bsky.feed.post", "app.bsky.feed.like", "app.bsky.graph.follow"]
            }),
            created_at: Utc::now(),
        };

        // Test matching second DID and first collection
        let input = serde_json::json!({
            "did": "did:plc:abcd1234",
            "commit": {
                "collection": "app.bsky.feed.post"
            }
        });

        let result = evaluator.evaluate(&node, &input).await.unwrap();
        assert_eq!(result, Some(input.clone()));

        // Test matching last DID and middle collection
        let input = serde_json::json!({
            "did": "did:web:test.org",
            "commit": {
                "collection": "app.bsky.feed.like"
            }
        });

        let result = evaluator.evaluate(&node, &input).await.unwrap();
        assert_eq!(result, Some(input.clone()));
    }

    #[tokio::test]
    async fn test_jetstream_entry_payload_false() {
        let evaluator = JetstreamEntryEvaluator::new();

        let node = Node {
            aturi: "test_jetstream".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "jetstream_entry".to_string(),
            payload: serde_json::json!(false), // Payload is false
            configuration: serde_json::json!({}),
            created_at: Utc::now(),
        };

        let input = serde_json::json!({
            "did": "did:web:example.com",
            "commit": {
                "collection": "app.bsky.feed.post"
            }
        });

        let result = evaluator.evaluate(&node, &input).await.unwrap();
        // Should return None because payload is false
        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn test_jetstream_entry_payload_object_evaluates_false() {
        let evaluator = JetstreamEntryEvaluator::new();

        let node = Node {
            aturi: "test_jetstream".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "jetstream_entry".to_string(),
            payload: serde_json::json!({
                "==": [
                    {"val": ["commit", "operation"]},
                    "delete" // Looking for delete operations
                ]
            }),
            configuration: serde_json::json!({}),
            created_at: Utc::now(),
        };

        let input = serde_json::json!({
            "did": "did:web:example.com",
            "commit": {
                "collection": "app.bsky.feed.post",
                "operation": "create" // But input has create
            }
        });

        let result = evaluator.evaluate(&node, &input).await.unwrap();
        // Should return None because payload evaluates to false
        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn test_jetstream_entry_invalid_payload_type() {
        let evaluator = JetstreamEntryEvaluator::new();

        let node = Node {
            aturi: "test_jetstream".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "jetstream_entry".to_string(),
            payload: serde_json::json!("string payload"), // Invalid - not boolean or object
            configuration: serde_json::json!({}),
            created_at: Utc::now(),
        };

        let input = serde_json::json!({
            "did": "did:web:example.com",
            "commit": {
                "collection": "app.bsky.feed.post"
            }
        });

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
    async fn test_jetstream_entry_payload_evaluates_non_boolean() {
        let evaluator = JetstreamEntryEvaluator::new();

        let node = Node {
            aturi: "test_jetstream".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "jetstream_entry".to_string(),
            payload: serde_json::json!({
                "val": ["commit", "collection"] // This returns a string, not a boolean
            }),
            configuration: serde_json::json!({}),
            created_at: Utc::now(),
        };

        let input = serde_json::json!({
            "did": "did:web:example.com",
            "commit": {
                "collection": "app.bsky.feed.post"
            }
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
    async fn test_jetstream_entry_case_sensitive_matching() {
        let evaluator = JetstreamEntryEvaluator::new();

        // Test case sensitivity for DIDs
        let node = Node {
            aturi: "test_jetstream".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "jetstream_entry".to_string(),
            payload: serde_json::json!(true),
            configuration: serde_json::json!({
                "did": "did:web:Example.Com" // Mixed case
            }),
            created_at: Utc::now(),
        };

        let input = serde_json::json!({
            "did": "did:web:example.com", // Lower case
            "commit": {
                "collection": "app.bsky.feed.post"
            }
        });

        let result = evaluator.evaluate(&node, &input).await.unwrap();
        // Should not match due to case sensitivity
        assert_eq!(result, None);

        // Test case sensitivity for collections
        let node = Node {
            aturi: "test_jetstream".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "jetstream_entry".to_string(),
            payload: serde_json::json!(true),
            configuration: serde_json::json!({
                "collection": "App.Bsky.Feed.Post" // Mixed case
            }),
            created_at: Utc::now(),
        };

        let input = serde_json::json!({
            "did": "did:web:example.com",
            "commit": {
                "collection": "app.bsky.feed.post" // Lower case
            }
        });

        let result = evaluator.evaluate(&node, &input).await.unwrap();
        // Should not match due to case sensitivity
        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn test_jetstream_entry_complex_payload_with_filters() {
        let evaluator = JetstreamEntryEvaluator::new();

        let node = Node {
            aturi: "test_jetstream".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "jetstream_entry".to_string(),
            payload: serde_json::json!({
                "==": [{"val": ["commit", "operation"]}, "create"]
            }),
            configuration: serde_json::json!({
                "did": "did:web:example.com",
                "collection": "app.bsky.feed.post"
            }),
            created_at: Utc::now(),
        };

        // Test with all matching conditions (filters and payload)
        let input = serde_json::json!({
            "did": "did:web:example.com",
            "commit": {
                "collection": "app.bsky.feed.post",
                "operation": "create",
                "record": {
                    "text": "hello world"
                }
            }
        });

        let result = evaluator.evaluate(&node, &input).await.unwrap();
        assert_eq!(result, Some(input.clone()));

        // Test with filters matching but payload condition failing (wrong operation)
        let input = serde_json::json!({
            "did": "did:web:example.com",
            "commit": {
                "collection": "app.bsky.feed.post",
                "operation": "delete", // Wrong operation - should not match
                "record": {
                    "text": "hello world"
                }
            }
        });

        let result = evaluator.evaluate(&node, &input).await.unwrap();
        assert_eq!(result, None);

        // Test with payload matching but filters not matching (wrong DID)
        let input = serde_json::json!({
            "did": "did:web:other.com", // Wrong DID - should not match
            "commit": {
                "collection": "app.bsky.feed.post",
                "operation": "create",
                "record": {
                    "text": "hello world"
                }
            }
        });

        let result = evaluator.evaluate(&node, &input).await.unwrap();
        assert_eq!(result, None);
    }
}
