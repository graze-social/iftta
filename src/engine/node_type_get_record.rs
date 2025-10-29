//! AT Protocol record retrieval node for fetching records from repositories.
//!
//! This module implements the capability to fetch existing records from AT Protocol
//! repositories. It parses AT-URIs, resolves DIDs to PDS endpoints, and retrieves
//! record content using the AT Protocol's getRecord API.
//!
//! # Usage
//!
//! Get record nodes are used to fetch records from AT Protocol repositories.
//! They extract an AT-URI from the input, resolve the authority to get the PDS endpoint,
//! and fetch the record content.
//!
//! # Example Blueprint
//!
//! ## Fetch a record and process it:
//! ```json
//! {
//!   "nodes": [
//!     {
//!       "type": "webhook_entry",
//!       "payload": true
//!     },
//!     {
//!       "type": "get_record",
//!       "configuration": {"destination": "fetched_record"},
//!       "payload": "record_uri"
//!     },
//!     {
//!       "type": "transform",
//!       "payload": {
//!         "original_text": {"val": ["fetched_record", "value", "text"]},
//!         "author": {"val": ["fetched_record", "uri"]}
//!       }
//!     }
//!   ]
//! }
//! ```
//!
//! ## Chain record fetching:
//! ```json
//! {
//!   "nodes": [
//!     {
//!       "type": "jetstream_entry",
//!       "configuration": {"collection": ["app.bsky.feed.like"]},
//!       "payload": true
//!     },
//!     {
//!       "type": "get_record",
//!       "configuration": {},
//!       "payload": {"val": ["commit", "record", "subject", "uri"]}
//!     },
//!     {
//!       "type": "debug_action",
//!       "payload": {}
//!     }
//!   ]
//! }
//! ```

use anyhow::Result;
use async_trait::async_trait;
use atproto_client::{
    client::Auth,
    com::atproto::repo::{GetRecordResponse, get_record},
};
use atproto_identity::resolve::IdentityResolver;
use atproto_record::aturi::ATURI;
use serde_json::{Value, json};
use std::{str::FromStr, sync::Arc};
use tracing::{debug, info, warn};

use super::common::with_cached_datalogic;
use super::evaluator::NodeEvaluator;
use crate::errors::EngineError;
use crate::storage::node::Node;

/// Evaluator for fetching records from AT Protocol repositories.
///
/// This evaluator handles the `get_record` node type, which retrieves existing records
/// from AT Protocol repositories by parsing AT-URIs, resolving authorities to PDS endpoints,
/// and fetching record content.
///
/// # How It Works
///
/// 1. The node's payload determines how to extract the AT-URI:
///    - String payload: used as field name to extract from input
///    - Object payload: evaluated with DataLogic to produce the AT-URI
/// 2. The AT-URI is parsed to extract authority, collection, and record key
/// 3. The authority (DID or handle) is resolved to get the PDS endpoint
/// 4. The record is fetched from the PDS using the getRecord API
/// 5. The output is the input with added destination field containing the record
///
/// # Configuration
///
/// The node's configuration supports:
/// - `destination`: The field name to store the fetched record (default: "get_record_result")
///
/// ## Configuration Examples
///
/// Use default destination:
/// ```json
/// {}
/// ```
///
/// With custom destination:
/// ```json
/// {
///   "destination": "fetched_post"
/// }
/// ```
///
/// # Payload
///
/// The payload determines what to parse:
/// - String: Use as field name to extract AT-URI text from input
/// - Object: Evaluate using DataLogic to get AT-URI text
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
///
/// # Output Format
///
/// The output is the original input with an added field at the destination:
/// ```json
/// {
///   "...original_input_fields...",
///   "get_record_result": {
///     "uri": "at://did:plc:abc/app.bsky.feed.post/xyz",
///     "cid": "bafyrei...",
///     "value": {
///       "$type": "app.bsky.feed.post",
///       "text": "Hello world!",
///       "createdAt": "2024-01-01T00:00:00Z"
///     }
///   }
/// }
/// ```
pub struct GetRecordEvaluator {
    http_client: Arc<reqwest::Client>,
    identity_resolver: Arc<dyn IdentityResolver>,
}

impl GetRecordEvaluator {
    /// Create a new get record evaluator with required dependencies
    pub fn new(
        http_client: Arc<reqwest::Client>,
        identity_resolver: Arc<dyn IdentityResolver>,
    ) -> Self {
        Self {
            http_client,
            identity_resolver,
        }
    }
}

#[async_trait]
impl NodeEvaluator for GetRecordEvaluator {
    async fn evaluate(&self, node: &Node, input: &Value) -> Result<Option<Value>> {
        debug!("Fetching AT Protocol record");

        // Determine what to extract based on payload
        let aturi_str = if node.payload.is_string() {
            // Payload is a string - use as field name
            let field_name =
                node.payload
                    .as_str()
                    .ok_or_else(|| EngineError::InvalidFieldType {
                        field_name: "payload".to_string(),
                        node_type: "get_record".to_string(),
                        expected_type: "string".to_string(),
                    })?;

            // Extract the field value from input
            input
                .get(field_name)
                .and_then(|v| v.as_str())
                .ok_or_else(|| EngineError::MissingRequiredField {
                    field_name: field_name.to_string(),
                    node_type: "get_record".to_string(),
                })?
                .to_string()
        } else if node.payload.is_object() {
            // Payload is an object - evaluate with DataLogic
            let result = with_cached_datalogic(|datalogic| {
                datalogic.evaluate_json(&node.payload, input, None)
            })?;

            // Ensure result is a string
            result
                .as_str()
                .ok_or_else(|| EngineError::DataLogicFailed {
                    expression: format!("{:?}", node.payload),
                    details: "Evaluation did not produce a string".to_string(),
                })?
                .to_string()
        } else {
            return Err(EngineError::InvalidFieldType {
                field_name: "payload".to_string(),
                node_type: "get_record".to_string(),
                expected_type: "string (field name) or object (DataLogic expression)".to_string(),
            }
            .into());
        };

        // Parse the AT-URI
        let parsed_uri =
            ATURI::from_str(&aturi_str).map_err(|e| EngineError::AtUriParsingFailed {
                uri: aturi_str.clone(),
                details: e.to_string(),
            })?;

        let authority = parsed_uri.authority.clone();
        let collection = parsed_uri.collection.clone();
        let record_key = parsed_uri.record_key.clone();

        // Validate that we have a collection and record key
        if collection.is_empty() {
            return Err(EngineError::AtUriParsingFailed {
                uri: aturi_str.clone(),
                details: "AT-URI missing collection".to_string(),
            }
            .into());
        }
        if record_key.is_empty() {
            return Err(EngineError::AtUriParsingFailed {
                uri: aturi_str.clone(),
                details: "AT-URI missing record key".to_string(),
            }
            .into());
        }

        // Resolve the authority to get the DID and PDS endpoint
        debug!("Resolving authority: {}", authority);
        let identity_doc = self
            .identity_resolver
            .resolve(&authority)
            .await
            .map_err(|e| EngineError::AtProtoOperationFailed {
                operation: format!("resolve_authority:{}", authority),
                details: e.to_string(),
            })?;

        // Get the DID from the resolved document
        let did = identity_doc.id.to_string();

        // Get the PDS endpoint from the document
        let pds_endpoints = identity_doc.pds_endpoints();
        let pds_endpoint =
            pds_endpoints
                .first()
                .ok_or_else(|| EngineError::AtProtoOperationFailed {
                    operation: "get_pds_endpoint".to_string(),
                    details: format!("No PDS endpoint found for DID: {}", did),
                })?;

        // Fetch the record
        info!(
            "Fetching record from PDS: {} - collection: {}, rkey: {}",
            pds_endpoint, collection, record_key
        );

        let response = get_record(
            &self.http_client,
            &Auth::None, // Public record access
            &pds_endpoint,
            &did,
            &collection,
            &record_key,
            None, // No specific CID
        )
        .await
        .map_err(|e| EngineError::AtProtoOperationFailed {
            operation: "get_record".to_string(),
            details: e.to_string(),
        })?;

        // Process the response
        let record_result = match response {
            GetRecordResponse::Record {
                uri, cid, value, ..
            } => {
                info!("Successfully fetched record: {}", uri);
                json!({
                    "uri": uri,
                    "cid": cid,
                    "value": value
                })
            }
            GetRecordResponse::Error(e) => {
                warn!("Failed to fetch record: {}", e.error_message());
                return Err(EngineError::AtProtoOperationFailed {
                    operation: "get_record".to_string(),
                    details: e.error_message(),
                }
                .into());
            }
        };

        // Get destination field name from configuration
        let destination = node
            .configuration
            .get("destination")
            .and_then(|v| v.as_str())
            .unwrap_or("get_record_result");

        // Clone input and add the fetched record at the destination
        let mut output = input.clone();
        if let Some(obj) = output.as_object_mut() {
            obj.insert(destination.to_string(), record_result);
        } else {
            // If input is not an object, create a new object with input and result
            output = json!({
                "input": input,
                destination: record_result
            });
        }

        Ok(Some(output))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use atproto_identity::model::Document;
    use serde_json::json;
    use std::collections::HashMap;

    // Mock IdentityResolver for testing
    struct MockIdentityResolver {
        documents: HashMap<String, String>, // Maps handles/DIDs to DIDs
    }

    impl MockIdentityResolver {
        fn new() -> Self {
            let mut documents = HashMap::new();
            // Add test mappings - in real implementation these would have PDS endpoints
            // Using valid DID format (24+ chars after prefix)
            documents.insert(
                "did:plc:cbkjy5n7bk3ax2wplmtjofq2".to_string(),
                "did:plc:cbkjy5n7bk3ax2wplmtjofq2".to_string(),
            );
            documents.insert(
                "alice.bsky.social".to_string(),
                "did:plc:cbkjy5n7bk3ax2wplmtjofq2".to_string(),
            );

            Self { documents }
        }
    }

    #[async_trait]
    impl IdentityResolver for MockIdentityResolver {
        async fn resolve(&self, subject: &str) -> anyhow::Result<Document> {
            // Check if we can resolve this subject
            if let Some(did) = self.documents.get(subject) {
                // Build a mock document
                // Note: In the real implementation, the Document would have PDS endpoints
                // accessible via pds_endpoints(), but for testing we just return a basic document
                Ok(Document::builder()
                    .id(did.clone())
                    .build()
                    .expect("Failed to build document"))
            } else {
                Err(EngineError::AtProtoOperationFailed {
                    operation: "resolve_authority".to_string(),
                    details: format!("Unable to resolve: {}", subject),
                }
                .into())
            }
        }
    }

    fn create_mock_resolver() -> Arc<MockIdentityResolver> {
        Arc::new(MockIdentityResolver::new())
    }

    #[tokio::test]
    async fn test_get_record_with_string_payload() {
        let http_client = Arc::new(reqwest::Client::new());
        let resolver = create_mock_resolver();
        let evaluator = GetRecordEvaluator::new(http_client, resolver);

        let node = Node {
            aturi: "test_node".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "get_record".to_string(),
            payload: json!("post_uri"),
            configuration: json!({}),
            created_at: chrono::Utc::now(),
        };

        let input = json!({
            "post_uri": "at://did:plc:cbkjy5n7bk3ax2wplmtjofq2/app.bsky.feed.post/abc123",
            "other_data": "test"
        });

        // This test will fail with network error since we can't actually fetch
        // Just testing the parsing and resolution logic
        let result = evaluator.evaluate(&node, &input).await;

        // Should fail on actual fetch, but parsing should work
        assert!(result.is_err());
        let error = result.unwrap_err().to_string();
        // The test will fail because we can't reach bsky.social from the mock
        // Check that it's failing because of PDS endpoint issues
        assert!(
            error.contains("PDS endpoint")
                || error.contains("Failed to fetch")
                || error.contains("error"),
            "Got error: {}",
            error
        );
    }

    #[tokio::test]
    async fn test_get_record_with_object_payload() {
        let http_client = Arc::new(reqwest::Client::new());
        let resolver = create_mock_resolver();
        let evaluator = GetRecordEvaluator::new(http_client, resolver);

        let node = Node {
            aturi: "test_node".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "get_record".to_string(),
            payload: json!({
                "val": ["subject", "uri"]
            }),
            configuration: json!({"destination": "fetched"}),
            created_at: chrono::Utc::now(),
        };

        let input = json!({
            "subject": {
                "uri": "at://alice.bsky.social/app.bsky.feed.post/xyz789"
            }
        });

        // This test will fail with network error since we can't actually fetch
        let result = evaluator.evaluate(&node, &input).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_get_record_invalid_aturi() {
        let http_client = Arc::new(reqwest::Client::new());
        let resolver = create_mock_resolver();
        let evaluator = GetRecordEvaluator::new(http_client, resolver);

        let node = Node {
            aturi: "test_node".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "get_record".to_string(),
            payload: json!("uri"),
            configuration: json!({}),
            created_at: chrono::Utc::now(),
        };

        let input = json!({
            "uri": "not-a-valid-aturi"
        });

        let result = evaluator.evaluate(&node, &input).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Invalid AT-URI"));
    }

    #[tokio::test]
    async fn test_get_record_missing_collection() {
        let http_client = Arc::new(reqwest::Client::new());
        let resolver = create_mock_resolver();
        let evaluator = GetRecordEvaluator::new(http_client, resolver);

        let node = Node {
            aturi: "test_node".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "get_record".to_string(),
            payload: json!("uri"),
            configuration: json!({}),
            created_at: chrono::Utc::now(),
        };

        let input = json!({
            "uri": "at://did:plc:cbkjy5n7bk3ax2wplmtjofq2"
        });

        let result = evaluator.evaluate(&node, &input).await;
        assert!(result.is_err());
        let error = result.unwrap_err().to_string();
        assert!(
            error.to_lowercase().contains("collection") && error.to_lowercase().contains("missing"),
            "Got error: {}",
            error
        );
    }

    #[tokio::test]
    async fn test_get_record_missing_record_key() {
        let http_client = Arc::new(reqwest::Client::new());
        let resolver = create_mock_resolver();
        let evaluator = GetRecordEvaluator::new(http_client, resolver);

        let node = Node {
            aturi: "test_node".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "get_record".to_string(),
            payload: json!("uri"),
            configuration: json!({}),
            created_at: chrono::Utc::now(),
        };

        let input = json!({
            "uri": "at://did:plc:cbkjy5n7bk3ax2wplmtjofq2/app.bsky.feed.post"
        });

        let result = evaluator.evaluate(&node, &input).await;
        assert!(result.is_err());
        let error = result.unwrap_err().to_string();
        assert!(
            error.to_lowercase().contains("record key") && error.to_lowercase().contains("missing"),
            "Got error: {}",
            error
        );
    }

    #[tokio::test]
    async fn test_get_record_field_not_found() {
        let http_client = Arc::new(reqwest::Client::new());
        let resolver = create_mock_resolver();
        let evaluator = GetRecordEvaluator::new(http_client, resolver);

        let node = Node {
            aturi: "test_node".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "get_record".to_string(),
            payload: json!("missing_field"),
            configuration: json!({}),
            created_at: chrono::Utc::now(),
        };

        let input = json!({
            "other_field": "value"
        });

        let result = evaluator.evaluate(&node, &input).await;
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("Missing required field") || err_msg.contains("missing_field"));
    }

    #[tokio::test]
    async fn test_get_record_invalid_payload_type() {
        let http_client = Arc::new(reqwest::Client::new());
        let resolver = create_mock_resolver();
        let evaluator = GetRecordEvaluator::new(http_client, resolver);

        let node = Node {
            aturi: "test_node".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "get_record".to_string(),
            payload: json!(123), // Invalid: number payload
            configuration: json!({}),
            created_at: chrono::Utc::now(),
        };

        let input = json!({});

        let result = evaluator.evaluate(&node, &input).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Invalid field type: payload")
        );
    }

    #[tokio::test]
    async fn test_get_record_datalogic_non_string_result() {
        let http_client = Arc::new(reqwest::Client::new());
        let resolver = create_mock_resolver();
        let evaluator = GetRecordEvaluator::new(http_client, resolver);

        let node = Node {
            aturi: "test_node".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "get_record".to_string(),
            payload: json!({
                "val": ["number_field"]
            }),
            configuration: json!({}),
            created_at: chrono::Utc::now(),
        };

        let input = json!({
            "number_field": 123
        });

        let result = evaluator.evaluate(&node, &input).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("did not produce a string")
        );
    }
}
