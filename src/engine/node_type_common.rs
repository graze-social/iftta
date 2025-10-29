//! Common utilities for node type implementations.
//!
//! This module provides shared functionality used across multiple node types
//! to reduce code duplication and ensure consistent behavior.

use anyhow::Result;
use serde_json::Value;
use std::collections::HashMap;

use super::common::with_cached_datalogic;
use crate::errors::EngineError;
use crate::storage::node::Node;

/// Extract payload data from node configuration using common pattern.
///
/// This function implements the standard payload extraction pattern used by
/// multiple node types (publish_record, publish_webhook, etc.).
///
/// The payload can be:
/// 1. A string - used as field name to extract from input
/// 2. An object - evaluated with DataLogic to produce the data
/// 3. Other types - returns an error
///
/// # Examples
///
/// String payload:
/// ```json
/// // Input: {"data": {"message": "hello"}}
/// // Payload: "data"
/// // Returns: {"message": "hello"}
/// ```
///
/// Object payload:
/// ```json
/// // Input: {"user": {"name": "Alice"}}
/// // Payload: {"val": ["user"]}
/// // Returns: {"name": "Alice"}
/// ```
pub fn extract_payload_data(node: &Node, input: &Value) -> Result<Value> {
    if let Some(field_name) = node.payload.as_str() {
        // String payload: use as field name to extract from input
        input
            .get(field_name)
            .ok_or_else(|| {
                EngineError::MissingRequiredField {
                    field_name: field_name.to_string(),
                    node_type: "common".to_string(),
                }
                .into()
            })
            .map(|v| v.clone())
    } else if node.payload.is_object() {
        // Object payload: evaluate with DataLogic
        with_cached_datalogic(|datalogic| {
            datalogic
                .evaluate_json(&node.payload, input, None)
                .map_err(|e| {
                    EngineError::DataLogicFailed {
                        expression: format!("{:?}", node.payload),
                        details: e.to_string(),
                    }
                    .into()
                })
        })
    } else {
        Err(EngineError::InvalidFieldType {
            field_name: "payload".to_string(),
            node_type: "common".to_string(),
            expected_type: "string or object".to_string(),
        }
        .into())
    }
}

/// Check if an HTTP status code is considered successful for webhooks.
///
/// Only HTTP 200 (OK) and 204 (No Content) are considered successful
/// for webhook deliveries. All other status codes, including other 2xx
/// codes and redirects, are considered failures.
pub fn is_webhook_success_status(status_code: u16) -> bool {
    status_code == 200 || status_code == 204
}

/// Extract and validate headers from node configuration.
///
/// Headers must be provided as an object with string key-value pairs.
/// Returns a HashMap suitable for use with HTTP requests.
///
/// # Examples
///
/// Valid headers:
/// ```json
/// {
///   "Authorization": "Bearer token",
///   "X-Custom-Header": "value"
/// }
/// ```
pub fn extract_headers_from_config(config: &Value) -> Result<HashMap<String, String>> {
    let mut headers = HashMap::new();

    // Always set default Content-Type for JSON webhooks
    headers.insert("Content-Type".to_string(), "application/json".to_string());

    if let Some(headers_value) = config.get("headers") {
        if !headers_value.is_object() {
            return Err(EngineError::InvalidFieldType {
                field_name: "headers".to_string(),
                node_type: "common".to_string(),
                expected_type: "object".to_string(),
            }
            .into());
        }

        if let Some(headers_obj) = headers_value.as_object() {
            for (key, value) in headers_obj {
                if let Some(header_value) = value.as_str() {
                    headers.insert(key.clone(), header_value.to_string());
                } else {
                    return Err(EngineError::InvalidFieldType {
                        field_name: format!("headers.{}", key),
                        node_type: "common".to_string(),
                        expected_type: "string".to_string(),
                    }
                    .into());
                }
            }
        }
    }

    Ok(headers)
}

/// Clone input and add a results field.
///
/// This is a common pattern for action nodes that need to return
/// the original input with additional results from their operation.
pub fn clone_input_with_results(input: &Value, field_name: &str, results: Value) -> Value {
    let mut output = input.clone();
    if let Some(obj) = output.as_object_mut() {
        obj.insert(field_name.to_string(), results);
    }
    output
}

/// Validate that a URL uses HTTPS protocol.
///
/// This is required for webhook URLs to ensure secure communication.
pub fn validate_https_url(url: &str) -> Result<()> {
    if !url.starts_with("https://") {
        Err(EngineError::InvalidNodeConfiguration {
            node_type: "webhook".to_string(),
            details: format!("URL must use HTTPS protocol, got: {}", url),
        }
        .into())
    } else {
        Ok(())
    }
}

/// Extract an optional string field from configuration.
///
/// Returns None if the field doesn't exist or is null.
/// Returns an error if the field exists but is not a string.
pub fn get_optional_config_string(config: &Value, field_name: &str) -> Result<Option<String>> {
    match config.get(field_name) {
        None => Ok(None),
        Some(Value::Null) => Ok(None),
        Some(v) => v.as_str().map(|s| Some(s.to_string())).ok_or_else(|| {
            EngineError::InvalidFieldType {
                field_name: field_name.to_string(),
                node_type: "configuration".to_string(),
                expected_type: "string".to_string(),
            }
            .into()
        }),
    }
}

/// Extract a required string field from configuration.
///
/// Returns an error if the field doesn't exist or is not a string.
pub fn get_required_config_string(config: &Value, field_name: &str) -> Result<String> {
    config
        .get(field_name)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| {
            EngineError::MissingRequiredField {
                field_name: field_name.to_string(),
                node_type: "configuration".to_string(),
            }
            .into()
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use serde_json::json;

    // ============== extract_payload_data tests ==============

    #[test]
    fn test_extract_payload_data_string_field_exists() {
        let node = Node {
            aturi: "test".to_string(),
            blueprint: "test".to_string(),
            node_type: "test".to_string(),
            payload: json!("data"),
            configuration: json!({}),
            created_at: Utc::now(),
        };

        let input = json!({
            "data": {"message": "hello"},
            "other": "field"
        });

        let result = extract_payload_data(&node, &input).unwrap();
        assert_eq!(result, json!({"message": "hello"}));
    }

    #[test]
    fn test_extract_payload_data_string_field_missing() {
        let node = Node {
            aturi: "test".to_string(),
            blueprint: "test".to_string(),
            node_type: "test".to_string(),
            payload: json!("missing_field"),
            configuration: json!({}),
            created_at: Utc::now(),
        };

        let input = json!({
            "data": {"message": "hello"},
            "other": "field"
        });

        let result = extract_payload_data(&node, &input);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("error-iftta-engine-6")
        );
    }

    #[test]
    fn test_extract_payload_data_string_field_null() {
        let node = Node {
            aturi: "test".to_string(),
            blueprint: "test".to_string(),
            node_type: "test".to_string(),
            payload: json!("nullable"),
            configuration: json!({}),
            created_at: Utc::now(),
        };

        let input = json!({
            "nullable": null,
            "other": "field"
        });

        let result = extract_payload_data(&node, &input).unwrap();
        assert_eq!(result, json!(null));
    }

    #[test]
    fn test_extract_payload_data_string_field_primitive_types() {
        let node = Node {
            aturi: "test".to_string(),
            blueprint: "test".to_string(),
            node_type: "test".to_string(),
            payload: json!("value"),
            configuration: json!({}),
            created_at: Utc::now(),
        };

        // Test with string value
        let input = json!({"value": "text"});
        let result = extract_payload_data(&node, &input).unwrap();
        assert_eq!(result, json!("text"));

        // Test with number value
        let input = json!({"value": 42});
        let result = extract_payload_data(&node, &input).unwrap();
        assert_eq!(result, json!(42));

        // Test with boolean value
        let input = json!({"value": true});
        let result = extract_payload_data(&node, &input).unwrap();
        assert_eq!(result, json!(true));

        // Test with array value
        let input = json!({"value": [1, 2, 3]});
        let result = extract_payload_data(&node, &input).unwrap();
        assert_eq!(result, json!([1, 2, 3]));
    }

    #[test]
    fn test_extract_payload_data_string_field_nested_object() {
        let node = Node {
            aturi: "test".to_string(),
            blueprint: "test".to_string(),
            node_type: "test".to_string(),
            payload: json!("deeply"),
            configuration: json!({}),
            created_at: Utc::now(),
        };

        let input = json!({
            "deeply": {
                "nested": {
                    "object": {
                        "value": "found"
                    }
                }
            }
        });

        let result = extract_payload_data(&node, &input).unwrap();
        assert_eq!(
            result,
            json!({
                "nested": {
                    "object": {
                        "value": "found"
                    }
                }
            })
        );
    }

    #[test]
    fn test_extract_payload_data_object_with_datalogic_val() {
        let node = Node {
            aturi: "test".to_string(),
            blueprint: "test".to_string(),
            node_type: "test".to_string(),
            payload: json!({"val": ["user", "profile"]}),
            configuration: json!({}),
            created_at: Utc::now(),
        };

        let input = json!({
            "user": {
                "profile": {"name": "Alice", "age": 30}
            }
        });

        let result = extract_payload_data(&node, &input).unwrap();
        assert_eq!(result, json!({"name": "Alice", "age": 30}));
    }

    #[test]
    fn test_extract_payload_data_object_with_datalogic_nested_path() {
        let node = Node {
            aturi: "test".to_string(),
            blueprint: "test".to_string(),
            node_type: "test".to_string(),
            payload: json!({"val": ["a", "b", "c", "d"]}),
            configuration: json!({}),
            created_at: Utc::now(),
        };

        let input = json!({
            "a": {
                "b": {
                    "c": {
                        "d": "deep_value"
                    }
                }
            }
        });

        let result = extract_payload_data(&node, &input).unwrap();
        assert_eq!(result, json!("deep_value"));
    }

    #[test]
    fn test_extract_payload_data_object_with_datalogic_missing_path() {
        let node = Node {
            aturi: "test".to_string(),
            blueprint: "test".to_string(),
            node_type: "test".to_string(),
            payload: json!({"val": ["non", "existent", "path"]}),
            configuration: json!({}),
            created_at: Utc::now(),
        };

        let input = json!({
            "user": {
                "profile": {"name": "Alice"}
            }
        });

        // DataLogic returns null for missing paths rather than erroring
        let result = extract_payload_data(&node, &input).unwrap();
        assert_eq!(result, json!(null));
    }

    #[test]
    fn test_extract_payload_data_object_with_complex_datalogic() {
        // Test concatenation
        let node = Node {
            aturi: "test".to_string(),
            blueprint: "test".to_string(),
            node_type: "test".to_string(),
            payload: json!({
                "cat": [
                    "Hello, ",
                    {"val": ["name"]},
                    "!"
                ]
            }),
            configuration: json!({}),
            created_at: Utc::now(),
        };

        let input = json!({
            "name": "World"
        });

        let result = extract_payload_data(&node, &input).unwrap();
        assert_eq!(result, json!("Hello, World!"));
    }

    #[test]
    fn test_extract_payload_data_object_with_comparison() {
        let node = Node {
            aturi: "test".to_string(),
            blueprint: "test".to_string(),
            node_type: "test".to_string(),
            payload: json!({
                "==": [{"val": ["status"]}, "active"]
            }),
            configuration: json!({}),
            created_at: Utc::now(),
        };

        let input = json!({
            "status": "active"
        });

        let result = extract_payload_data(&node, &input).unwrap();
        assert_eq!(result, json!(true));

        let input = json!({
            "status": "inactive"
        });

        let result = extract_payload_data(&node, &input).unwrap();
        assert_eq!(result, json!(false));
    }

    #[test]
    fn test_extract_payload_data_invalid_payload_type_number() {
        let node = Node {
            aturi: "test".to_string(),
            blueprint: "test".to_string(),
            node_type: "test".to_string(),
            payload: json!(123),
            configuration: json!({}),
            created_at: Utc::now(),
        };

        let input = json!({"data": "value"});

        let result = extract_payload_data(&node, &input);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("error-iftta-engine-7")
        );
    }

    #[test]
    fn test_extract_payload_data_invalid_payload_type_array() {
        let node = Node {
            aturi: "test".to_string(),
            blueprint: "test".to_string(),
            node_type: "test".to_string(),
            payload: json!(["array", "payload"]),
            configuration: json!({}),
            created_at: Utc::now(),
        };

        let input = json!({"data": "value"});

        let result = extract_payload_data(&node, &input);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("error-iftta-engine-7")
        );
    }

    #[test]
    fn test_extract_payload_data_invalid_payload_type_boolean() {
        let node = Node {
            aturi: "test".to_string(),
            blueprint: "test".to_string(),
            node_type: "test".to_string(),
            payload: json!(true),
            configuration: json!({}),
            created_at: Utc::now(),
        };

        let input = json!({"data": "value"});

        let result = extract_payload_data(&node, &input);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("error-iftta-engine-7")
        );
    }

    #[test]
    fn test_extract_payload_data_invalid_payload_type_null() {
        let node = Node {
            aturi: "test".to_string(),
            blueprint: "test".to_string(),
            node_type: "test".to_string(),
            payload: json!(null),
            configuration: json!({}),
            created_at: Utc::now(),
        };

        let input = json!({"data": "value"});

        let result = extract_payload_data(&node, &input);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("error-iftta-engine-7")
        );
    }

    #[test]
    fn test_extract_payload_data_empty_string_field() {
        let node = Node {
            aturi: "test".to_string(),
            blueprint: "test".to_string(),
            node_type: "test".to_string(),
            payload: json!(""),
            configuration: json!({}),
            created_at: Utc::now(),
        };

        let input = json!({
            "": "empty_key_value",
            "other": "field"
        });

        let result = extract_payload_data(&node, &input).unwrap();
        assert_eq!(result, json!("empty_key_value"));
    }

    #[test]
    fn test_extract_payload_data_special_characters_in_field_name() {
        let node = Node {
            aturi: "test".to_string(),
            blueprint: "test".to_string(),
            node_type: "test".to_string(),
            payload: json!("field.with.dots"),
            configuration: json!({}),
            created_at: Utc::now(),
        };

        let input = json!({
            "field.with.dots": "special_value",
            "other": "field"
        });

        let result = extract_payload_data(&node, &input).unwrap();
        assert_eq!(result, json!("special_value"));
    }

    #[test]
    fn test_extract_payload_data_with_empty_input() {
        let node = Node {
            aturi: "test".to_string(),
            blueprint: "test".to_string(),
            node_type: "test".to_string(),
            payload: json!("field"),
            configuration: json!({}),
            created_at: Utc::now(),
        };

        let input = json!({});

        let result = extract_payload_data(&node, &input);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("error-iftta-engine-6")
        );
    }

    #[test]
    fn test_extract_payload_data_with_non_object_input() {
        let node = Node {
            aturi: "test".to_string(),
            blueprint: "test".to_string(),
            node_type: "test".to_string(),
            payload: json!("field"),
            configuration: json!({}),
            created_at: Utc::now(),
        };

        // Input is an array, not an object
        let input = json!(["not", "an", "object"]);

        let result = extract_payload_data(&node, &input);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("error-iftta-engine-6")
        );
    }

    #[test]
    fn test_is_webhook_success_status() {
        assert!(is_webhook_success_status(200));
        assert!(is_webhook_success_status(204));
        assert!(!is_webhook_success_status(201));
        assert!(!is_webhook_success_status(202));
        assert!(!is_webhook_success_status(301));
        assert!(!is_webhook_success_status(302));
        assert!(!is_webhook_success_status(400));
        assert!(!is_webhook_success_status(500));
    }

    #[test]
    fn test_extract_headers_from_config() {
        let config = json!({
            "url": "https://example.com",
            "headers": {
                "Authorization": "Bearer token",
                "X-Custom": "value"
            }
        });

        let headers = extract_headers_from_config(&config).unwrap();
        assert_eq!(
            headers.get("Content-Type"),
            Some(&"application/json".to_string())
        );
        assert_eq!(
            headers.get("Authorization"),
            Some(&"Bearer token".to_string())
        );
        assert_eq!(headers.get("X-Custom"), Some(&"value".to_string()));
    }

    #[test]
    fn test_clone_input_with_results() {
        let input = json!({
            "data": "test",
            "count": 42
        });

        let results = json!({
            "uri": "at://did:plc:abc/collection/xyz",
            "cid": "bafyrei..."
        });

        let output = clone_input_with_results(&input, "publish_results", results);

        assert_eq!(output["data"], "test");
        assert_eq!(output["count"], 42);
        assert_eq!(
            output["publish_results"]["uri"],
            "at://did:plc:abc/collection/xyz"
        );
    }

    #[test]
    fn test_validate_https_url() {
        assert!(validate_https_url("https://example.com").is_ok());
        assert!(validate_https_url("https://api.example.com/webhook").is_ok());
        assert!(validate_https_url("http://example.com").is_err());
        assert!(validate_https_url("ftp://example.com").is_err());
        assert!(validate_https_url("example.com").is_err());
    }
}
