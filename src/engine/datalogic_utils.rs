//! DataLogic utilities and custom functions for blueprint evaluation.
//!
//! This module provides custom DataLogic operators and utility functions
//! that extend the standard DataLogic functionality with application-specific
//! operations used throughout the blueprint evaluation system.

use atproto_record::aturi::ATURI;
use datalogic_rs::arena::DataArena;
use datalogic_rs::value::NumberValue;
use datalogic_rs::{CustomOperator, DataLogic, DataValue, LogicError, Result};
use metrohash::MetroHash64;
use std::fmt::Debug;
use std::hash::Hasher;
use std::str::FromStr;

/// Custom DataLogic operator that concatenates multiple strings and applies MetroHash64.
///
/// The `metrohash` operator takes one or more string arguments, concatenates them,
/// applies the MetroHash64 algorithm, and returns the hash as an integer.
///
/// # Arguments
///
/// The operator accepts one or more arguments. Each argument is converted to a string:
/// - String values are used directly
/// - Numbers are converted to their string representation
/// - Booleans become "true" or "false"
/// - Null becomes "null"
/// - Arrays and objects are serialized to JSON strings
///
/// # Returns
///
/// Returns a positive integer (i64) representing the MetroHash64 of the concatenated input.
///
/// # Example
///
/// ```json
/// {
///   "metrohash": ["user", ":", "12345"]
/// }
/// ```
///
/// This would concatenate "user:12345" and return its MetroHash64 value.
///
/// # Use Cases
///
/// - Creating deterministic IDs from multiple fields
/// - Generating cache keys from compound values
/// - Creating hash-based partitioning keys
/// - Deduplication based on content hashing
#[derive(Debug)]
pub struct MetroHashOperator;

impl CustomOperator for MetroHashOperator {
    fn evaluate<'a>(
        &self,
        args: &'a [DataValue<'a>],
        arena: &'a DataArena,
    ) -> Result<&'a DataValue<'a>> {
        // Require at least one argument
        if args.is_empty() {
            return Err(LogicError::custom(
                "metrohash requires at least one argument",
            ));
        }

        // Concatenate all arguments as strings
        let mut concatenated = String::new();
        for arg in args {
            match arg {
                DataValue::String(s) => concatenated.push_str(s),
                DataValue::Number(n) => concatenated.push_str(&n.to_string()),
                DataValue::Bool(b) => concatenated.push_str(if *b { "true" } else { "false" }),
                DataValue::Null => concatenated.push_str("null"),
                DataValue::Array(arr) => {
                    // Convert array to string representation
                    for item in arr.iter() {
                        // Recursively convert array items to strings
                        match item {
                            DataValue::String(s) => {
                                concatenated.push_str(s);
                            }
                            DataValue::Number(n) => concatenated.push_str(&n.to_string()),
                            DataValue::Bool(b) => {
                                concatenated.push_str(if *b { "true" } else { "false" })
                            }
                            DataValue::Null => concatenated.push_str("null"),
                            _ => concatenated.push_str(&format!("{:?}", item)),
                        }
                    }
                }
                DataValue::Object(obj) => {
                    // Convert object to string representation
                    for (key, value) in obj.iter() {
                        concatenated.push_str(key);
                        // Recursively convert object values to strings
                        match value {
                            DataValue::String(s) => {
                                concatenated.push_str(s);
                            }
                            DataValue::Number(n) => concatenated.push_str(&n.to_string()),
                            DataValue::Bool(b) => {
                                concatenated.push_str(if *b { "true" } else { "false" })
                            }
                            DataValue::Null => concatenated.push_str("null"),
                            _ => concatenated.push_str(&format!("{:?}", value)),
                        }
                    }
                }
                DataValue::DateTime(dt) => {
                    // Convert datetime to ISO string
                    concatenated.push_str(&dt.to_rfc3339());
                }
                DataValue::Duration(dur) => {
                    // Convert duration to string representation
                    concatenated.push_str(&format!("{:?}", dur));
                }
            }
        }

        // Apply MetroHash64
        let mut hasher = MetroHash64::new();
        hasher.write(concatenated.as_bytes());
        let hash_value = hasher.finish() as i64;

        // Ensure the value is positive by taking absolute value
        // MetroHash64 returns u64, but we need i64 for DataValue
        let positive_hash = if hash_value < 0 {
            hash_value.wrapping_abs()
        } else {
            hash_value
        };

        // Allocate the result in the arena
        Ok(arena.alloc(DataValue::Number(NumberValue::from_i64(positive_hash))))
    }
}

/// Custom DataLogic operator that parses AT-URIs into their component parts.
///
/// The `parse_aturi` operator takes a single AT-URI string argument and parses it
/// into an object containing the repository, collection, and record_key fields
/// using the AT Protocol's official ATURI parser.
///
/// # Arguments
///
/// The operator accepts exactly one string argument containing an AT-URI
/// in the format: `at://[repository]/[collection]/[record_key]`
///
/// # Returns
///
/// Returns an object with three fields:
/// - `repository`: The repository identifier (typically a DID)
/// - `collection`: The collection/lexicon identifier
/// - `record_key`: The record key/identifier
///
/// # Errors
///
/// Returns an error if:
/// - No arguments or more than one argument is provided
/// - The argument is not a string
/// - The AT-URI format is invalid
/// - The AT-URI is missing required components
///
/// # Example
///
/// ```json
/// {
///   "parse_aturi": ["at://did:plc:cbkjy5n7bk3ax2wplmtjofq2/app.bsky.feed.post/xyz789"]
/// }
/// ```
///
/// Returns:
/// ```json
/// {
///   "repository": "did:plc:cbkjy5n7bk3ax2wplmtjofq2",
///   "collection": "app.bsky.feed.post",
///   "record_key": "xyz789"
/// }
/// ```
///
/// # Use Cases
///
/// - Extracting components from AT-URIs for routing
/// - Validating AT-URI format in blueprints
/// - Filtering based on collection or repository
/// - Building queries based on parsed URI components
#[derive(Debug)]
pub struct ParseAturiOperator;

impl CustomOperator for ParseAturiOperator {
    fn evaluate<'a>(
        &self,
        args: &'a [DataValue<'a>],
        arena: &'a DataArena,
    ) -> Result<&'a DataValue<'a>> {
        // Require exactly one argument
        if args.len() != 1 {
            return Err(LogicError::custom(
                "parse_aturi requires exactly one string argument",
            ));
        }

        // Extract the AT-URI string
        let uri_str = match &args[0] {
            DataValue::String(s) => s,
            _ => {
                return Err(LogicError::custom("parse_aturi argument must be a string"));
            }
        };

        // Parse the AT-URI using the atproto_record library
        let parsed = ATURI::from_str(uri_str)
            .map_err(|e| LogicError::custom(&format!("Invalid AT-URI format: {}", e)))?;

        // Extract components
        let repository = parsed.authority;
        let collection = parsed.collection;
        let record_key = parsed.record_key;

        // Validate that collection is not empty
        if collection.is_empty() {
            return Err(LogicError::custom(
                "Invalid AT-URI: missing collection component",
            ));
        }

        // Create the result object in the arena
        let repository_str = arena.alloc_str(&repository);
        let collection_str = arena.alloc_str(&collection);
        let record_key_str = arena.alloc_str(&record_key);

        let object = arena.alloc(DataValue::Object(arena.alloc_slice_fill_with(
            3,
            |i| match i {
                0 => ("repository", DataValue::String(repository_str)),
                1 => ("collection", DataValue::String(collection_str)),
                _ => ("record_key", DataValue::String(record_key_str)),
            },
        )));

        Ok(object)
    }
}

/// Creates a DataLogic instance with all custom operators registered.
///
/// This function creates a standard DataLogic instance and registers
/// all custom operators provided by this module. It should be used
/// instead of the basic `create_datalogic()` when custom operators
/// are needed.
///
/// # Custom Operators
///
/// The following custom operators are registered:
/// - `metrohash`: Concatenates strings and applies MetroHash64
/// - `facet_text`: Parses text for mentions and URLs to create AT Protocol facets
/// - `parse_aturi`: Parses AT-URIs into repository, collection, and record_key components
///
/// # Configuration
///
/// The DataLogic instance is configured with:
/// - Chunk size: 1MB (1024 * 1024 bytes)
/// - Preserve structure: Enabled to maintain JSON structure integrity
///
/// # Example
///
/// ```rust,ignore
/// use ifthisthenat::engine::datalogic_utils::create_datalogic_with_custom_ops;
/// use serde_json::json;
///
/// let datalogic = create_datalogic_with_custom_ops();
///
/// // Use the metrohash operator
/// let expression = json!({
///     "metrohash": ["user", ":", {"val": ["user_id"]}]
/// });
/// let data = json!({"user_id": "12345"});
///
/// match datalogic.evaluate_json(&expression, &data, None) {
///     Ok(result) => println!("Hash: {}", result),
///     Err(e) => println!("Error: {}", e),
/// }
/// ```
pub fn create_datalogic_with_custom_ops() -> DataLogic {
    let mut datalogic = DataLogic::with_chunk_size_and_preserve_structure(1024 * 1024 * 3);

    // Register the metrohash operator
    datalogic.register_custom_operator("metrohash", Box::new(MetroHashOperator));

    // Register the facet_text operator
    datalogic.register_custom_operator(
        "facet_text",
        Box::new(crate::engine::node_type_facet_text::FacetTextOperator::new()),
    );

    // Register the parse_aturi operator
    datalogic.register_custom_operator("parse_aturi", Box::new(ParseAturiOperator));

    datalogic
}

/// Convenience function to hash a single string using MetroHash64.
///
/// This is a standalone utility function that doesn't require DataLogic
/// and can be used directly in Rust code.
///
/// # Arguments
///
/// * `input` - The string to hash
///
/// # Returns
///
/// Returns a positive i64 hash value
///
/// # Example
///
/// ```rust,ignore
/// use ifthisthenat::engine::datalogic_utils::metrohash_string;
///
/// let hash = metrohash_string("user:12345");
/// println!("Hash: {}", hash);
/// ```
pub fn metrohash_string(input: &str) -> i64 {
    let mut hasher = MetroHash64::new();
    hasher.write(input.as_bytes());
    let hash_value = hasher.finish() as i64;

    // Ensure positive value
    if hash_value < 0 {
        hash_value.wrapping_abs()
    } else {
        hash_value
    }
}

/// Convenience function to hash multiple strings using MetroHash64.
///
/// This concatenates the strings and returns their hash.
///
/// # Arguments
///
/// * `inputs` - Slice of strings to concatenate and hash
///
/// # Returns
///
/// Returns a positive i64 hash value
///
/// # Example
///
/// ```rust,ignore
/// use ifthisthenat::engine::datalogic_utils::metrohash_strings;
///
/// let hash = metrohash_strings(&["user", ":", "12345"]);
/// println!("Hash: {}", hash);
/// ```
pub fn metrohash_strings(inputs: &[&str]) -> i64 {
    let concatenated = inputs.join("");
    metrohash_string(&concatenated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_metrohash_string() {
        // Test single string hashing
        let hash1 = metrohash_string("test");
        let hash2 = metrohash_string("test");
        assert_eq!(hash1, hash2, "Same input should produce same hash");

        let hash3 = metrohash_string("different");
        assert_ne!(
            hash1, hash3,
            "Different inputs should produce different hashes"
        );

        // Test that hash is positive
        assert!(hash1 > 0, "Hash should be positive");
    }

    #[test]
    fn test_metrohash_strings() {
        // Test multiple string concatenation
        let hash1 = metrohash_strings(&["user", ":", "12345"]);
        let hash2 = metrohash_strings(&["user", ":", "12345"]);
        assert_eq!(hash1, hash2, "Same inputs should produce same hash");

        // Test that concatenation order matters
        let hash3 = metrohash_strings(&["12345", ":", "user"]);
        assert_ne!(
            hash1, hash3,
            "Different order should produce different hash"
        );

        // Test single vs concatenated
        let hash4 = metrohash_string("user:12345");
        assert_eq!(
            hash1, hash4,
            "Concatenated strings should match single string"
        );
    }

    #[test]
    fn test_metrohash_operator_single_string() {
        let datalogic = create_datalogic_with_custom_ops();

        let expression = json!({
            "metrohash": ["test_string"]
        });
        let data = json!({});

        let result = datalogic.evaluate_json(&expression, &data, None).unwrap();
        assert!(result.is_number(), "Result should be a number");

        let hash = result.as_i64().unwrap();
        assert!(hash > 0, "Hash should be positive");

        // Verify it matches the utility function
        let expected = metrohash_string("test_string");
        assert_eq!(hash, expected, "Should match utility function result");
    }

    #[test]
    fn test_metrohash_operator_multiple_strings() {
        let datalogic = create_datalogic_with_custom_ops();

        let expression = json!({
            "metrohash": ["user", ":", "12345"]
        });
        let data = json!({});

        let result = datalogic.evaluate_json(&expression, &data, None).unwrap();
        assert!(result.is_number(), "Result should be a number");

        let hash = result.as_i64().unwrap();
        assert!(hash > 0, "Hash should be positive");

        // Verify it matches the utility function
        let expected = metrohash_strings(&["user", ":", "12345"]);
        assert_eq!(hash, expected, "Should match utility function result");
    }

    #[test]
    fn test_metrohash_operator_with_datalogic_values() {
        let datalogic = create_datalogic_with_custom_ops();

        // Test with value extraction
        let expression = json!({
            "metrohash": [
                "prefix:",
                {"val": ["user_id"]},
                ":",
                {"val": ["timestamp"]}
            ]
        });
        let data = json!({
            "user_id": "abc123",
            "timestamp": 1234567890
        });

        let result = datalogic.evaluate_json(&expression, &data, None).unwrap();
        assert!(result.is_number(), "Result should be a number");

        let hash = result.as_i64().unwrap();
        assert!(hash > 0, "Hash should be positive");

        // Verify deterministic behavior
        let result2 = datalogic.evaluate_json(&expression, &data, None).unwrap();
        assert_eq!(
            result, result2,
            "Same expression and data should produce same hash"
        );
    }

    #[test]
    fn test_metrohash_operator_with_different_types() {
        let datalogic = create_datalogic_with_custom_ops();

        // Test with mixed types
        let expression = json!({
            "metrohash": [
                "str",
                123,
                true,
                null,
                {"val": ["arr"]},
                {"val": ["obj"]}
            ]
        });
        let data = json!({
            "arr": [1, 2, 3],
            "obj": {"key": "value"}
        });

        let result = datalogic.evaluate_json(&expression, &data, None).unwrap();
        assert!(result.is_number(), "Result should be a number");

        let hash = result.as_i64().unwrap();
        assert!(hash > 0, "Hash should be positive");
    }

    #[test]
    fn test_metrohash_operator_empty_args() {
        let datalogic = create_datalogic_with_custom_ops();

        let expression = json!({
            "metrohash": []
        });
        let data = json!({});

        let result = datalogic.evaluate_json(&expression, &data, None);
        assert!(result.is_err(), "Empty arguments should produce an error");
    }

    #[test]
    fn test_metrohash_in_conditional() {
        let datalogic = create_datalogic_with_custom_ops();

        // Use metrohash in a more complex expression
        let expression = json!({
            "==": [
                {"metrohash": ["user", ":", {"val": ["id"]}]},
                {"metrohash": ["user", ":", "123"]}
            ]
        });
        let data = json!({"id": "123"});

        let result = datalogic.evaluate_json(&expression, &data, None).unwrap();
        assert!(result.is_boolean(), "Result should be a boolean");
        assert_eq!(result.as_bool(), Some(true), "Hashes should match");

        // Test with different ID
        let data2 = json!({"id": "456"});
        let result2 = datalogic.evaluate_json(&expression, &data2, None).unwrap();
        assert_eq!(result2.as_bool(), Some(false), "Hashes should not match");
    }

    #[test]
    fn test_consistency_across_invocations() {
        // Test that the same input always produces the same hash
        let datalogic1 = create_datalogic_with_custom_ops();
        let datalogic2 = create_datalogic_with_custom_ops();

        let expression = json!({
            "metrohash": ["consistent", "test", "data"]
        });
        let data = json!({});

        let result1 = datalogic1.evaluate_json(&expression, &data, None).unwrap();
        let result2 = datalogic2.evaluate_json(&expression, &data, None).unwrap();

        assert_eq!(
            result1, result2,
            "Same expression should produce same hash across different DataLogic instances"
        );
    }

    #[test]
    fn test_facet_text_operator_basic() {
        let datalogic = create_datalogic_with_custom_ops();

        let expression = json!({
            "facet_text": ["Hello @alice.bsky.social! Check out https://example.com"]
        });
        let data = json!({});

        let result = datalogic.evaluate_json(&expression, &data, None).unwrap();
        assert!(result.is_array(), "Result should be an array");

        // The result is now directly the facets array
        let facets = result.as_array().unwrap();
        assert_eq!(facets.len(), 2, "Should have 2 facets (1 mention + 1 URL)");

        // Check mention facet
        let mention_facet = &facets[0];
        assert_eq!(mention_facet["index"]["byteStart"], 6);
        assert_eq!(mention_facet["index"]["byteEnd"], 24);
        assert_eq!(
            mention_facet["features"][0]["$type"],
            "app.bsky.richtext.facet#mention"
        );
        assert_eq!(mention_facet["features"][0]["handle"], "alice.bsky.social");

        // Check URL facet
        let url_facet = &facets[1];
        assert_eq!(url_facet["index"]["byteStart"], 36);
        assert_eq!(url_facet["index"]["byteEnd"], 55);
        assert_eq!(
            url_facet["features"][0]["$type"],
            "app.bsky.richtext.facet#link"
        );
        assert_eq!(url_facet["features"][0]["uri"], "https://example.com");
    }

    #[test]
    fn test_facet_text_operator_with_data_extraction() {
        let datalogic = create_datalogic_with_custom_ops();

        // Use facet_text with data extracted from input
        let expression = json!({
            "facet_text": [{"val": ["message"]}]
        });
        let data = json!({
            "message": "Visit @bob.test.com and https://test.org"
        });

        let result = datalogic.evaluate_json(&expression, &data, None).unwrap();
        assert!(result.is_array(), "Result should be an array");

        let facets = result.as_array().unwrap();
        assert_eq!(facets.len(), 2, "Should have 2 facets");
    }

    #[test]
    fn test_facet_text_operator_empty_text() {
        let datalogic = create_datalogic_with_custom_ops();

        let expression = json!({
            "facet_text": [""]
        });
        let data = json!({});

        let result = datalogic.evaluate_json(&expression, &data, None).unwrap();
        assert!(result.is_array(), "Result should be an array");

        let facets = result.as_array().unwrap();
        assert_eq!(facets.len(), 0, "Should have no facets for empty text");
    }

    #[test]
    fn test_facet_text_operator_no_mentions_or_urls() {
        let datalogic = create_datalogic_with_custom_ops();

        let expression = json!({
            "facet_text": ["Just plain text without any mentions or URLs"]
        });
        let data = json!({});

        let result = datalogic.evaluate_json(&expression, &data, None).unwrap();
        assert!(result.is_array(), "Result should be an array");

        let facets = result.as_array().unwrap();
        assert_eq!(facets.len(), 0, "Should have no facets");
    }

    #[test]
    fn test_facet_text_operator_multiple_mentions() {
        let datalogic = create_datalogic_with_custom_ops();

        let expression = json!({
            "facet_text": ["@alice.bsky.social and @bob.test.com are here"]
        });
        let data = json!({});

        let result = datalogic.evaluate_json(&expression, &data, None).unwrap();
        assert!(result.is_array(), "Result should be an array");

        let facets = result.as_array().unwrap();
        assert_eq!(facets.len(), 2, "Should have 2 mention facets");

        // Both should be mentions
        for facet in facets {
            assert_eq!(
                facet["features"][0]["$type"],
                "app.bsky.richtext.facet#mention"
            );
        }
    }

    #[test]
    fn test_facet_text_operator_invalid_args() {
        let datalogic = create_datalogic_with_custom_ops();

        // Test with no arguments
        let expression = json!({
            "facet_text": []
        });
        let data = json!({});
        let result = datalogic.evaluate_json(&expression, &data, None);
        assert!(result.is_err(), "Should error with no arguments");

        // Test with multiple arguments
        let expression = json!({
            "facet_text": ["text1", "text2"]
        });
        let result = datalogic.evaluate_json(&expression, &data, None);
        assert!(result.is_err(), "Should error with multiple arguments");

        // Test with non-string argument
        let expression = json!({
            "facet_text": [123]
        });
        let result = datalogic.evaluate_json(&expression, &data, None);
        assert!(result.is_err(), "Should error with non-string argument");
    }

    #[test]
    fn test_facet_text_in_transform_chain() {
        let datalogic = create_datalogic_with_custom_ops();

        // First test that facet_text works and extract facets correctly
        let facet_expr = json!({
            "facet_text": ["Hello @alice.bsky.social!"]
        });
        let data = json!({});

        let facet_result = datalogic.evaluate_json(&facet_expr, &data, None).unwrap();
        assert!(facet_result.is_array(), "Facet result should be an array");

        let facets = facet_result.as_array().unwrap();
        assert_eq!(facets.len(), 1, "Should have 1 facet");

        // Test using facet_text with val extraction
        let complex_expr = json!({
            "facet_text": [{"val": ["input_text"]}]
        });
        let complex_data = json!({
            "input_text": "Hello @alice.bsky.social!"
        });

        let complex_result = datalogic
            .evaluate_json(&complex_expr, &complex_data, None)
            .unwrap();
        assert!(
            complex_result.is_array(),
            "Complex result should be an array"
        );

        let complex_facets = complex_result.as_array().unwrap();
        assert_eq!(complex_facets.len(), 1, "Should have 1 facet");
        assert_eq!(
            complex_facets[0]["features"][0]["$type"],
            "app.bsky.richtext.facet#mention"
        );
        assert_eq!(
            complex_facets[0]["features"][0]["handle"],
            "alice.bsky.social"
        );
    }

    #[test]
    fn test_parse_aturi_operator_basic() {
        let datalogic = create_datalogic_with_custom_ops();

        let expression = json!({
            "parse_aturi": ["at://did:plc:cbkjy5n7bk3ax2wplmtjofq2/app.bsky.feed.post/xyz789"]
        });
        let data = json!({});

        let result = datalogic.evaluate_json(&expression, &data, None).unwrap();
        assert!(result.is_object(), "Result should be an object");

        assert_eq!(result["repository"], "did:plc:cbkjy5n7bk3ax2wplmtjofq2");
        assert_eq!(result["collection"], "app.bsky.feed.post");
        assert_eq!(result["record_key"], "xyz789");
    }

    #[test]
    fn test_parse_aturi_operator_complex_did() {
        let datalogic = create_datalogic_with_custom_ops();

        let expression = json!({
            "parse_aturi": ["at://did:plc:lehcqqkwzcwvjvw66uthu5oq/community.lexicon.calendar.event/3lte3c7x43l2e"]
        });
        let data = json!({});

        let result = datalogic.evaluate_json(&expression, &data, None).unwrap();
        assert!(result.is_object(), "Result should be an object");

        assert_eq!(result["repository"], "did:plc:lehcqqkwzcwvjvw66uthu5oq");
        assert_eq!(result["collection"], "community.lexicon.calendar.event");
        assert_eq!(result["record_key"], "3lte3c7x43l2e");
    }

    #[test]
    fn test_parse_aturi_operator_with_web_did() {
        let datalogic = create_datalogic_with_custom_ops();

        let expression = json!({
            "parse_aturi": ["at://did:web:example.com/app.bsky.feed.like/abc123"]
        });
        let data = json!({});

        let result = datalogic.evaluate_json(&expression, &data, None).unwrap();
        assert!(result.is_object(), "Result should be an object");

        assert_eq!(result["repository"], "did:web:example.com");
        assert_eq!(result["collection"], "app.bsky.feed.like");
        assert_eq!(result["record_key"], "abc123");
    }

    #[test]
    fn test_parse_aturi_operator_with_data_extraction() {
        let datalogic = create_datalogic_with_custom_ops();

        // Use parse_aturi with data extracted from input
        let expression = json!({
            "parse_aturi": [{"val": ["uri"]}]
        });
        let data = json!({
            "uri": "at://did:plc:cbkjy5n7bk3ax2wplmtjofq2/app.bsky.feed.post/xyz789"
        });

        let result = datalogic.evaluate_json(&expression, &data, None).unwrap();
        assert!(result.is_object(), "Result should be an object");

        assert_eq!(result["repository"], "did:plc:cbkjy5n7bk3ax2wplmtjofq2");
        assert_eq!(result["collection"], "app.bsky.feed.post");
        assert_eq!(result["record_key"], "xyz789");
    }

    #[test]
    fn test_parse_aturi_operator_invalid_uri() {
        let datalogic = create_datalogic_with_custom_ops();

        let expression = json!({
            "parse_aturi": ["https://example.com/not/an/aturi"]
        });
        let data = json!({});

        let result = datalogic.evaluate_json(&expression, &data, None);
        assert!(result.is_err(), "Should error with invalid URI format");
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Invalid AT-URI format"),
            "Error should mention invalid AT-URI format"
        );
    }

    #[test]
    fn test_parse_aturi_operator_missing_components() {
        let datalogic = create_datalogic_with_custom_ops();

        // Test URI missing record key
        let expression = json!({
            "parse_aturi": ["at://did:plc:cbkjy5n7bk3ax2wplmtjofq2/app.bsky.feed.post"]
        });
        let data = json!({});

        let result = datalogic.evaluate_json(&expression, &data, None);
        assert!(result.is_err(), "Should error with missing record key");
    }

    #[test]
    fn test_parse_aturi_operator_invalid_args() {
        let datalogic = create_datalogic_with_custom_ops();

        // Test with no arguments
        let expression = json!({
            "parse_aturi": []
        });
        let data = json!({});
        let result = datalogic.evaluate_json(&expression, &data, None);
        assert!(result.is_err(), "Should error with no arguments");
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("exactly one string argument"),
            "Error should mention exactly one string argument"
        );

        // Test with multiple arguments
        let expression = json!({
            "parse_aturi": ["uri1", "uri2"]
        });
        let result = datalogic.evaluate_json(&expression, &data, None);
        assert!(result.is_err(), "Should error with multiple arguments");
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("exactly one string argument"),
            "Error should mention exactly one string argument"
        );

        // Test with non-string argument
        let expression = json!({
            "parse_aturi": [123]
        });
        let result = datalogic.evaluate_json(&expression, &data, None);
        assert!(result.is_err(), "Should error with non-string argument");
        assert!(
            result.unwrap_err().to_string().contains("must be a string"),
            "Error should mention string requirement"
        );
    }

    #[test]
    fn test_parse_aturi_in_complex_expression() {
        let datalogic = create_datalogic_with_custom_ops();

        // Use parse_aturi in a conditional expression to check collection
        let expression = json!({
            "==": [
                {"val": [{"parse_aturi": [{"val": ["uri"]}]}, "collection"]},
                "app.bsky.feed.post"
            ]
        });
        let data = json!({
            "uri": "at://did:plc:cbkjy5n7bk3ax2wplmtjofq2/app.bsky.feed.post/xyz789"
        });

        let result = datalogic.evaluate_json(&expression, &data, None).unwrap();
        assert!(result.is_boolean(), "Result should be a boolean");
        assert_eq!(result.as_bool(), Some(true), "Should match collection");

        // Test with different collection
        let data2 = json!({
            "uri": "at://did:plc:cbkjy5n7bk3ax2wplmtjofq2/app.bsky.feed.like/xyz789"
        });
        let result2 = datalogic.evaluate_json(&expression, &data2, None).unwrap();
        assert_eq!(
            result2.as_bool(),
            Some(false),
            "Should not match collection"
        );
    }

    #[test]
    fn test_parse_aturi_with_nested_data() {
        let datalogic = create_datalogic_with_custom_ops();

        // Test extracting URI from nested data structure
        let expression = json!({
            "parse_aturi": [{"val": ["commit", "record", "subject", "uri"]}]
        });
        let data = json!({
            "commit": {
                "record": {
                    "subject": {
                        "uri": "at://did:plc:lehcqqkwzcwvjvw66uthu5oq/community.lexicon.calendar.event/3lte3c7x43l2e"
                    }
                }
            }
        });

        let result = datalogic.evaluate_json(&expression, &data, None).unwrap();
        assert!(result.is_object(), "Result should be an object");

        assert_eq!(result["repository"], "did:plc:lehcqqkwzcwvjvw66uthu5oq");
        assert_eq!(result["collection"], "community.lexicon.calendar.event");
        assert_eq!(result["record_key"], "3lte3c7x43l2e");
    }

    #[test]
    fn test_parse_aturi_consistency() {
        let datalogic1 = create_datalogic_with_custom_ops();
        let datalogic2 = create_datalogic_with_custom_ops();

        let expression = json!({
            "parse_aturi": ["at://did:plc:cbkjy5n7bk3ax2wplmtjofq2/app.bsky.feed.post/xyz789"]
        });
        let data = json!({});

        let result1 = datalogic1.evaluate_json(&expression, &data, None).unwrap();
        let result2 = datalogic2.evaluate_json(&expression, &data, None).unwrap();

        assert_eq!(
            result1, result2,
            "Same URI should produce same parsed result across different DataLogic instances"
        );
    }

    #[test]
    fn test_parse_aturi_empty_collection() {
        let datalogic = create_datalogic_with_custom_ops();

        let expression = json!({
            "parse_aturi": ["at://did:plc:cbkjy5n7bk3ax2wplmtjofq2//xyz789"]
        });
        let data = json!({});

        let result = datalogic.evaluate_json(&expression, &data, None);
        assert!(result.is_err(), "Should error with empty collection");
        assert!(
            result.unwrap_err().to_string().contains("Invalid AT-URI"),
            "Error should mention invalid AT-URI"
        );
    }

    #[test]
    fn test_parse_aturi_transform_chain() {
        let datalogic = create_datalogic_with_custom_ops();

        // Test using parse_aturi to extract repository field
        let expression = json!({
            "val": [{"parse_aturi": [{"val": ["uri"]}]}, "repository"]
        });

        let data = json!({
            "uri": "at://did:plc:cbkjy5n7bk3ax2wplmtjofq2/app.bsky.feed.post/xyz789"
        });

        let result = datalogic.evaluate_json(&expression, &data, None).unwrap();
        assert!(result.is_string(), "Result should be a string");
        assert_eq!(result, "did:plc:cbkjy5n7bk3ax2wplmtjofq2");

        // Test extracting collection
        let collection_expr = json!({
            "val": [{"parse_aturi": [{"val": ["uri"]}]}, "collection"]
        });

        let result2 = datalogic
            .evaluate_json(&collection_expr, &data, None)
            .unwrap();
        assert_eq!(result2, "app.bsky.feed.post");

        // Test extracting record_key
        let rkey_expr = json!({
            "val": [{"parse_aturi": [{"val": ["uri"]}]}, "record_key"]
        });

        let result3 = datalogic.evaluate_json(&rkey_expr, &data, None).unwrap();
        assert_eq!(result3, "xyz789");
    }
}
