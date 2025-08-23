//! Integration test for node configuration normalization

use ifthisthenat::storage::normalize_node_configuration;
use serde_json::json;

#[test]
fn test_jetstream_entry_normalization_integration() {
    // Create a jetstream_entry configuration with unsorted, duplicate values
    let config = json!({
        "did": [
            "did:plc:zzz999999999999999999999",
            "did:plc:aaa111111111111111111111",
            "did:plc:zzz999999999999999999999",  // duplicate
            "did:plc:bbb222222222222222222222"
        ],
        "collection": [
            "app.bsky.feed.like",
            "app.bsky.feed.post",
            "app.bsky.feed.like",  // duplicate
            "app.bsky.actor.profile"
        ]
    });

    // Normalize the configuration
    let normalized = normalize_node_configuration("jetstream_entry", &config).unwrap();

    // Extract the arrays from the normalized configuration
    let did_array = normalized
        .get("did")
        .and_then(|v| v.as_array())
        .expect("did field should be an array");
    let collection_array = normalized
        .get("collection")
        .and_then(|v| v.as_array())
        .expect("collection field should be an array");

    // Verify DIDs are sorted and deduplicated
    assert_eq!(did_array.len(), 3); // 4 original, 1 duplicate removed
    assert_eq!(
        did_array[0].as_str(),
        Some("did:plc:aaa111111111111111111111")
    );
    assert_eq!(
        did_array[1].as_str(),
        Some("did:plc:bbb222222222222222222222")
    );
    assert_eq!(
        did_array[2].as_str(),
        Some("did:plc:zzz999999999999999999999")
    );

    // Verify collections are sorted and deduplicated
    assert_eq!(collection_array.len(), 3); // 4 original, 1 duplicate removed
    assert_eq!(collection_array[0].as_str(), Some("app.bsky.actor.profile"));
    assert_eq!(collection_array[1].as_str(), Some("app.bsky.feed.like"));
    assert_eq!(collection_array[2].as_str(), Some("app.bsky.feed.post"));
}

#[test]
fn test_identical_configs_produce_identical_normalized_output() {
    // Two configurations with different ordering but same content
    let config1 = json!({
        "collection": ["app.bsky.feed.post", "app.bsky.feed.like"],
        "did": ["did:plc:bbb222222222222222222222", "did:plc:aaa111111111111111111111"]
    });

    let config2 = json!({
        "did": ["did:plc:aaa111111111111111111111", "did:plc:bbb222222222222222222222"],
        "collection": ["app.bsky.feed.like", "app.bsky.feed.post"]
    });

    let normalized1 = normalize_node_configuration("jetstream_entry", &config1).unwrap();
    let normalized2 = normalize_node_configuration("jetstream_entry", &config2).unwrap();

    // Convert to strings for comparison (canonicalized JSON should be identical)
    let str1 = serde_json::to_string(&normalized1).unwrap();
    let str2 = serde_json::to_string(&normalized2).unwrap();

    assert_eq!(
        str1, str2,
        "Identical configurations should produce identical normalized output"
    );
}
