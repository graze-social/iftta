//! AT Protocol record publishing node for creating records in repositories.
//!
//! This module implements the capability to create new records in AT Protocol
//! repositories. It handles authentication, PDS resolution, and record creation
//! for various record types including posts, likes, follows, and custom records.
//!
//! # Important Changes
//!
//! The publish_record node now uses a simplified approach:
//! - Payload can be a string (field name) or object (DataLogic expression)
//! - The record's `$type` field determines the collection
//! - Configuration only supports optional `record_key` field
//! - Output includes the original input with added `publish_record_results`
//!
//! # Usage
//!
//! Publish record nodes are action nodes that create new records in AT Protocol
//! repositories. They require an authenticated session for the target DID and
//! properly formatted record data.
//!
//! # Example Blueprints
//!
//! ## Auto-like posts containing a keyword:
//! ```json
//! {
//!   "nodes": [
//!     {
//!       "type": "jetstream_entry",
//!       "configuration": {"collection": ["app.bsky.feed.post"]},
//!       "payload": {
//!         "contains": [{"val": ["commit", "record", "text"]}, "#ifthisthenat"]
//!       }
//!     },
//!     {
//!       "type": "transform",
//!       "payload": {
//!         "record": {
//!           "$type": "app.bsky.feed.like",
//!           "subject": {
//!             "uri": {
//!               "cat": [
//!                 "at://",
//!                 {"val": ["did"]},
//!                 "/app.bsky.feed.post/",
//!                 {"val": ["commit", "rkey"]}
//!               ]
//!             },
//!             "cid": {"val": ["commit", "cid"]}
//!           },
//!           "createdAt": {"now": []}
//!         }
//!       }
//!     },
//!     {
//!       "type": "publish_record",
//!       "configuration": {},
//!       "payload": "record"  // Extract "record" field from input
//!     }
//!   ]
//! }
//! ```
//!
//! ## Create a post with facets:
//! ```json
//! {
//!   "nodes": [
//!     {
//!       "type": "webhook_entry",
//!       "payload": true
//!     },
//!     {
//!       "type": "facet_text",
//!       "configuration": {"field": "text"},
//!       "payload": {}
//!     },
//!     {
//!       "type": "transform",
//!       "payload": {
//!         "record": {
//!           "$type": "app.bsky.feed.post",
//!           "text": {"val": ["text"]},
//!           "facets": {"val": ["facets"]},
//!           "createdAt": {"now": []},
//!           "langs": ["en"]
//!         }
//!       }
//!     },
//!     {
//!       "type": "publish_record",
//!       "configuration": {},
//!       "payload": "record"  // Extract "record" field from input
//!     }
//!   ]
//! }
//! ```
//!
//! ## Data flow example:
//! ```json
//! // Input to transform node:
//! {
//!   "text": "Hello @alice.bsky.social!",
//!   "facets": [
//!     {
//!       "index": {"byteStart": 6, "byteEnd": 24},
//!       "features": [{"$type": "app.bsky.richtext.facet#mention", "did": "did:plc:..."}]
//!     }
//!   ]
//! }
//!
//! // Output from transform node (input to publish_record):
//! {
//!   "record": {
//!     "$type": "app.bsky.feed.post",
//!     "text": "Hello @alice.bsky.social!",
//!     "facets": [...],
//!     "createdAt": "2024-01-01T12:00:00.000Z",
//!     "langs": ["en"]
//!   }
//! }
//!
//! // The publish_record node extracts the record and publishes it
//! // The output includes the original input plus publish_record_results:
//! // {
//! //   "record": {...},
//! //   "publish_record_results": {
//! //     "uri": "at://did:plc:.../app.bsky.feed.post/...",
//! //     "cid": "bafyrei..."
//! //   }
//! // }
//! ```

use anyhow::Result;
use async_trait::async_trait;
use atproto_client::{
    client::Auth,
    com::atproto::repo::{
        CreateRecordRequest, CreateRecordResponse, PutRecordRequest, PutRecordResponse,
        create_record, put_record,
    },
};
use atproto_record::aturi::ATURI;
use serde_json::Value;
use std::{str::FromStr, sync::Arc};
use tracing::{Instrument, debug, error, info, instrument};

use crate::{atproto::auth::ServiceAccessTokenManager, errors::EngineError, storage::node::Node};

use super::evaluator::NodeEvaluator;
use super::node_type_common::{clone_input_with_results, extract_payload_data};

/// Evaluator for publishing records to AT Protocol repositories.
///
/// This evaluator handles the `publish_record` node type, which creates new records
/// in AT Protocol repositories using authenticated sessions. It supports all
/// AT Protocol record types and handles authentication via stored sessions.
///
/// # How It Works
///
/// 1. The node's payload determines how to extract the record:
///    - String payload: used as field name to extract from input
///    - Object payload: evaluated with DataLogic to produce the record
/// 2. The record must be an object with a `$type` field (determines collection)
/// 3. The record is published using the authenticated session
/// 4. The output is the input with added `publish_record_results` field
///
/// # Configuration
///
/// The node's configuration is an object that may contain:
/// - `record_key`: Optional string specifying the record key (auto-generated if not provided)
///
/// ## Configuration Examples
///
/// Default (auto-generated key):
/// ```json
/// {}
/// ```
///
/// With specific record key:
/// ```json
/// {
///   "record_key": "custom-key-123"
/// }
/// ```
///
/// # Authentication
///
/// Authentication is configured when the evaluator is created, not per-node.
/// The service can be configured with either:
/// - A DID (which will use the most recent session for that DID)
/// - An AIP access token
///
/// The evaluator handles the OAuth flow with the AIP instance to obtain
/// the necessary AT Protocol session for publishing records.
///
/// # Payload
///
/// The payload can be:
/// 1. **String**: Field name to extract the record from input
/// 2. **Object**: DataLogic expression that evaluates to the record object
///
/// ## String Payload Examples
///
/// Extract from "record" field:
/// ```json
/// "record"
/// ```
///
/// Extract from nested field:
/// ```json
/// "post_data"
/// ```
///
/// ## Object Payload Examples
///
/// Pass through entire input:
/// ```json
/// {"val": []}
/// ```
///
/// Extract specific field:
/// ```json
/// {"val": ["record"]}
/// ```
///
/// Build record with DataLogic:
/// ```json
/// {
///   "$type": "app.bsky.feed.post",
///   "text": {"val": ["message"]},
///   "createdAt": {"now": []}
/// }
/// ```
///
/// # Record Requirements
///
/// The extracted/evaluated record must be an object containing:
/// - `$type`: The record type/collection (e.g., "app.bsky.feed.post") (required)
/// - Other fields specific to the record type
///
/// If the record is not an object or missing `$type`, an error is returned.
///
/// # Output Format
///
/// The output is the original input with an added field:
/// ```json
/// {
///   "...original_input_fields...",
///   "publish_record_results": {
///     "uri": "at://did:plc:abc/app.bsky.feed.post/xyz",
///     "cid": "bafyrei..."
///   }
/// }
/// ```
///
/// # Authentication
///
/// Records are published using stored sessions. Users must authenticate
/// through the web interface to create sessions for their DIDs.
///
/// # Record Types
///
/// Common AT Protocol record types:
/// - `app.bsky.feed.post`: Text posts with optional facets
/// - `app.bsky.feed.like`: Likes on posts
/// - `app.bsky.feed.repost`: Reposts of posts
/// - `app.bsky.graph.follow`: Follow relationships
/// - `app.bsky.graph.block`: Block relationships
/// - Custom app-specific record types
pub struct PublishRecordEvaluator {
    http_client: Arc<reqwest::Client>,
    service_token_manager: Arc<ServiceAccessTokenManager>,
}

impl PublishRecordEvaluator {
    /// Create a new evaluator with a ServiceAccessTokenManager
    pub fn new(
        http_client: Arc<reqwest::Client>,
        service_token_manager: Arc<ServiceAccessTokenManager>,
    ) -> Self {
        Self {
            http_client,
            service_token_manager,
        }
    }

    /// Get the authenticated session for publishing records.
    ///
    /// This method handles the OAuth flow with the AIP instance:
    /// - If configured with ServiceToken, uses the ServiceAccessTokenManager
    /// - If configured with a DID, looks up the most recent session and uses its AIP token
    /// - Exchanges the AIP token for an AT Protocol OAuth session
    async fn get_authenticated_session(&self, did: &str) -> Result<(String, String, Auth)> {
        // Look up the most recent session for this DID
        let (auth, session) = self.service_token_manager.auth(&did).await?;

        Ok((session.did, session.pds_endpoint, auth))
    }
}

#[async_trait]
impl NodeEvaluator for PublishRecordEvaluator {
    #[instrument(skip(self, input), fields(
        node.type = %node.node_type,
        node.aturi = %node.aturi,
    ))]
    async fn evaluate(&self, node: &Node, input: &Value) -> Result<Option<Value>> {
        debug!("Publishing AT Protocol record");

        let blueprint_aturi = ATURI::from_str(&node.blueprint)?;

        // Apply payload logic to get the record data
        let mut record_data = extract_payload_data(node, input)?;

        // If record_data is a string, try to parse it as JSON
        if let Some(record_str) = record_data.as_str() {
            debug!("Record data is a string, attempting to parse as JSON");
            record_data =
                serde_json::from_str(record_str).map_err(|e| EngineError::RecordParsingFailed {
                    details: format!(
                        "Failed to parse record data as JSON: {}. Input: {}",
                        e, record_str
                    ),
                })?;
        }

        // Ensure the record data is an object
        if !record_data.is_object() {
            return Err(EngineError::RecordParsingFailed {
                details: format!("Record data must be an object, got: {:?}", record_data),
            }
            .into());
        }

        // Extract required fields from record data
        let record_obj = record_data.as_object().unwrap();

        // Get $type (collection) from record
        let collection = record_obj
            .get("$type")
            .and_then(|v| v.as_str())
            .ok_or_else(|| EngineError::MissingRequiredField {
                field_name: "$type".to_string(),
                node_type: "publish_record".to_string(),
            })?;

        // Get authenticated session from evaluator configuration
        let (did, pds_endpoint, auth) = self
            .get_authenticated_session(&blueprint_aturi.authority)
            .await?;

        // Extract record_key from configuration (optional)
        let record_key = node
            .configuration
            .get("record_key")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        // Decide whether to create or update based on record_key presence
        let (uri, cid) = if let Some(rkey) = record_key {
            // Use put_record when record_key is specified
            let put_request = PutRecordRequest {
                repo: did.clone(),
                collection: collection.to_string(),
                record_key: rkey.clone(),
                validate: false,
                record: record_obj.clone(),
                swap_commit: None,
                swap_record: None,
            };

            let span = tracing::info_span!("put_record",
                repo = %did,
                collection = %collection,
                pds_endpoint = %pds_endpoint,
                rkey = %rkey
            );

            let response = put_record(&self.http_client, &auth, &pds_endpoint, put_request)
                .instrument(span)
                .await?;

            match response {
                PutRecordResponse::StrongRef { uri, cid, extra: _ } => {
                    info!(
                        uri = %uri,
                        cid = %cid,
                        "Successfully updated record"
                    );
                    (uri, cid)
                }
                PutRecordResponse::Error(e) => {
                    error!(
                        error = %e.error_message(),
                        "Failed to update record"
                    );
                    return Err(EngineError::AtProtoOperationFailed {
                        operation: "put_record".to_string(),
                        details: e.error_message(),
                    }
                    .into());
                }
            }
        } else {
            // Use create_record when no record_key is specified
            let create_request = CreateRecordRequest {
                repo: did.clone(),
                collection: collection.to_string(),
                record_key: None,
                validate: false,
                record: record_obj.clone(),
                swap_commit: None,
            };

            let span = tracing::info_span!("create_record",
                repo = %did,
                collection = %collection,
                pds_endpoint = %pds_endpoint
            );

            let response = create_record(&self.http_client, &auth, &pds_endpoint, create_request)
                .instrument(span)
                .await?;

            match response {
                CreateRecordResponse::StrongRef { uri, cid, extra: _ } => {
                    info!(
                        uri = %uri,
                        cid = %cid,
                        "Successfully created record"
                    );
                    (uri, cid)
                }
                CreateRecordResponse::Error(e) => {
                    error!(
                        error = %e.error_message(),
                        "Failed to create record"
                    );
                    return Err(EngineError::AtProtoOperationFailed {
                        operation: "create_record".to_string(),
                        details: e.error_message(),
                    }
                    .into());
                }
            }
        };

        // Clone input and add publish_record_results
        let result = clone_input_with_results(
            input,
            "publish_record_results",
            serde_json::json!({
                "uri": uri,
                "cid": cid,
            }),
        );

        Ok(Some(result))
    }
}
