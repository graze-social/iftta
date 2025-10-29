//! Common utilities for webhook publishing nodes.
//!
//! This module provides shared functionality for webhook evaluation used by
//! both direct and queue-based webhook publishers.

use anyhow::Result;
use serde_json::Value;

use super::node_type_common::{extract_payload_data, validate_https_url};
use crate::errors::EngineError;
use crate::storage::node::Node;

/// Extract webhook payload data from node configuration.
///
/// This is a wrapper around the common extract_payload_data function
/// for backward compatibility with existing webhook code.
/// Also handles JSON strings by parsing them into objects.
pub fn extract_webhook_payload(node: &Node, input: &Value) -> Result<Value> {
    let mut payload_data = extract_payload_data(node, input)?;

    // If payload_data is a string, try to parse it as JSON
    if let Some(payload_str) = payload_data.as_str() {
        // Try to parse as JSON - if it fails, keep it as a string
        if let Ok(parsed) = serde_json::from_str::<Value>(payload_str) {
            payload_data = parsed;
        }
    }

    Ok(payload_data)
}

/// Validate webhook configuration.
///
/// Ensures:
/// - Configuration is an object
/// - Contains required "url" field with HTTPS URL
/// - Optional "headers" field is an object if present
pub fn validate_webhook_config(config: &Value) -> Result<()> {
    if !config.is_object() {
        return Err(EngineError::InvalidNodeConfiguration {
            node_type: "publish_webhook".to_string(),
            details: "Configuration must be an object".to_string(),
        }
        .into());
    }

    // Check required URL field
    let url = config.get("url").and_then(|v| v.as_str()).ok_or_else(|| {
        EngineError::InvalidNodeConfiguration {
            node_type: "publish_webhook".to_string(),
            details: "Configuration must contain 'url' field".to_string(),
        }
    })?;

    // Validate URL is HTTPS
    validate_https_url(url).map_err(|_| EngineError::InvalidNodeConfiguration {
        node_type: "publish_webhook".to_string(),
        details: format!("Webhook URL must use HTTPS protocol, got: {}", url),
    })?;

    // Validate headers if present
    if let Some(headers) = config.get("headers") {
        if !headers.is_object() {
            return Err(EngineError::InvalidNodeConfiguration {
                node_type: "publish_webhook".to_string(),
                details: "Configuration 'headers' field must be an object".to_string(),
            }
            .into());
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use serde_json::json;

    #[test]
    fn test_extract_webhook_payload_string() {
        let node = Node {
            aturi: "test".to_string(),
            blueprint: "test".to_string(),
            node_type: "publish_webhook".to_string(),
            payload: json!("data"),
            configuration: json!({}),
            created_at: Utc::now(),
        };

        let input = json!({
            "data": {"message": "hello"},
            "other": "field"
        });

        let result = extract_webhook_payload(&node, &input).unwrap();
        assert_eq!(result, json!({"message": "hello"}));
    }

    #[test]
    fn test_extract_webhook_payload_string_missing_field() {
        let node = Node {
            aturi: "test".to_string(),
            blueprint: "test".to_string(),
            node_type: "publish_webhook".to_string(),
            payload: json!("missing"),
            configuration: json!({}),
            created_at: Utc::now(),
        };

        let input = json!({"data": "value"});

        let result = extract_webhook_payload(&node, &input);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("not found") || err_msg.contains("Field") || err_msg.contains("missing"));
    }

    #[test]
    fn test_extract_webhook_payload_object() {
        let node = Node {
            aturi: "test".to_string(),
            blueprint: "test".to_string(),
            node_type: "publish_webhook".to_string(),
            payload: json!({"val": ["user", "profile"]}),
            configuration: json!({}),
            created_at: Utc::now(),
        };

        let input = json!({
            "user": {
                "profile": {"name": "Alice", "age": 30}
            }
        });

        let result = extract_webhook_payload(&node, &input).unwrap();
        assert_eq!(result, json!({"name": "Alice", "age": 30}));
    }

    #[test]
    fn test_extract_webhook_payload_invalid_type() {
        let node = Node {
            aturi: "test".to_string(),
            blueprint: "test".to_string(),
            node_type: "publish_webhook".to_string(),
            payload: json!(123),
            configuration: json!({}),
            created_at: Utc::now(),
        };

        let input = json!({});

        let result = extract_webhook_payload(&node, &input);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("Invalid") || err_msg.contains("payload"));
    }

    #[test]
    fn test_validate_webhook_config_valid() {
        let config = json!({
            "url": "https://example.com/webhook"
        });
        assert!(validate_webhook_config(&config).is_ok());

        let config_with_headers = json!({
            "url": "https://example.com/webhook",
            "headers": {
                "Authorization": "Bearer token"
            }
        });
        assert!(validate_webhook_config(&config_with_headers).is_ok());
    }

    #[test]
    fn test_validate_webhook_config_missing_url() {
        let config = json!({});
        let result = validate_webhook_config(&config);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("must contain 'url' field")
        );
    }

    #[test]
    fn test_validate_webhook_config_non_https() {
        let config = json!({
            "url": "http://example.com/webhook"
        });
        let result = validate_webhook_config(&config);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("must use HTTPS protocol")
        );
    }

    #[test]
    fn test_validate_webhook_config_invalid_headers() {
        let config = json!({
            "url": "https://example.com/webhook",
            "headers": "invalid"
        });
        let result = validate_webhook_config(&config);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("'headers' field must be an object")
        );
    }
}
