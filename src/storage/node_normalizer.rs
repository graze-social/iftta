//! Node configuration normalization utilities
//!
//! Ensures node configurations are stored in a canonical format for
//! consistent hashing and comparison.

use crate::constants::NODE_TYPE_JETSTREAM_ENTRY;
use anyhow::{Context, Result};
use serde_json::Value;

/// Normalize a node configuration to canonical form
///
/// This function:
/// 1. Applies node-type specific normalization (e.g., sorting arrays for jetstream_entry)
/// 2. Canonicalizes the JSON to ensure consistent formatting
pub fn normalize_node_configuration(node_type: &str, config: &Value) -> Result<Value> {
    // First apply node-type specific normalization
    let normalized = match node_type {
        NODE_TYPE_JETSTREAM_ENTRY => normalize_jetstream_entry_config(config)?,
        _ => config.clone(), // No special normalization for other node types
    };

    // Then canonicalize the JSON
    let canonical_str = serde_json_canonicalizer::to_vec(&normalized)
        .context("Failed to canonicalize node configuration")?;

    // Parse back to Value
    serde_json::from_slice(&canonical_str).context("Failed to parse canonicalized configuration")
}

/// Normalize jetstream_entry node configuration
///
/// Specifically:
/// - Sorts and deduplicates 'did' array if present
/// - Sorts and deduplicates 'collection' array if present
fn normalize_jetstream_entry_config(config: &Value) -> Result<Value> {
    let mut normalized = config.clone();

    // Normalize 'did' field if present
    if let Some(obj) = normalized.as_object_mut() {
        if let Some(did_value) = obj.get_mut("did")
            && let Some(did_array) = did_value.as_array_mut()
        {
            // Extract strings, deduplicate, and sort
            let mut dids: Vec<String> = did_array
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect();

            // Remove duplicates while preserving order
            dids.sort();
            dids.dedup();

            // Convert back to JSON array
            *did_value = Value::Array(dids.into_iter().map(Value::String).collect());
        }

        // Normalize 'collection' field if present
        if let Some(collection_value) = obj.get_mut("collection")
            && let Some(collection_array) = collection_value.as_array_mut()
        {
            // Extract strings, deduplicate, and sort
            let mut collections: Vec<String> = collection_array
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect();

            // Remove duplicates while preserving order
            collections.sort();
            collections.dedup();

            // Convert back to JSON array
            *collection_value = Value::Array(collections.into_iter().map(Value::String).collect());
        }
    }

    Ok(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_normalize_jetstream_entry_sorts_and_deduplicates() {
        let config = json!({
            "did": [
                "did:plc:zzz",
                "did:plc:aaa",
                "did:plc:zzz",  // duplicate
                "did:plc:bbb"
            ],
            "collection": [
                "app.bsky.feed.like",
                "app.bsky.feed.post",
                "app.bsky.feed.like",  // duplicate
                "app.bsky.actor.profile"
            ]
        });

        let normalized = normalize_jetstream_entry_config(&config).unwrap();

        let expected = json!({
            "did": [
                "did:plc:aaa",
                "did:plc:bbb",
                "did:plc:zzz"
            ],
            "collection": [
                "app.bsky.actor.profile",
                "app.bsky.feed.like",
                "app.bsky.feed.post"
            ]
        });

        assert_eq!(normalized, expected);
    }

    #[test]
    fn test_normalize_jetstream_entry_handles_missing_fields() {
        // Config with only 'did' field
        let config = json!({
            "did": ["did:plc:bbb", "did:plc:aaa"]
        });

        let normalized = normalize_jetstream_entry_config(&config).unwrap();
        assert_eq!(
            normalized,
            json!({
                "did": ["did:plc:aaa", "did:plc:bbb"]
            })
        );

        // Config with only 'collection' field
        let config = json!({
            "collection": ["app.bsky.feed.like", "app.bsky.feed.post"]
        });

        let normalized = normalize_jetstream_entry_config(&config).unwrap();
        assert_eq!(
            normalized,
            json!({
                "collection": ["app.bsky.feed.like", "app.bsky.feed.post"]
            })
        );

        // Config with neither field
        let config = json!({
            "other_field": "value"
        });

        let normalized = normalize_jetstream_entry_config(&config).unwrap();
        assert_eq!(normalized, config);
    }

    #[test]
    fn test_canonicalization_produces_consistent_output() {
        let config1 = json!({
            "collection": ["app.bsky.feed.post"],
            "did": ["did:plc:abc"]
        });

        let config2 = json!({
            "did": ["did:plc:abc"],
            "collection": ["app.bsky.feed.post"]
        });

        let normalized1 =
            normalize_node_configuration(NODE_TYPE_JETSTREAM_ENTRY, &config1).unwrap();
        let normalized2 =
            normalize_node_configuration(NODE_TYPE_JETSTREAM_ENTRY, &config2).unwrap();

        // After canonicalization, both should be identical
        assert_eq!(
            serde_json::to_string(&normalized1).unwrap(),
            serde_json::to_string(&normalized2).unwrap()
        );
    }

    #[test]
    fn test_other_node_types_pass_through() {
        let config = json!({
            "url": "https://example.com",
            "timeout": 5000
        });

        let normalized = normalize_node_configuration("webhook_entry", &config).unwrap();

        // Should be canonicalized but not otherwise modified
        assert_eq!(
            normalized.get("url").and_then(|v| v.as_str()),
            Some("https://example.com")
        );
        assert_eq!(
            normalized.get("timeout").and_then(|v| v.as_u64()),
            Some(5000)
        );
    }
}
