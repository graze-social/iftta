//! Facet text node for parsing mentions, URLs, and hashtags to create AT Protocol facets.
//!
//! This module implements text processing that extracts mentions, URLs, and hashtags from
//! text content and creates proper AT Protocol facets. Facets enable rich text
//! features like clickable mentions, links, and hashtags in AT Protocol applications.
//!
//! # Byte Offset Calculation
//!
//! This implementation correctly uses UTF-8 byte offsets as required by AT Protocol.
//! The facets use "inclusive start and exclusive end" byte ranges. All parsing is done
//! using `regex::bytes::Regex` which operates on byte slices and returns byte positions,
//! ensuring correct handling of multi-byte UTF-8 characters (emojis, CJK, accented chars).
//!
//! See: https://docs.bsky.app/docs/advanced-guides/post-richtext
//!
//! # Usage
//!
//! Facet text nodes are typically placed before publish_record nodes to enrich
//! text content with proper facets. They parse @mentions and URLs, resolve
//! handles to DIDs, and generate the facet structure required by AT Protocol.
//!
//! # Example Blueprint
//!
//! ```json
//! {
//!   "nodes": [
//!     {
//!       "node_type": "transform",
//!       "payload": {
//!         "content": "Hello @alice.bsky.social! Check out https://example.com"
//!       }
//!     },
//!     {
//!       "node_type": "facet_text",
//!       "configuration": {"destination": "processed"},
//!       "payload": "content"
//!     },
//!     {
//!       "node_type": "transform",
//!       "payload": {
//!         "record": {
//!           "$type": "app.bsky.feed.post",
//!           "text": {"val": ["processed", "text"]},
//!           "facets": {"val": ["processed", "facets"]},
//!           "createdAt": {"now": []}
//!         }
//!       }
//!     },
//!     {
//!       "node_type": "publish_record",
//!       "configuration": {"did": "did:plc:...", "collection": "app.bsky.feed.post"},
//!       "payload": {}
//!     }
//!   ]
//! }
//! ```
//!
//! # Output Structure
//!
//! The node clones the input and sets the destination field with a facet result object containing:
//! - `text`: The original text content
//! - `facets`: An array of facet objects with byte ranges and features
//!
//! Example with input `{"content": "Hello @alice.bsky.social!"}` and destination `"processed"`:
//! ```json
//! {
//!   "content": "Hello @alice.bsky.social!",
//!   "processed": {
//!     "text": "Hello @alice.bsky.social!",
//!     "facets": [
//!       {
//!         "index": {"byteStart": 6, "byteEnd": 24},
//!         "features": [{"$type": "app.bsky.richtext.facet#mention", "did": "did:plc:..."}]
//!       }
//!     ]
//!   }
//! }
//! ```

use anyhow::Result;
use async_trait::async_trait;
use atproto_identity::resolve::IdentityResolver;
use datalogic_rs::{ContextStack, Error as DataLogicError, Evaluator, Operator};
use regex::bytes::Regex;
use serde_json::{json, Value};
use std::sync::Arc;

use super::common::evaluate_json_logic;
use super::evaluator::NodeEvaluator;
use crate::storage::node::Node;

/// Evaluator for the facet_text node type.
///
/// This evaluator processes text to extract mentions and URLs, creating
/// AT Protocol facets that enable rich text features. It uses the
/// IdentityResolver to convert handles to DIDs for proper mention linking.
///
/// # Configuration
///
/// The node's configuration supports:
/// - `destination`: The field name to store the facet result (default: "text")
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
/// {"destination": "content"}
/// ```
///
/// # Payload
///
/// The payload field determines how to extract text:
/// - String: Use as field name to extract text from input
/// - Object: Evaluate using DataLogic to get text
///
/// ## Payload Examples
///
/// Extract from field:
/// ```json
/// "text"
/// ```
///
/// Extract using DataLogic:
/// ```json
/// {"val": ["record", "text"]}
/// ```
pub struct FacetTextEvaluator {
    identity_resolver: Arc<dyn IdentityResolver>,
}

impl FacetTextEvaluator {
    /// Creates a new FacetTextEvaluator with the given identity resolver.
    ///
    /// # Arguments
    ///
    /// * `identity_resolver` - Resolver for converting handles to DIDs
    pub fn new(identity_resolver: Arc<dyn IdentityResolver>) -> Self {
        Self { identity_resolver }
    }

    fn parse_tags(text: &str) -> Vec<TagSpan> {
        let mut spans = Vec::new();

        // Regex based on: https://github.com/bluesky-social/atproto/blob/d91988fe79030b61b556dd6f16a46f0c3b9d0b44/packages/api/src/rich-text/util.ts
        // Simplified for Rust - matches hashtags at word boundaries
        // Pattern matches: start of string or non-word char, then # or ＃, then tag content
        let tag_regex = Regex::new(r"(?:^|[^\w])([#＃])([\w]+(?:[\w]*)*)").unwrap();

        let text_bytes = text.as_bytes();

        // Work with bytes for proper position tracking
        for capture in tag_regex.captures_iter(text_bytes) {
            if let (Some(full_match), Some(hash_match), Some(tag_match)) =
                (capture.get(0), capture.get(1), capture.get(2))
            {
                // Calculate the absolute byte position of the hash symbol
                // The full match includes the preceding character (if any)
                // so we need to adjust for that
                let match_start = full_match.start();
                let hash_offset = hash_match.start() - full_match.start();
                let start = match_start + hash_offset;
                let end = match_start + hash_offset + hash_match.len() + tag_match.len();

                // Extract just the tag text (without the hash symbol)
                let tag = std::str::from_utf8(tag_match.as_bytes())
                    .unwrap_or_default()
                    .to_string();

                // Only include tags that are not purely numeric
                if !tag.chars().all(|c| c.is_ascii_digit()) {
                    spans.push(TagSpan { start, end, tag });
                }
            }
        }

        spans
    }

    /// Parse mentions from text and return their byte positions
    fn parse_mentions(text: &str) -> Vec<MentionSpan> {
        let mut spans = Vec::new();

        // Regex based on: https://atproto.com/specs/handle#handle-identifier-syntax
        // Pattern: [$|\W](@([a-zA-Z0-9]([a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?\.)+[a-zA-Z]([a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?)
        let mention_regex = Regex::new(
            r"(?:^|[^\w])(@([a-zA-Z0-9]([a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?\.)+[a-zA-Z]([a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?)"
        ).unwrap();

        let text_bytes = text.as_bytes();
        for capture in mention_regex.captures_iter(text_bytes) {
            if let Some(mention_match) = capture.get(1) {
                let handle = std::str::from_utf8(&mention_match.as_bytes()[1..])
                    .unwrap_or_default()
                    .to_string();

                spans.push(MentionSpan {
                    start: mention_match.start(),
                    end: mention_match.end(),
                    handle,
                });
            }
        }

        spans
    }

    /// Parse URLs from text and return their byte positions
    fn parse_urls(text: &str) -> Vec<UrlSpan> {
        let mut spans = Vec::new();

        // Partial/naive URL regex based on: https://stackoverflow.com/a/3809435
        // Pattern: [$|\W](https?:\/\/(www\.)?[-a-zA-Z0-9@:%._\+~#=]{1,256}\.[a-zA-Z0-9()]{1,6}\b([-a-zA-Z0-9()@:%_\+.~#?&//=]*[-a-zA-Z0-9@%_\+~#//=])?)
        let url_regex = Regex::new(
            r"(?:^|[^\w])(https?://(?:www\.)?[-a-zA-Z0-9@:%._\+~#=]{1,256}\.[a-zA-Z0-9()]{1,6}\b(?:[-a-zA-Z0-9()@:%_\+.~#?&//=]*[-a-zA-Z0-9@%_\+~#//=])?)"
        ).unwrap();

        let text_bytes = text.as_bytes();
        for capture in url_regex.captures_iter(text_bytes) {
            if let Some(url_match) = capture.get(1) {
                let url = std::str::from_utf8(url_match.as_bytes())
                    .unwrap_or_default()
                    .to_string();

                spans.push(UrlSpan {
                    start: url_match.start(),
                    end: url_match.end(),
                    url,
                });
            }
        }

        spans
    }
}

#[derive(Debug)]
struct MentionSpan {
    start: usize,
    end: usize,
    handle: String,
}

#[derive(Debug)]
struct UrlSpan {
    start: usize,
    end: usize,
    url: String,
}

#[derive(Debug)]
struct TagSpan {
    start: usize,
    end: usize,
    tag: String,
}

#[async_trait]
impl NodeEvaluator for FacetTextEvaluator {
    async fn evaluate(&self, node: &Node, input: &Value) -> Result<Option<Value>> {
        // Get the destination field name from configuration (default to "text")
        let destination = node
            .configuration
            .get("destination")
            .and_then(|v| v.as_str())
            .unwrap_or("text");

        // Extract the text based on payload type
        let text = if let Some(field_name) = node.payload.as_str() {
            // Payload is string - use as field name
            input
                .get(field_name)
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    anyhow::anyhow!("Field '{}' not found or not a string in input", field_name)
                })?
                .to_string()
        } else if node.payload.is_object() {
            // Payload is object - evaluate with DataLogic
            let result = evaluate_json_logic(true, &node.payload, input)?;
            if let Some(text_str) = result.as_str() {
                text_str.to_string()
            } else {
                return Err(anyhow::anyhow!(
                    "DataLogic evaluation must return a string, got: {:?}",
                    result
                ));
            }
        } else {
            return Err(anyhow::anyhow!(
                "Payload must be a string or object, got: {:?}",
                node.payload
            ));
        };

        // Parse mentions, URLs, and tags
        let mention_spans = Self::parse_mentions(&text);
        let url_spans = Self::parse_urls(&text);
        let tag_spans = Self::parse_tags(&text);

        // Build facets array
        let mut facets = Vec::new();

        // Process mentions - resolve handles to DIDs
        for mention in mention_spans {
            // Try to resolve the handle to a DID using the IdentityResolver
            // The resolver accepts both DIDs and handles as subjects
            match self.identity_resolver.resolve(&mention.handle).await {
                Ok(document) => {
                    // Extract the DID from the resolved document
                    facets.push(json!({
                        "index": {
                            "byteStart": mention.start,
                            "byteEnd": mention.end,
                        },
                        "features": [{
                            "$type": "app.bsky.richtext.facet#mention",
                            "did": document.id,
                        }],
                    }));
                }
                Err(e) => {
                    // If handle can't be resolved, skip it (will be rendered as plain text)
                    tracing::debug!(
                        handle = %mention.handle,
                        error = ?e,
                        "Failed to resolve handle for mention, skipping"
                    );
                }
            }
        }

        // Process URLs
        for url in url_spans {
            facets.push(json!({
                "index": {
                    "byteStart": url.start,
                    "byteEnd": url.end,
                },
                "features": [{
                    "$type": "app.bsky.richtext.facet#link",
                    "uri": url.url,
                }],
            }));
        }

        // Process tags (hashtags)
        for tag in tag_spans {
            facets.push(json!({
                "index": {
                    "byteStart": tag.start,
                    "byteEnd": tag.end,
                },
                "features": [{
                    "$type": "app.bsky.richtext.facet#tag",
                    "tag": tag.tag,
                }],
            }));
        }

        // Build facet result
        let facet_result = json!({
            "text": &text,
            "facets": facets,
        });

        // Clone input and set destination field
        let mut output = input.clone();
        output[destination] = facet_result;

        Ok(Some(output))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use atproto_identity::model::Document;
    use std::collections::HashMap;

    // Mock IdentityResolver for testing
    struct MockIdentityResolver {
        handles: HashMap<String, String>,
    }

    impl MockIdentityResolver {
        fn new() -> Self {
            let mut handles = HashMap::new();
            // Add some test handles
            handles.insert(
                "alice.bsky.social".to_string(),
                "did:plc:alice123".to_string(),
            );
            handles.insert("bob.test.com".to_string(), "did:plc:bob456".to_string());
            Self { handles }
        }
    }

    #[async_trait]
    impl IdentityResolver for MockIdentityResolver {
        async fn resolve(&self, subject: &str) -> anyhow::Result<Document> {
            // Normalize handle (lowercase)
            let normalized = if subject.starts_with("did:") {
                subject.to_string()
            } else {
                subject.to_lowercase()
            };

            // Check if it's a handle we know about
            if let Some(did) = self.handles.get(&normalized) {
                Ok(Document::builder()
                    .id(did.clone())
                    .build()
                    .expect("Failed to build document"))
            } else {
                Err(anyhow::anyhow!("Handle not found: {}", subject))
            }
        }
    }

    #[test]
    fn test_parse_tags() {
        // Test case from the comment
        let text = "Barack Obummer Complains About Kimmel and Cancel Culture? Here's a Look Back at Our Former Censor in Chief #RedState";
        let spans = FacetTextEvaluator::parse_tags(text);

        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].tag, "RedState");
        assert_eq!(spans[0].start, 107); // Position of #
        assert_eq!(spans[0].end, 116); // End of "#RedState"

        // Additional test cases
        let text2 = "#hello #world #123 #test123";
        let spans2 = FacetTextEvaluator::parse_tags(text2);

        assert_eq!(spans2.len(), 3); // Should exclude #123 as it's purely numeric
        assert_eq!(spans2[0].tag, "hello");
        assert_eq!(spans2[1].tag, "world");
        assert_eq!(spans2[2].tag, "test123");

        // Test with fullwidth hash
        let text3 = "Test ＃fullwidth tag";
        let spans3 = FacetTextEvaluator::parse_tags(text3);

        assert_eq!(spans3.len(), 1);
        assert_eq!(spans3[0].tag, "fullwidth");
    }

    #[test]
    fn test_parse_mentions() {
        let text = "Hello @alice.bsky.social and @bob.test.com!";
        let spans = FacetTextEvaluator::parse_mentions(text);

        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].handle, "alice.bsky.social");
        assert_eq!(spans[0].start, 6);
        assert_eq!(spans[0].end, 24);
        assert_eq!(spans[1].handle, "bob.test.com");
        assert_eq!(spans[1].start, 29);
        assert_eq!(spans[1].end, 42);
    }

    #[test]
    fn test_parse_urls() {
        let text = "Check out https://example.com and https://test.org/path?query=1";
        let spans = FacetTextEvaluator::parse_urls(text);

        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].url, "https://example.com");
        assert_eq!(spans[0].start, 10);
        assert_eq!(spans[0].end, 29);
        assert_eq!(spans[1].url, "https://test.org/path?query=1");
        assert_eq!(spans[1].start, 34);
        assert_eq!(spans[1].end, 63);
    }

    #[test]
    fn test_mention_at_start() {
        let text = "@alice.bsky.social is here";
        let spans = FacetTextEvaluator::parse_mentions(text);

        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].handle, "alice.bsky.social");
        assert_eq!(spans[0].start, 0);
        assert_eq!(spans[0].end, 18);
    }

    #[test]
    fn test_url_at_start() {
        let text = "https://example.com is a website";
        let spans = FacetTextEvaluator::parse_urls(text);

        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].url, "https://example.com");
        assert_eq!(spans[0].start, 0);
        assert_eq!(spans[0].end, 19);
    }

    #[tokio::test]
    async fn test_evaluate() {
        // Create mock identity resolver
        let resolver = Arc::new(MockIdentityResolver::new());
        let evaluator = FacetTextEvaluator::new(resolver);

        let node = Node {
            aturi: "test".to_string(),
            blueprint: "test".to_string(),
            node_type: "facet_text".to_string(),
            payload: json!("text"), // Use string payload to extract from "text" field
            configuration: json!({}),
            created_at: chrono::Utc::now(),
        };

        let input = json!({
            "text": "Hello @alice.bsky.social! Check out https://example.com",
            "other": "data"
        });

        let result = evaluator.evaluate(&node, &input).await.unwrap();
        assert!(result.is_some());

        let output = result.unwrap();
        // Check that input was cloned and destination field was set
        assert_eq!(output["other"], "data"); // Original field preserved
        assert_eq!(
            output["text"]["text"],
            "Hello @alice.bsky.social! Check out https://example.com"
        );
        assert!(output["text"]["facets"].is_array());

        // Should have found one mention and one URL
        let facets = output["text"]["facets"].as_array().unwrap();
        assert_eq!(facets.len(), 2);

        // Check mention facet
        assert_eq!(facets[0]["index"]["byteStart"], 6);
        assert_eq!(facets[0]["index"]["byteEnd"], 24);
        assert_eq!(
            facets[0]["features"][0]["$type"],
            "app.bsky.richtext.facet#mention"
        );
        assert_eq!(facets[0]["features"][0]["did"], "did:plc:alice123");

        // Check URL facet
        assert_eq!(facets[1]["index"]["byteStart"], 36);
        assert_eq!(facets[1]["index"]["byteEnd"], 55);
        assert_eq!(
            facets[1]["features"][0]["$type"],
            "app.bsky.richtext.facet#link"
        );
    }

    #[tokio::test]
    async fn test_evaluate_custom_field() {
        // Create mock identity resolver
        let resolver = Arc::new(MockIdentityResolver::new());
        let evaluator = FacetTextEvaluator::new(resolver);

        let node = Node {
            aturi: "test".to_string(),
            blueprint: "test".to_string(),
            node_type: "facet_text".to_string(),
            payload: json!("content"), // Use string payload to extract from "content" field
            configuration: json!({
                "destination": "processed"
            }),
            created_at: chrono::Utc::now(),
        };

        let input = json!({
            "content": "Visit https://test.org",
            "text": "This should be ignored"
        });

        let result = evaluator.evaluate(&node, &input).await.unwrap();
        assert!(result.is_some());

        let output = result.unwrap();
        // Check original input is preserved
        assert_eq!(output["text"], "This should be ignored");
        // Check destination field has the result
        assert_eq!(output["processed"]["text"], "Visit https://test.org");

        // Should have found one URL
        let facets = output["processed"]["facets"].as_array().unwrap();
        assert_eq!(facets.len(), 1);
    }

    #[tokio::test]
    async fn test_unresolvable_handle() {
        // Create mock identity resolver
        let resolver = Arc::new(MockIdentityResolver::new());
        let evaluator = FacetTextEvaluator::new(resolver);

        let node = Node {
            aturi: "test".to_string(),
            blueprint: "test".to_string(),
            node_type: "facet_text".to_string(),
            payload: json!("text"), // Use string payload
            configuration: json!({}),
            created_at: chrono::Utc::now(),
        };

        // Use a handle that won't resolve
        let input = json!({
            "text": "Hello @unknown.handle! Visit https://test.org"
        });

        let result = evaluator.evaluate(&node, &input).await.unwrap();
        assert!(result.is_some());

        let output = result.unwrap();
        assert_eq!(
            output["text"]["text"],
            "Hello @unknown.handle! Visit https://test.org"
        );

        // Should only have the URL facet, not the unresolved mention
        let facets = output["text"]["facets"].as_array().unwrap();
        assert_eq!(facets.len(), 1);
        assert_eq!(
            facets[0]["features"][0]["$type"],
            "app.bsky.richtext.facet#link"
        );
        assert_eq!(facets[0]["features"][0]["uri"], "https://test.org");
    }

    #[tokio::test]
    async fn test_evaluate_object_payload() {
        // Create mock identity resolver
        let resolver = Arc::new(MockIdentityResolver::new());
        let evaluator = FacetTextEvaluator::new(resolver);

        let node = Node {
            aturi: "test".to_string(),
            blueprint: "test".to_string(),
            node_type: "facet_text".to_string(),
            payload: json!({"val": ["record", "text"]}), // Use object payload with DataLogic
            configuration: json!({
                "destination": "facets_result"
            }),
            created_at: chrono::Utc::now(),
        };

        let input = json!({
            "record": {
                "text": "Hello @alice.bsky.social!"
            },
            "other": "data"
        });

        let result = evaluator.evaluate(&node, &input).await.unwrap();
        assert!(result.is_some());

        let output = result.unwrap();
        // Check original input is preserved
        assert_eq!(output["other"], "data");
        assert_eq!(output["record"]["text"], "Hello @alice.bsky.social!");

        // Check destination field has the result
        assert_eq!(output["facets_result"]["text"], "Hello @alice.bsky.social!");

        // Should have found one mention
        let facets = output["facets_result"]["facets"].as_array().unwrap();
        assert_eq!(facets.len(), 1);
        assert_eq!(
            facets[0]["features"][0]["$type"],
            "app.bsky.richtext.facet#mention"
        );
    }

    #[tokio::test]
    async fn test_invalid_payload_types() {
        let resolver = Arc::new(MockIdentityResolver::new());
        let evaluator = FacetTextEvaluator::new(resolver);
        let input = json!({"text": "test"});

        // Test with boolean payload
        let node_bool = Node {
            aturi: "test".to_string(),
            blueprint: "test".to_string(),
            node_type: "facet_text".to_string(),
            payload: json!(true),
            configuration: json!({}),
            created_at: chrono::Utc::now(),
        };

        let result = evaluator.evaluate(&node_bool, &input).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Payload must be a string or object")
        );

        // Test with number payload
        let node_number = Node {
            aturi: "test".to_string(),
            blueprint: "test".to_string(),
            node_type: "facet_text".to_string(),
            payload: json!(42),
            configuration: json!({}),
            created_at: chrono::Utc::now(),
        };

        let result = evaluator.evaluate(&node_number, &input).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Payload must be a string or object")
        );

        // Test with array payload
        let node_array = Node {
            aturi: "test".to_string(),
            blueprint: "test".to_string(),
            node_type: "facet_text".to_string(),
            payload: json!([1, 2, 3]),
            configuration: json!({}),
            created_at: chrono::Utc::now(),
        };

        let result = evaluator.evaluate(&node_array, &input).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Payload must be a string or object")
        );
    }

    #[tokio::test]
    async fn test_missing_field_string_payload() {
        let resolver = Arc::new(MockIdentityResolver::new());
        let evaluator = FacetTextEvaluator::new(resolver);

        let node = Node {
            aturi: "test".to_string(),
            blueprint: "test".to_string(),
            node_type: "facet_text".to_string(),
            payload: json!("missing_field"), // Field that doesn't exist
            configuration: json!({}),
            created_at: chrono::Utc::now(),
        };

        let input = json!({
            "text": "Hello world"
        });

        let result = evaluator.evaluate(&node, &input).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Field 'missing_field' not found or not a string")
        );
    }

    #[tokio::test]
    async fn test_non_string_field_value() {
        let resolver = Arc::new(MockIdentityResolver::new());
        let evaluator = FacetTextEvaluator::new(resolver);

        let node = Node {
            aturi: "test".to_string(),
            blueprint: "test".to_string(),
            node_type: "facet_text".to_string(),
            payload: json!("number_field"),
            configuration: json!({}),
            created_at: chrono::Utc::now(),
        };

        let input = json!({
            "number_field": 123 // Not a string
        });

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
    async fn test_datalogic_non_string_result() {
        let resolver = Arc::new(MockIdentityResolver::new());
        let evaluator = FacetTextEvaluator::new(resolver);

        let node = Node {
            aturi: "test".to_string(),
            blueprint: "test".to_string(),
            node_type: "facet_text".to_string(),
            payload: json!({"val": ["count"]}), // Will return a number
            configuration: json!({}),
            created_at: chrono::Utc::now(),
        };

        let input = json!({
            "count": 42
        });

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
        let resolver = Arc::new(MockIdentityResolver::new());
        let evaluator = FacetTextEvaluator::new(resolver);

        let node = Node {
            aturi: "test".to_string(),
            blueprint: "test".to_string(),
            node_type: "facet_text".to_string(),
            payload: json!("content"),
            configuration: json!({}), // No destination specified, should default to "text"
            created_at: chrono::Utc::now(),
        };

        let input = json!({
            "content": "Hello world",
            "original": "data"
        });

        let result = evaluator.evaluate(&node, &input).await.unwrap();
        assert!(result.is_some());

        let output = result.unwrap();
        // Check original input is preserved
        assert_eq!(output["original"], "data");
        // Check default destination "text" is used
        assert_eq!(output["text"]["text"], "Hello world");
        assert!(output["text"]["facets"].is_array());
    }

    #[tokio::test]
    async fn test_evaluate_with_tags() {
        // Create mock identity resolver
        let resolver = Arc::new(MockIdentityResolver::new());
        let evaluator = FacetTextEvaluator::new(resolver);

        let node = Node {
            aturi: "test".to_string(),
            blueprint: "test".to_string(),
            node_type: "facet_text".to_string(),
            payload: json!("content"), // Use string payload to extract from "content" field
            configuration: json!({"destination": "processed"}),
            created_at: chrono::Utc::now(),
        };

        let input = json!({
            "content": "Hello @alice.bsky.social! Check out https://example.com #rust #atproto",
            "other": "data"
        });

        let result = evaluator.evaluate(&node, &input).await.unwrap();
        assert!(result.is_some());

        let output = result.unwrap();
        // Check that input was cloned and destination field was set
        assert_eq!(output["other"], "data"); // Original field preserved
        assert_eq!(
            output["processed"]["text"],
            "Hello @alice.bsky.social! Check out https://example.com #rust #atproto"
        );
        assert!(output["processed"]["facets"].is_array());

        // Should have found one mention, one URL, and two tags
        let facets = output["processed"]["facets"].as_array().unwrap();
        assert_eq!(facets.len(), 4);

        // Check mention facet
        assert_eq!(facets[0]["index"]["byteStart"], 6);
        assert_eq!(facets[0]["index"]["byteEnd"], 24);
        assert_eq!(
            facets[0]["features"][0]["$type"],
            "app.bsky.richtext.facet#mention"
        );
        assert_eq!(facets[0]["features"][0]["did"], "did:plc:alice123");

        // Check URL facet
        assert_eq!(facets[1]["index"]["byteStart"], 36);
        assert_eq!(facets[1]["index"]["byteEnd"], 55);
        assert_eq!(
            facets[1]["features"][0]["$type"],
            "app.bsky.richtext.facet#link"
        );
        assert_eq!(facets[1]["features"][0]["uri"], "https://example.com");

        // Check first tag facet (#rust)
        assert_eq!(facets[2]["index"]["byteStart"], 56);
        assert_eq!(facets[2]["index"]["byteEnd"], 61);
        assert_eq!(
            facets[2]["features"][0]["$type"],
            "app.bsky.richtext.facet#tag"
        );
        assert_eq!(facets[2]["features"][0]["tag"], "rust");

        // Check second tag facet (#atproto)
        assert_eq!(facets[3]["index"]["byteStart"], 62);
        assert_eq!(facets[3]["index"]["byteEnd"], 70);
        assert_eq!(
            facets[3]["features"][0]["$type"],
            "app.bsky.richtext.facet#tag"
        );
        assert_eq!(facets[3]["features"][0]["tag"], "atproto");
    }

    #[test]
    fn test_utf8_byte_offsets() {
        // Test with multi-byte UTF-8 characters to ensure correct byte offset calculation
        // Emoji (4 bytes), accented characters (2 bytes), CJK characters (3 bytes)

        // Test mentions with emojis before
        let text1 = "Hello 🎉 @alice.bsky.social!";
        let spans1 = FacetTextEvaluator::parse_mentions(text1);
        assert_eq!(spans1.len(), 1);
        // "Hello " = 6 bytes, "🎉" = 4 bytes, " " = 1 byte, "@" starts at byte 11
        assert_eq!(spans1[0].start, 11);
        assert_eq!(spans1[0].end, 29); // 11 + 18 bytes for "@alice.bsky.social"
        assert_eq!(spans1[0].handle, "alice.bsky.social");

        // Test URLs with accented characters before
        let text2 = "Café résumé: https://example.com";
        let spans2 = FacetTextEvaluator::parse_urls(text2);
        assert_eq!(spans2.len(), 1);
        // "Café" = 5 bytes (é is 2 bytes), " résumé: " = 11 bytes (é is 2 bytes)
        assert_eq!(spans2[0].start, 16);
        assert_eq!(spans2[0].end, 35); // 16 + 19 bytes for the URL
        assert_eq!(spans2[0].url, "https://example.com");

        // Test tags with CJK characters before
        let text3 = "日本語 #rust programming";
        let spans3 = FacetTextEvaluator::parse_tags(text3);
        assert_eq!(spans3.len(), 1);
        // "日本語" = 9 bytes (3 bytes per character), " " = 1 byte
        assert_eq!(spans3[0].start, 10);
        assert_eq!(spans3[0].end, 15); // 10 + 5 bytes for "#rust"
        assert_eq!(spans3[0].tag, "rust");

        // Complex test with all types and multi-byte characters
        let text4 = "👋 Hello @alice.bsky.social! 日本語 https://example.com #atproto 🚀";
        let mention_spans = FacetTextEvaluator::parse_mentions(text4);
        let url_spans = FacetTextEvaluator::parse_urls(text4);
        let tag_spans = FacetTextEvaluator::parse_tags(text4);

        // "👋 " = 5 bytes (emoji 4 + space 1), "Hello " = 6 bytes
        assert_eq!(mention_spans[0].start, 11); // @alice starts at byte 11
        assert_eq!(mention_spans[0].end, 29); // ends at byte 29

        // After mention: "! 日本語 " = 12 bytes (! + space + 9 bytes CJK + space)
        assert_eq!(url_spans[0].start, 41); // URL starts at byte 41
        assert_eq!(url_spans[0].end, 60); // ends at byte 60

        // After URL: " " = 1 byte
        assert_eq!(tag_spans[0].start, 61); // #atproto starts at byte 61
        assert_eq!(tag_spans[0].end, 69); // ends at byte 69
    }

    #[tokio::test]
    async fn test_evaluate_with_utf8_characters() {
        // Test the full evaluate function with multi-byte UTF-8 characters
        let resolver = Arc::new(MockIdentityResolver::new());
        let evaluator = FacetTextEvaluator::new(resolver);

        let node = Node {
            aturi: "test".to_string(),
            blueprint: "test".to_string(),
            node_type: "facet_text".to_string(),
            payload: json!("text"),
            configuration: json!({}),
            created_at: chrono::Utc::now(),
        };

        let input = json!({
            "text": "Hello 🎉 @alice.bsky.social! Check 日本語 https://example.com #rust 🚀"
        });

        let result = evaluator.evaluate(&node, &input).await.unwrap();
        assert!(result.is_some());

        let output = result.unwrap();
        let facets = output["text"]["facets"].as_array().unwrap();

        // Should have found one mention, one URL, and one tag
        assert_eq!(facets.len(), 3);

        // Check mention facet - starts after "Hello 🎉 " (11 bytes: "Hello " = 6, emoji = 4, space = 1)
        assert_eq!(facets[0]["index"]["byteStart"], 11);
        assert_eq!(facets[0]["index"]["byteEnd"], 29);
        assert_eq!(
            facets[0]["features"][0]["$type"],
            "app.bsky.richtext.facet#mention"
        );

        // Check URL facet - starts after mention and "! Check 日本語 "
        // "! Check " = 8 bytes, "日本語" = 9 bytes, " " = 1 byte = 18 bytes after mention
        assert_eq!(facets[1]["index"]["byteStart"], 47);
        assert_eq!(facets[1]["index"]["byteEnd"], 66);
        assert_eq!(
            facets[1]["features"][0]["$type"],
            "app.bsky.richtext.facet#link"
        );

        // Check tag facet - starts after URL and " " (1 byte)
        assert_eq!(facets[2]["index"]["byteStart"], 67);
        assert_eq!(facets[2]["index"]["byteEnd"], 72);
        assert_eq!(
            facets[2]["features"][0]["$type"],
            "app.bsky.richtext.facet#tag"
        );
    }

    #[tokio::test]
    async fn test_empty_text() {
        let resolver = Arc::new(MockIdentityResolver::new());
        let evaluator = FacetTextEvaluator::new(resolver);

        let node = Node {
            aturi: "test".to_string(),
            blueprint: "test".to_string(),
            node_type: "facet_text".to_string(),
            payload: json!("text"),
            configuration: json!({}),
            created_at: chrono::Utc::now(),
        };

        let input = json!({
            "text": ""
        });

        let result = evaluator.evaluate(&node, &input).await.unwrap();
        assert!(result.is_some());

        let output = result.unwrap();
        assert_eq!(output["text"]["text"], "");

        // Should have no facets for empty text
        let facets = output["text"]["facets"].as_array().unwrap();
        assert_eq!(facets.len(), 0);
    }
}

/// Custom DataLogic operator for facet text processing.
///
/// The `facet_text` operator takes a text string and returns an array of facets
/// extracted from the text, including mentions and URLs.
///
/// # Arguments
///
/// The operator accepts a single string argument containing the text to process.
///
/// # Returns
///
/// Returns an array of facet objects with byte positions and features.
///
/// # Example
///
/// ```json
/// {
///   "facet_text": ["Hello @alice.bsky.social! Check out https://example.com"]
/// }
/// ```
///
/// Returns:
/// ```json
/// [
///   {
///     "index": {"byteStart": 6, "byteEnd": 24},
///     "features": [{"$type": "app.bsky.richtext.facet#mention", "handle": "alice.bsky.social"}]
///   },
///   {
///     "index": {"byteStart": 36, "byteEnd": 55},
///     "features": [{"$type": "app.bsky.richtext.facet#link", "uri": "https://example.com"}]
///   }
/// ]
/// ```
///
/// Note: This operator parses mentions and URLs but does not resolve handles to DIDs.
/// For full DID resolution, use the facet_text node type instead.
#[derive(Debug)]
pub struct FacetTextOperator;

impl FacetTextOperator {
    /// Create a new facet text operator
    pub fn new() -> Self {
        Self
    }
}

impl Default for FacetTextOperator {
    fn default() -> Self {
        Self::new()
    }
}

/// Helper function to evaluate an argument that might contain nested expressions.
fn evaluate_arg(
    arg: &Value,
    context: &mut ContextStack,
    evaluator: &dyn Evaluator,
) -> Result<Value, DataLogicError> {
    // If it's an object, it might be a nested expression - try to evaluate it
    if arg.is_object() {
        match evaluator.evaluate(arg, context) {
            Ok(result) => Ok(result),
            Err(_) => Ok(arg.clone()),  // If evaluation fails, use as literal
        }
    } else {
        // For non-objects, return as-is
        Ok(arg.clone())
    }
}

impl Operator for FacetTextOperator {
    fn evaluate(
        &self,
        args: &[Value],
        context: &mut ContextStack,
        evaluator: &dyn Evaluator,
    ) -> Result<Value, DataLogicError> {
        // Require exactly one argument
        if args.len() != 1 {
            return Err(DataLogicError::InvalidArguments(
                "facet_text requires exactly one string argument".to_string(),
            ));
        }

        // Evaluate the argument (handles nested expressions)
        let evaluated_arg = evaluate_arg(&args[0], context, evaluator)?;

        // Extract the text string
        let text = match &evaluated_arg {
            Value::String(s) => s.as_str(),
            _ => {
                return Err(DataLogicError::InvalidArguments(
                    "facet_text argument must be a string".to_string(),
                ));
            }
        };

        // Parse mentions and URLs using the same logic as FacetTextEvaluator
        let mention_spans = parse_mentions_for_operator(text);
        let url_spans = parse_urls_for_operator(text);

        // Build the facets array
        let mut facets = Vec::new();

        // Add mention facets
        for mention in mention_spans {
            facets.push(json!({
                "index": {
                    "byteStart": mention.start,
                    "byteEnd": mention.end
                },
                "features": [{
                    "$type": "app.bsky.richtext.facet#mention",
                    "handle": mention.handle
                }]
            }));
        }

        // Add URL facets
        for url in url_spans {
            facets.push(json!({
                "index": {
                    "byteStart": url.start,
                    "byteEnd": url.end
                },
                "features": [{
                    "$type": "app.bsky.richtext.facet#link",
                    "uri": url.url
                }]
            }));
        }

        Ok(Value::Array(facets))
    }
}

// Helper functions for the operator (non-async versions)
fn parse_mentions_for_operator(text: &str) -> Vec<MentionSpan> {
    let mut spans = Vec::new();

    // Regex for @mentions - handles like @alice.bsky.social or @handle.com
    let mention_regex = Regex::new(
        r"(?:^|[^\w])(@[a-zA-Z0-9]([a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?(?:\.[a-zA-Z0-9]([a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?)*)"
    ).unwrap();

    let text_bytes = text.as_bytes();
    for capture in mention_regex.captures_iter(text_bytes) {
        if let Some(mention_match) = capture.get(1) {
            let handle = std::str::from_utf8(&mention_match.as_bytes()[1..])
                .unwrap_or_default()
                .to_string();

            spans.push(MentionSpan {
                start: mention_match.start(),
                end: mention_match.end(),
                handle,
            });
        }
    }

    spans
}

fn parse_urls_for_operator(text: &str) -> Vec<UrlSpan> {
    let mut spans = Vec::new();

    // URL regex
    let url_regex = Regex::new(
        r"(?:^|[^\w])(https?://(?:www\.)?[-a-zA-Z0-9@:%._\+~#=]{1,256}\.[a-zA-Z0-9()]{1,6}\b(?:[-a-zA-Z0-9()@:%_\+.~#?&//=]*[-a-zA-Z0-9@%_\+~#//=])?)"
    ).unwrap();

    let text_bytes = text.as_bytes();
    for capture in url_regex.captures_iter(text_bytes) {
        if let Some(url_match) = capture.get(1) {
            let url = std::str::from_utf8(url_match.as_bytes())
                .unwrap_or_default()
                .to_string();

            spans.push(UrlSpan {
                start: url_match.start(),
                end: url_match.end(),
                url,
            });
        }
    }

    spans
}
