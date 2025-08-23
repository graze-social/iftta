//! Parse AT-URI node for extracting repository, collection, and record key from AT-URIs.
//!
//! This module implements AT-URI parsing to extract structured components from
//! AT Protocol URIs. AT-URIs follow the format: at://[repository]/[collection]/[record_key]
//! where repository is typically a DID, collection is a lexicon identifier, and
//! record_key is a record identifier.
//!
//! # Usage
//!
//! Parse AT-URI nodes extract and parse AT-URI strings from input data,
//! outputting structured components for use in subsequent processing steps.
//!
//! # Example Blueprint
//!
//! ```json
//! {
//!   "nodes": [
//!     {
//!       "node_type": "webhook_entry"
//!     },
//!     {
//!       "node_type": "parse_aturi",
//!       "configuration": {"destination": "parsed"},
//!       "payload": {"val": ["commit", "record", "subject", "uri"]}
//!     },
//!     {
//!       "node_type": "condition",
//!       "payload": {
//!         "==": [
//!           {"val": ["parsed", "collection"]},
//!           "community.lexicon.calendar.event"
//!         ]
//!       }
//!     }
//!   ]
//! }
//! ```
//!
//! # Output Structure
//!
//! The node clones the input and sets the destination field with a parsed result object containing:
//! - `repository`: The repository identifier (typically a DID)
//! - `collection`: The collection/lexicon identifier
//! - `record_key`: The record key/identifier
//!
//! Example with input `{"uri": "at://did:plc:abc123/app.bsky.feed.post/xyz789"}` and destination `"parsed"`:
//! ```json
//! {
//!   "uri": "at://did:plc:abc123/app.bsky.feed.post/xyz789",
//!   "parsed": {
//!     "repository": "did:plc:abc123",
//!     "collection": "app.bsky.feed.post",
//!     "record_key": "xyz789"
//!   }
//! }
//! ```

use anyhow::Result;
use async_trait::async_trait;
use atproto_record::aturi::ATURI;
use serde_json::{Value, json};
use std::str::FromStr;

use super::common::create_datalogic;
use super::evaluator::NodeEvaluator;
use crate::errors::EngineError;
use crate::storage::node::Node;

/// Evaluator for the parse_aturi node type.
///
/// This evaluator parses AT-URI strings to extract repository, collection,
/// and record key components. It validates the URI format and extracts
/// the structured components for downstream processing.
///
/// # Configuration
///
/// The node's configuration supports:
/// - `destination`: The field name to store the parsed result (default: "parsed_aturi")
///
/// ## Configuration Examples
///
/// Use default destination:
/// ```json
/// {}
/// ```
///
/// Specify custom destination:
/// ```json
/// {"destination": "parsed"}
/// ```
///
/// # Payload
///
/// The payload field determines how to extract the AT-URI:
/// - String: Use as field name to extract AT-URI from input
/// - Object: Evaluate using DataLogic to get AT-URI
///
/// ## Payload Examples
///
/// Extract from field:
/// ```json
/// "uri"
/// ```
///
/// Extract using DataLogic:
/// ```json
/// {"val": ["subject", "uri"]}
/// ```
pub struct ParseAturiEvaluator;

impl ParseAturiEvaluator {
    /// Creates a new ParseAturiEvaluator.
    pub fn new() -> Self {
        Self
    }

    /// Parse an AT-URI string into its components.
    ///
    /// # Arguments
    ///
    /// * `uri` - The AT-URI string to parse
    ///
    /// # Returns
    ///
    /// Returns a tuple of (repository, collection, record_key) if the URI is valid,
    /// or an error if the format is invalid.
    fn parse_aturi(uri: &str) -> Result<(String, String, String)> {
        // Parse the AT-URI using the atproto_record library
        let parsed = ATURI::from_str(uri)
            .map_err(|e| EngineError::AtUriParsingFailed {
                uri: uri.to_string(),
                details: format!("Invalid AT-URI format: {}", e),
            })?;

        // Extract components from the parsed ATURI
        let repository = parsed.authority.clone();
        let collection = parsed.collection.clone();
        let record_key = parsed.record_key.clone();

        // Validate that collection is not empty
        if collection.is_empty() {
            return Err(EngineError::AtUriParsingFailed {
                uri: uri.to_string(),
                details: "Invalid AT-URI: missing collection".to_string(),
            }.into());
        }

        Ok((repository, collection, record_key))
    }
}

impl Default for ParseAturiEvaluator {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl NodeEvaluator for ParseAturiEvaluator {
    async fn evaluate(&self, node: &Node, input: &Value) -> Result<Option<Value>> {
        // Get the destination field name from configuration (default to "parsed_aturi")
        let destination = node
            .configuration
            .get("destination")
            .and_then(|v| v.as_str())
            .unwrap_or("parsed_aturi");

        // Extract the AT-URI based on payload type
        let uri = if let Some(field_name) = node.payload.as_str() {
            // Payload is string - use as field name
            input
                .get(field_name)
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    EngineError::AtUriParsingFailed {
                        uri: field_name.to_string(),
                        details: format!("Field '{}' not found or not a string in input", field_name),
                    }
                })?
                .to_string()
        } else if node.payload.is_object() {
            // Payload is object - evaluate with DataLogic
            let datalogic = create_datalogic();
            let result = datalogic.evaluate_json(&node.payload, input, None)?;
            if let Some(uri_str) = result.as_str() {
                uri_str.to_string()
            } else {
                return Err(EngineError::AtUriParsingFailed {
                    uri: "datalogic_result".to_string(),
                    details: format!("DataLogic evaluation must return a string, got: {:?}", result),
                }.into());
            }
        } else {
            return Err(EngineError::AtUriParsingFailed {
                uri: "payload".to_string(),
                details: format!("Payload must be a string or object, got: {:?}", node.payload),
            }.into());
        };

        // Parse the AT-URI
        let (repository, collection, record_key) = Self::parse_aturi(&uri)?;

        // Build parsed result
        let parsed_result = json!({
            "repository": repository,
            "collection": collection,
            "record_key": record_key,
        });

        // Clone input and set destination field
        let mut output = input.clone();
        output[destination] = parsed_result;

        Ok(Some(output))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_aturi_valid() {
        // Use a valid DID format (DIDs must be at least 24 characters after the prefix)
        let uri = "at://did:plc:cbkjy5n7bk3ax2wplmtjofq2/app.bsky.feed.post/xyz789";
        let result = ParseAturiEvaluator::parse_aturi(uri);

        // Debug the error if it occurs
        if let Err(ref e) = result {
            eprintln!("Parse error: {}", e);
        }

        assert!(result.is_ok());
        let (repo, collection, rkey) = result.unwrap();
        assert_eq!(repo, "did:plc:cbkjy5n7bk3ax2wplmtjofq2");
        assert_eq!(collection, "app.bsky.feed.post");
        assert_eq!(rkey, "xyz789");
    }

    #[test]
    fn test_parse_aturi_with_complex_did() {
        let uri =
            "at://did:plc:lehcqqkwzcwvjvw66uthu5oq/community.lexicon.calendar.event/3lte3c7x43l2e";
        let result = ParseAturiEvaluator::parse_aturi(uri);
        assert!(result.is_ok());
        let (repo, collection, rkey) = result.unwrap();
        assert_eq!(repo, "did:plc:lehcqqkwzcwvjvw66uthu5oq");
        assert_eq!(collection, "community.lexicon.calendar.event");
        assert_eq!(rkey, "3lte3c7x43l2e");
    }

    #[test]
    fn test_parse_aturi_with_web_did() {
        let uri = "at://did:web:example.com/app.bsky.feed.like/abc123";
        let result = ParseAturiEvaluator::parse_aturi(uri);
        assert!(result.is_ok());
        let (repo, collection, rkey) = result.unwrap();
        assert_eq!(repo, "did:web:example.com");
        assert_eq!(collection, "app.bsky.feed.like");
        assert_eq!(rkey, "abc123");
    }

    #[test]
    fn test_parse_aturi_invalid_prefix() {
        let uri = "https://example.com/some/path";
        let result = ParseAturiEvaluator::parse_aturi(uri);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Invalid AT-URI format")
        );
    }

    #[test]
    fn test_parse_aturi_missing_component() {
        let uri = "at://did:plc:cbkjy5n7bk3ax2wplmtjofq2/app.bsky.feed.post";
        let result = ParseAturiEvaluator::parse_aturi(uri);
        assert!(result.is_err());
        // The ATURI parser will reject this as invalid format
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Invalid AT-URI format")
        );
    }

    #[test]
    fn test_parse_aturi_extra_components() {
        // Note: The ATURI parser actually accepts extra path components
        // This is valid in AT Protocol for referencing specific fields
        let uri = "at://did:plc:cbkjy5n7bk3ax2wplmtjofq2/app.bsky.feed.post/xyz789/extra";
        let result = ParseAturiEvaluator::parse_aturi(uri);
        // This should actually succeed as the library accepts paths with more components
        assert!(result.is_ok());
        let (repo, collection, rkey) = result.unwrap();
        assert_eq!(repo, "did:plc:cbkjy5n7bk3ax2wplmtjofq2");
        assert_eq!(collection, "app.bsky.feed.post");
        assert_eq!(rkey, "xyz789");
    }

    #[test]
    fn test_parse_aturi_empty_repository() {
        let uri = "at:///app.bsky.feed.post/xyz789";
        let result = ParseAturiEvaluator::parse_aturi(uri);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Invalid AT-URI format")
        );
    }

    #[test]
    fn test_parse_aturi_empty_collection() {
        let uri = "at://did:plc:cbkjy5n7bk3ax2wplmtjofq2//xyz789";
        let result = ParseAturiEvaluator::parse_aturi(uri);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Invalid AT-URI format")
        );
    }

    #[test]
    fn test_parse_aturi_empty_record_key() {
        let uri = "at://did:plc:cbkjy5n7bk3ax2wplmtjofq2/app.bsky.feed.post/";
        let result = ParseAturiEvaluator::parse_aturi(uri);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Invalid AT-URI format")
        );
    }

    #[tokio::test]
    async fn test_evaluate_string_payload() {
        let evaluator = ParseAturiEvaluator::new();

        let node = Node {
            aturi: "test".to_string(),
            blueprint: "test".to_string(),
            node_type: "parse_aturi".to_string(),
            payload: json!("uri"), // Use string payload to extract from "uri" field
            configuration: json!({}),
            created_at: chrono::Utc::now(),
        };

        let input = json!({
            "uri": "at://did:plc:lehcqqkwzcwvjvw66uthu5oq/community.lexicon.calendar.event/3lte3c7x43l2e",
            "other": "data"
        });

        let result = evaluator.evaluate(&node, &input).await.unwrap();
        assert!(result.is_some());

        let output = result.unwrap();
        // Check that input was cloned and destination field was set
        assert_eq!(output["other"], "data"); // Original field preserved
        assert_eq!(
            output["parsed_aturi"]["repository"],
            "did:plc:lehcqqkwzcwvjvw66uthu5oq"
        );
        assert_eq!(
            output["parsed_aturi"]["collection"],
            "community.lexicon.calendar.event"
        );
        assert_eq!(output["parsed_aturi"]["record_key"], "3lte3c7x43l2e");
    }

    #[tokio::test]
    async fn test_evaluate_object_payload() {
        let evaluator = ParseAturiEvaluator::new();

        let node = Node {
            aturi: "test".to_string(),
            blueprint: "test".to_string(),
            node_type: "parse_aturi".to_string(),
            payload: json!({"val": ["commit", "record", "subject", "uri"]}), // Use object payload with DataLogic
            configuration: json!({}),
            created_at: chrono::Utc::now(),
        };

        let input = json!({
            "did": "did:plc:cbkjy5n7bk3ax2wplmtjofq2",
            "time_us": 1758295749959492i64,
            "kind": "commit",
            "commit": {
                "rev": "3lz76ksqmyk2f",
                "operation": "update",
                "collection": "community.lexicon.calendar.rsvp",
                "rkey": "7T68ZD4HPRWHQ",
                "record": {
                    "$type": "community.lexicon.calendar.rsvp",
                    "createdAt": "2025-09-19T15:29:09.174Z",
                    "status": "community.lexicon.calendar.rsvp#going",
                    "subject": {
                        "cid": "bafyreia66bgnwqxxiq74c6w27xr3gwtiqeuklfs4a6jwulcaxbwb3rowiu",
                        "uri": "at://did:plc:lehcqqkwzcwvjvw66uthu5oq/community.lexicon.calendar.event/3lte3c7x43l2e"
                    }
                },
                "cid": "bafyreianss3teu5epefzl72q5sljmv6pnlxzgnh6ujqftazgue5eindliq"
            }
        });

        let result = evaluator.evaluate(&node, &input).await.unwrap();
        assert!(result.is_some());

        let output = result.unwrap();
        // Check that input was cloned
        assert_eq!(output["did"], "did:plc:cbkjy5n7bk3ax2wplmtjofq2");
        assert_eq!(output["kind"], "commit");

        // Check parsed result
        assert_eq!(
            output["parsed_aturi"]["repository"],
            "did:plc:lehcqqkwzcwvjvw66uthu5oq"
        );
        assert_eq!(
            output["parsed_aturi"]["collection"],
            "community.lexicon.calendar.event"
        );
        assert_eq!(output["parsed_aturi"]["record_key"], "3lte3c7x43l2e");
    }

    #[tokio::test]
    async fn test_evaluate_custom_destination() {
        let evaluator = ParseAturiEvaluator::new();

        let node = Node {
            aturi: "test".to_string(),
            blueprint: "test".to_string(),
            node_type: "parse_aturi".to_string(),
            payload: json!("uri"),
            configuration: json!({"destination": "parsed"}),
            created_at: chrono::Utc::now(),
        };

        let input = json!({
            "uri": "at://did:plc:cbkjy5n7bk3ax2wplmtjofq2/app.bsky.feed.post/xyz789"
        });

        let result = evaluator.evaluate(&node, &input).await.unwrap();
        assert!(result.is_some());

        let output = result.unwrap();
        // Check custom destination field
        assert_eq!(
            output["parsed"]["repository"],
            "did:plc:cbkjy5n7bk3ax2wplmtjofq2"
        );
        assert_eq!(output["parsed"]["collection"], "app.bsky.feed.post");
        assert_eq!(output["parsed"]["record_key"], "xyz789");
    }

    #[tokio::test]
    async fn test_evaluate_missing_field() {
        let evaluator = ParseAturiEvaluator::new();

        let node = Node {
            aturi: "test".to_string(),
            blueprint: "test".to_string(),
            node_type: "parse_aturi".to_string(),
            payload: json!("missing_field"),
            configuration: json!({}),
            created_at: chrono::Utc::now(),
        };

        let input = json!({"uri": "at://did:plc:abc123/app.bsky.feed.post/xyz789"});

        let result = evaluator.evaluate(&node, &input).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Field 'missing_field' not found")
        );
    }

    #[tokio::test]
    async fn test_evaluate_invalid_uri() {
        let evaluator = ParseAturiEvaluator::new();

        let node = Node {
            aturi: "test".to_string(),
            blueprint: "test".to_string(),
            node_type: "parse_aturi".to_string(),
            payload: json!("uri"),
            configuration: json!({}),
            created_at: chrono::Utc::now(),
        };

        let input = json!({"uri": "https://example.com/not/an/aturi"});

        let result = evaluator.evaluate(&node, &input).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Invalid AT-URI format")
        );
    }

    #[tokio::test]
    async fn test_evaluate_non_string_payload_field() {
        let evaluator = ParseAturiEvaluator::new();

        let node = Node {
            aturi: "test".to_string(),
            blueprint: "test".to_string(),
            node_type: "parse_aturi".to_string(),
            payload: json!("number_field"),
            configuration: json!({}),
            created_at: chrono::Utc::now(),
        };

        let input = json!({"number_field": 123});

        let result = evaluator.evaluate(&node, &input).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Field 'number_field' not found or not a string")
        );
    }

    #[tokio::test]
    async fn test_evaluate_invalid_payload_type() {
        let evaluator = ParseAturiEvaluator::new();

        let node = Node {
            aturi: "test".to_string(),
            blueprint: "test".to_string(),
            node_type: "parse_aturi".to_string(),
            payload: json!(true), // Boolean payload
            configuration: json!({}),
            created_at: chrono::Utc::now(),
        };

        let input = json!({"uri": "at://did:plc:abc123/app.bsky.feed.post/xyz789"});

        let result = evaluator.evaluate(&node, &input).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Payload must be a string or object")
        );
    }

    #[tokio::test]
    async fn test_evaluate_datalogic_non_string_result() {
        let evaluator = ParseAturiEvaluator::new();

        let node = Node {
            aturi: "test".to_string(),
            blueprint: "test".to_string(),
            node_type: "parse_aturi".to_string(),
            payload: json!({"val": ["count"]}), // Will return a number
            configuration: json!({}),
            created_at: chrono::Utc::now(),
        };

        let input = json!({"count": 42});

        let result = evaluator.evaluate(&node, &input).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("DataLogic evaluation must return a string")
        );
    }

    #[tokio::test]
    async fn test_default_destination() {
        let evaluator = ParseAturiEvaluator::new();

        let node = Node {
            aturi: "test".to_string(),
            blueprint: "test".to_string(),
            node_type: "parse_aturi".to_string(),
            payload: json!("uri"),
            configuration: json!({}), // No destination specified, should default to "parsed_aturi"
            created_at: chrono::Utc::now(),
        };

        let input = json!({
            "uri": "at://did:plc:cbkjy5n7bk3ax2wplmtjofq2/app.bsky.feed.post/xyz789",
            "original": "data"
        });

        let result = evaluator.evaluate(&node, &input).await.unwrap();
        assert!(result.is_some());

        let output = result.unwrap();
        // Check original input is preserved
        assert_eq!(output["original"], "data");
        // Check default destination "parsed_aturi" is used
        assert_eq!(
            output["parsed_aturi"]["repository"],
            "did:plc:cbkjy5n7bk3ax2wplmtjofq2"
        );
        assert_eq!(output["parsed_aturi"]["collection"], "app.bsky.feed.post");
        assert_eq!(output["parsed_aturi"]["record_key"], "xyz789");
    }
}
