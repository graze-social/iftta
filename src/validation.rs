//! Unified validation logic for blueprints and nodes

use crate::errors::ValidationError;
use serde_json::Value;
use std::collections::HashSet;

use crate::{
    constants::{
        NODE_TYPE_JETSTREAM_ENTRY, NODE_TYPE_PUBLISH_RECORD, NODE_TYPE_PUBLISH_WEBHOOK,
        is_action_node_type, is_entry_node_type, is_valid_node_type,
    },
    engine::node_type_jetstream_entry::validate_jetstream_entry_config,
    storage::node::Node,
};

/// Validates that a blueprint's nodes follow the required rules
pub struct Validator;

impl Validator {
    /// Validate that blueprint nodes follow the required rules
    ///
    /// Rules:
    /// 1. A blueprint MUST have at least 2 nodes
    /// 2. A blueprint MUST have exactly 1 entry node
    /// 3. The first node MUST be an entry node (jetstream_entry, webhook_entry, periodic_entry, or zap_entry)
    /// 4. A blueprint MUST have at least one action node (publish_record, publish_webhook, or debug_action)
    /// 5. The last node MUST be an action node
    pub fn validate_blueprint_nodes(nodes: &[Node]) -> Result<(), ValidationError> {
        // Rule 1: Must have at least 2 nodes
        if nodes.len() < 2 {
            return Err(ValidationError::InvalidBlueprintStructure {
                details: format!(
                    "Blueprint must have at least 2 nodes (an entry node and an action node), found {}",
                    nodes.len()
                ),
            });
        }

        // Rule 2 & 3: First node must be an entry node, and only one entry node allowed
        let first_node = &nodes[0];
        if !is_entry_node_type(&first_node.node_type) {
            return Err(ValidationError::InvalidBlueprintStructure {
                details: format!(
                    "First node must be an entry node (jetstream_entry, webhook_entry, periodic_entry, or zap_entry), found '{}'",
                    first_node.node_type
                ),
            });
        }

        // Check that no other nodes are entry nodes (ensures exactly one entry node)
        for (index, node) in nodes.iter().enumerate().skip(1) {
            if is_entry_node_type(&node.node_type) {
                return Err(ValidationError::InvalidBlueprintStructure {
                    details: format!(
                        "Blueprint must have exactly one entry node. Found additional entry node '{}' at position {}",
                        node.node_type,
                        index + 1 // Convert to 1-based indexing for user-friendly error message
                    ),
                });
            }
        }

        // Rule 4: Must have at least one action node
        let has_action = nodes.iter().any(|n| is_action_node_type(&n.node_type));
        if !has_action {
            return Err(ValidationError::InvalidBlueprintStructure {
                details: "Blueprint must have at least one action node (publish_record, publish_webhook, or debug_action)".to_string(),
            });
        }

        // Rule 5: Last node must be an action node
        let last_node = nodes.last().unwrap(); // Safe because we checked len >= 2
        if !is_action_node_type(&last_node.node_type) {
            return Err(ValidationError::InvalidBlueprintStructure {
                details: format!(
                    "Last node must be an action node (publish_record, publish_webhook, or debug_action), found '{}'",
                    last_node.node_type
                ),
            });
        }

        Ok(())
    }

    /// Validate a single node type
    pub fn validate_node_type(node_type: &str) -> Result<(), ValidationError> {
        if !is_valid_node_type(node_type) {
            return Err(ValidationError::InvalidNodeType {
                node_type: node_type.to_string(),
            });
        }
        Ok(())
    }

    /// Validate that a node can be added at a specific position
    pub fn validate_node_position(node_type: &str, position: usize) -> Result<(), ValidationError> {
        if position == 0 {
            // First node must be an entry node
            if !is_entry_node_type(node_type) {
                return Err(ValidationError::InvalidBlueprintStructure {
                    details: "First node must be an entry node (jetstream_entry, webhook_entry, periodic_entry, or zap_entry)".to_string(),
                });
            }
        } else {
            // Non-first nodes cannot be entry nodes
            if is_entry_node_type(node_type) {
                return Err(ValidationError::EntryNodeNotFirst);
            }
        }
        Ok(())
    }

    /// Validate publish_record node configuration
    pub fn validate_publish_record_config(
        configuration: &Value,
        _authenticated_did: &str,
    ) -> Result<(), ValidationError> {
        // Configuration must be an object
        if !configuration.is_object() {
            return Err(ValidationError::InvalidNodeConfiguration {
                node_type: "publish_record".to_string(),
                details: "Configuration must be an object for publish_record nodes".to_string(),
            });
        }

        // Check optional 'record_key' field if present
        if let Some(record_key) = configuration.get("record_key") {
            if !record_key.is_string() {
                return Err(ValidationError::InvalidFieldType {
                    field_name: "record_key".to_string(),
                    context: "publish_record configuration".to_string(),
                    expected_type: "string".to_string(),
                });
            }
        }

        Ok(())
    }

    /// Validate publish_webhook node configuration
    pub fn validate_publish_webhook_config(configuration: &Value) -> Result<(), ValidationError> {
        // Configuration must be an object
        if !configuration.is_object() {
            return Err(ValidationError::InvalidNodeConfiguration {
                node_type: "publish_webhook".to_string(),
                details: "Configuration must be an object for publish_webhook nodes".to_string(),
            });
        }

        let config_obj = configuration.as_object().unwrap();

        // Check for required 'url' field
        let url = config_obj
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ValidationError::MissingRequiredField {
                field_name: "url".to_string(),
                context: "publish_webhook configuration".to_string(),
            })?;

        // Validate that URL is HTTPS
        if !url.starts_with("https://") {
            return Err(ValidationError::InvalidUrlFormat {
                url: url.to_string(),
                details: "publish_webhook URL must use HTTPS protocol".to_string(),
            });
        }

        // Parse URL to validate it's well-formed
        url::Url::parse(url).map_err(|e| ValidationError::InvalidUrlFormat {
            url: url.to_string(),
            details: format!("Invalid URL format for publish_webhook: {}", e),
        })?;

        // Validate optional headers field
        if let Some(headers) = config_obj.get("headers") {
            if !headers.is_object() {
                return Err(ValidationError::InvalidFieldType {
                    field_name: "headers".to_string(),
                    context: "publish_webhook configuration".to_string(),
                    expected_type: "object".to_string(),
                });
            }
        }

        Ok(())
    }

    /// Validate a node's configuration based on its type
    pub fn validate_node_config(
        node_type: &str,
        configuration: &Value,
        authenticated_did: &str,
    ) -> Result<(), ValidationError> {
        match node_type {
            NODE_TYPE_JETSTREAM_ENTRY => {
                validate_jetstream_entry_config(configuration).map_err(|e| {
                    ValidationError::InvalidNodeConfiguration {
                        node_type: NODE_TYPE_JETSTREAM_ENTRY.to_string(),
                        details: format!("Jetstream entry configuration validation failed: {}", e),
                    }
                })?;
            }
            NODE_TYPE_PUBLISH_RECORD => {
                Self::validate_publish_record_config(configuration, authenticated_did)?;
            }
            NODE_TYPE_PUBLISH_WEBHOOK => {
                Self::validate_publish_webhook_config(configuration)?;
            }
            "transform" => {
                // Transform node configuration must be an object (currently empty)
                if !configuration.is_object() {
                    return Err(ValidationError::InvalidNodeConfiguration {
                        node_type: "transform".to_string(),
                        details: "Transform node configuration must be an object".to_string(),
                    });
                }
            }
            _ => {
                // No specific validation for other node types
            }
        }
        Ok(())
    }

    /// Validate node payload structure based on node type
    pub fn validate_node_payload(node_type: &str, payload: &Value) -> Result<(), ValidationError> {
        match node_type {
            "jetstream_entry" | "webhook_entry" | "zap_entry" | "periodic_entry" | "condition" => {
                // Allow boolean or object payloads
                if payload.is_boolean() {
                    // Boolean payload is valid for simple true/false entry conditions
                    return Ok(());
                }

                // Webhook and zap entry payload validation
                if !payload.is_object() {
                    return Err(ValidationError::DataTypeValidationFailed {
                        field_name: "payload".to_string(),
                        expected: "boolean or object".to_string(),
                        actual: format!("{:?}", payload),
                    });
                }
            }
            "transform" => {
                // Transform nodes accept object or array of objects payloads
                if payload.is_object() {
                    // Single transformation - valid
                    return Ok(());
                } else if payload.is_array() {
                    // Array of transformations - validate each element is an object
                    let array = payload.as_array().unwrap();
                    if array.is_empty() {
                        return Err(ValidationError::TransformPayloadInvalid {
                            details: "Transform array payload cannot be empty".to_string(),
                        });
                    }
                    for (index, element) in array.iter().enumerate() {
                        if !element.is_object() {
                            return Err(ValidationError::TransformPayloadInvalid {
                                details: format!(
                                    "Transform array element {} must be an object, got: {:?}",
                                    index, element
                                ),
                            });
                        }
                    }
                    return Ok(());
                } else {
                    return Err(ValidationError::TransformPayloadInvalid {
                        details: "Transform node payload must be an object or array of objects"
                            .to_string(),
                    });
                }
            }
            "publish_record" => {
                // Validate publish_record payload - can be string or object
                if !payload.is_string() && !payload.is_object() {
                    return Err(ValidationError::DataTypeValidationFailed {
                        field_name: "payload".to_string(),
                        expected: "string or object".to_string(),
                        actual: format!("{:?}", payload),
                    });
                }
            }
            "publish_webhook_direct" | "publish_webhook_queue" | "publish_webhook" => {
                // Validate webhook payload - can be string or object
                if !payload.is_string() && !payload.is_object() {
                    return Err(ValidationError::DataTypeValidationFailed {
                        field_name: "payload".to_string(),
                        expected: "string or object".to_string(),
                        actual: format!("{:?}", payload),
                    });
                }
            }
            "facet_text" => {
                // Validate facet_text payload - can be string or object
                if !payload.is_string() && !payload.is_object() {
                    return Err(ValidationError::DataTypeValidationFailed {
                        field_name: "payload".to_string(),
                        expected: "string or object".to_string(),
                        actual: format!("{:?}", payload),
                    });
                }
            }
            "parse_aturi" => {
                // Validate parse_aturi payload - can be string or object
                if !payload.is_string() && !payload.is_object() {
                    return Err(ValidationError::DataTypeValidationFailed {
                        field_name: "payload".to_string(),
                        expected: "string or object".to_string(),
                        actual: format!("{:?}", payload),
                    });
                }
            }
            "get_record" => {
                // Validate get_record payload - can be string or object
                if !payload.is_string() && !payload.is_object() {
                    return Err(ValidationError::DataTypeValidationFailed {
                        field_name: "payload".to_string(),
                        expected: "string or object".to_string(),
                        actual: format!("{:?}", payload),
                    });
                }
            }
            "debug_action" => {
                // Debug action can have any payload
            }
            _ => {
                // Unknown node types are already caught by validate_node_type
            }
        }

        Ok(())
    }

    /// Check if a blueprint's publish_record nodes are allowed based on collection constraints
    ///
    /// Note: With the new publish_record design, collection is determined at runtime
    /// from the payload evaluation, not from configuration. This function is kept
    /// for backward compatibility but always returns Ok.
    pub fn validate_publish_collections(
        _nodes: &[Node],
        _allowed_collections: &HashSet<String>,
    ) -> Result<(), ValidationError> {
        // Collection validation is no longer applicable since collection
        // is determined at runtime from payload evaluation
        Ok(())
    }

    /// Validate that OAuth scopes are compatible with publish_record collection constraints
    ///
    /// This validation ensures:
    /// 1. If publish_record node type is enabled, OAuth scopes must have at least one "repo:" scope
    /// 2. If OAuth has "repo:*" or "transition:generic", all collections are allowed (blanket access)
    /// 3. Otherwise, each constrained collection must have a corresponding "repo:{collection}" scope
    pub fn validate_oauth_scopes_for_collections(
        oauth_scopes: Option<&str>,
        disabled_node_types: &HashSet<String>,
        allowed_publish_collections: &HashSet<String>,
    ) -> Result<(), ValidationError> {
        // Check if publish_record node type is enabled
        let publish_record_enabled = !disabled_node_types.contains(NODE_TYPE_PUBLISH_RECORD);

        if !publish_record_enabled {
            // If publish_record is disabled, no OAuth scope validation needed
            return Ok(());
        }

        // If publish_record is enabled, we need OAuth scopes
        let scopes = match oauth_scopes {
            Some(s) if !s.trim().is_empty() => s,
            _ => {
                return Err(ValidationError::InvalidNodeConfiguration {
                    node_type: "publish_record".to_string(),
                    details: "publish_record node type is enabled but no OAuth scopes are configured. OAuth scopes with repository access are required.".to_string(),
                });
            }
        };

        // Parse scopes into a set for easier lookup
        let scope_set: HashSet<&str> = scopes.split_whitespace().collect();

        // Check for blanket repository access first
        let has_blanket_access =
            scope_set.contains("repo:*") || scope_set.contains("transition:generic");

        if has_blanket_access {
            // Blanket access satisfies all collection requirements
            return Ok(());
        }

        // If no blanket access, check if we have any repo scopes at all
        let has_repo_scope = scope_set.iter().any(|scope| scope.starts_with("repo:"));

        if !has_repo_scope {
            return Err(ValidationError::InvalidNodeConfiguration {
                node_type: "publish_record".to_string(),
                details: "publish_record node type is enabled but OAuth scopes contain no repository access permissions. At least one 'repo:' scope is required.".to_string(),
            });
        }

        // If we have collection constraints, validate each one has a corresponding scope
        if !allowed_publish_collections.is_empty() {
            for collection in allowed_publish_collections {
                let required_scope = format!("repo:{}", collection);
                if !scope_set.contains(required_scope.as_str()) {
                    return Err(ValidationError::InvalidNodeConfiguration {
                        node_type: "publish_record".to_string(),
                        details: format!(
                            "Collection '{}' is allowed in publish_record constraints but OAuth scopes are missing 'repo:{}' permission. Either add the scope or remove the collection constraint.",
                            collection, collection
                        ),
                    });
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use serde_json::json;

    #[test]
    fn test_validate_blueprint_nodes_minimum_valid() {
        // Valid blueprint with exactly 2 nodes: entry + action
        let nodes = vec![
            Node {
                aturi: "at://did:plc:test/tools.graze.ifthisthenat.node/1".to_string(),
                blueprint: "at://did:plc:test/tools.graze.ifthisthenat.blueprint/1".to_string(),
                node_type: "jetstream_entry".to_string(),
                payload: json!({}),
                configuration: json!({}),
                created_at: Utc::now(),
            },
            Node {
                aturi: "at://did:plc:test/tools.graze.ifthisthenat.node/2".to_string(),
                blueprint: "at://did:plc:test/tools.graze.ifthisthenat.blueprint/1".to_string(),
                node_type: "publish_record".to_string(),
                payload: json!({}),
                configuration: json!({}),
                created_at: Utc::now(),
            },
        ];

        assert!(Validator::validate_blueprint_nodes(&nodes).is_ok());
    }

    #[test]
    fn test_validate_blueprint_nodes_too_few_nodes() {
        // Invalid: only 1 node
        let nodes = vec![Node {
            aturi: "at://did:plc:test/tools.graze.ifthisthenat.node/1".to_string(),
            blueprint: "at://did:plc:test/tools.graze.ifthisthenat.blueprint/1".to_string(),
            node_type: "jetstream_entry".to_string(),
            payload: json!({}),
            configuration: json!({}),
            created_at: Utc::now(),
        }];

        let result = Validator::validate_blueprint_nodes(&nodes);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("at least 2 nodes"));
    }

    #[test]
    fn test_validate_blueprint_nodes_first_not_entry() {
        // Invalid: first node is not an entry node
        let nodes = vec![
            Node {
                aturi: "at://did:plc:test/tools.graze.ifthisthenat.node/1".to_string(),
                blueprint: "at://did:plc:test/tools.graze.ifthisthenat.blueprint/1".to_string(),
                node_type: "condition".to_string(),
                payload: json!({}),
                configuration: json!({}),
                created_at: Utc::now(),
            },
            Node {
                aturi: "at://did:plc:test/tools.graze.ifthisthenat.node/2".to_string(),
                blueprint: "at://did:plc:test/tools.graze.ifthisthenat.blueprint/1".to_string(),
                node_type: "publish_record".to_string(),
                payload: json!({}),
                configuration: json!({}),
                created_at: Utc::now(),
            },
        ];

        let result = Validator::validate_blueprint_nodes(&nodes);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("First node must be an entry node")
        );
    }

    #[test]
    fn test_validate_blueprint_nodes_multiple_entry_nodes() {
        // Invalid: more than one entry node
        let nodes = vec![
            Node {
                aturi: "at://did:plc:test/tools.graze.ifthisthenat.node/1".to_string(),
                blueprint: "at://did:plc:test/tools.graze.ifthisthenat.blueprint/1".to_string(),
                node_type: "jetstream_entry".to_string(),
                payload: json!({}),
                configuration: json!({}),
                created_at: Utc::now(),
            },
            Node {
                aturi: "at://did:plc:test/tools.graze.ifthisthenat.node/2".to_string(),
                blueprint: "at://did:plc:test/tools.graze.ifthisthenat.blueprint/1".to_string(),
                node_type: "webhook_entry".to_string(),
                payload: json!({}),
                configuration: json!({}),
                created_at: Utc::now(),
            },
            Node {
                aturi: "at://did:plc:test/tools.graze.ifthisthenat.node/3".to_string(),
                blueprint: "at://did:plc:test/tools.graze.ifthisthenat.blueprint/1".to_string(),
                node_type: "publish_record".to_string(),
                payload: json!({}),
                configuration: json!({}),
                created_at: Utc::now(),
            },
        ];

        let result = Validator::validate_blueprint_nodes(&nodes);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("exactly one entry node")
        );
    }

    #[test]
    fn test_validate_blueprint_nodes_last_not_action() {
        // Invalid: last node is not an action node
        let nodes = vec![
            Node {
                aturi: "at://did:plc:test/tools.graze.ifthisthenat.node/1".to_string(),
                blueprint: "at://did:plc:test/tools.graze.ifthisthenat.blueprint/1".to_string(),
                node_type: "jetstream_entry".to_string(),
                payload: json!({}),
                configuration: json!({}),
                created_at: Utc::now(),
            },
            Node {
                aturi: "at://did:plc:test/tools.graze.ifthisthenat.node/2".to_string(),
                blueprint: "at://did:plc:test/tools.graze.ifthisthenat.blueprint/1".to_string(),
                node_type: "publish_record".to_string(),
                payload: json!({}),
                configuration: json!({}),
                created_at: Utc::now(),
            },
            Node {
                aturi: "at://did:plc:test/tools.graze.ifthisthenat.node/3".to_string(),
                blueprint: "at://did:plc:test/tools.graze.ifthisthenat.blueprint/1".to_string(),
                node_type: "condition".to_string(),
                payload: json!({}),
                configuration: json!({}),
                created_at: Utc::now(),
            },
        ];

        let result = Validator::validate_blueprint_nodes(&nodes);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Last node must be an action node")
        );
    }

    #[test]
    fn test_validate_blueprint_nodes_no_action_nodes() {
        // Invalid: no action nodes at all
        let nodes = vec![
            Node {
                aturi: "at://did:plc:test/tools.graze.ifthisthenat.node/1".to_string(),
                blueprint: "at://did:plc:test/tools.graze.ifthisthenat.blueprint/1".to_string(),
                node_type: "jetstream_entry".to_string(),
                payload: json!({}),
                configuration: json!({}),
                created_at: Utc::now(),
            },
            Node {
                aturi: "at://did:plc:test/tools.graze.ifthisthenat.node/2".to_string(),
                blueprint: "at://did:plc:test/tools.graze.ifthisthenat.blueprint/1".to_string(),
                node_type: "condition".to_string(),
                payload: json!({}),
                configuration: json!({}),
                created_at: Utc::now(),
            },
        ];

        let result = Validator::validate_blueprint_nodes(&nodes);
        assert!(result.is_err());
        // Should fail on multiple rules, but at least one should be about action nodes
        let error_msg = result.unwrap_err().to_string();
        assert!(error_msg.contains("action node") || error_msg.contains("Last node must be"));
    }

    #[test]
    fn test_validate_blueprint_nodes_complex_valid() {
        // Valid blueprint with multiple nodes: entry + conditions + transforms + action
        let nodes = vec![
            Node {
                aturi: "at://did:plc:test/tools.graze.ifthisthenat.node/1".to_string(),
                blueprint: "at://did:plc:test/tools.graze.ifthisthenat.blueprint/1".to_string(),
                node_type: "webhook_entry".to_string(),
                payload: json!({}),
                configuration: json!({}),
                created_at: Utc::now(),
            },
            Node {
                aturi: "at://did:plc:test/tools.graze.ifthisthenat.node/2".to_string(),
                blueprint: "at://did:plc:test/tools.graze.ifthisthenat.blueprint/1".to_string(),
                node_type: "condition".to_string(),
                payload: json!({}),
                configuration: json!({}),
                created_at: Utc::now(),
            },
            Node {
                aturi: "at://did:plc:test/tools.graze.ifthisthenat.node/3".to_string(),
                blueprint: "at://did:plc:test/tools.graze.ifthisthenat.blueprint/1".to_string(),
                node_type: "transform".to_string(),
                payload: json!({}),
                configuration: json!({}),
                created_at: Utc::now(),
            },
            Node {
                aturi: "at://did:plc:test/tools.graze.ifthisthenat.node/4".to_string(),
                blueprint: "at://did:plc:test/tools.graze.ifthisthenat.blueprint/1".to_string(),
                node_type: "publish_webhook".to_string(),
                payload: json!({}),
                configuration: json!({}),
                created_at: Utc::now(),
            },
        ];

        assert!(Validator::validate_blueprint_nodes(&nodes).is_ok());
    }

    #[test]
    fn test_validate_blueprint_nodes_all_entry_types() {
        // Test with different entry node types
        let entry_types = vec![
            "jetstream_entry",
            "webhook_entry",
            "periodic_entry",
            "zap_entry",
        ];

        for entry_type in entry_types {
            let nodes = vec![
                Node {
                    aturi: "at://did:plc:test/tools.graze.ifthisthenat.node/1".to_string(),
                    blueprint: "at://did:plc:test/tools.graze.ifthisthenat.blueprint/1".to_string(),
                    node_type: entry_type.to_string(),
                    payload: json!({}),
                    configuration: json!({}),
                    created_at: Utc::now(),
                },
                Node {
                    aturi: "at://did:plc:test/tools.graze.ifthisthenat.node/2".to_string(),
                    blueprint: "at://did:plc:test/tools.graze.ifthisthenat.blueprint/1".to_string(),
                    node_type: "debug_action".to_string(),
                    payload: json!({}),
                    configuration: json!({}),
                    created_at: Utc::now(),
                },
            ];

            assert!(
                Validator::validate_blueprint_nodes(&nodes).is_ok(),
                "Failed for entry type: {}",
                entry_type
            );
        }
    }

    #[test]
    fn test_validate_blueprint_nodes_all_action_types() {
        // Test with different action node types as last node
        let action_types = vec!["publish_record", "publish_webhook", "debug_action"];

        for action_type in action_types {
            let nodes = vec![
                Node {
                    aturi: "at://did:plc:test/tools.graze.ifthisthenat.node/1".to_string(),
                    blueprint: "at://did:plc:test/tools.graze.ifthisthenat.blueprint/1".to_string(),
                    node_type: "periodic_entry".to_string(),
                    payload: json!({}),
                    configuration: json!({}),
                    created_at: Utc::now(),
                },
                Node {
                    aturi: "at://did:plc:test/tools.graze.ifthisthenat.node/2".to_string(),
                    blueprint: "at://did:plc:test/tools.graze.ifthisthenat.blueprint/1".to_string(),
                    node_type: action_type.to_string(),
                    payload: json!({}),
                    configuration: json!({}),
                    created_at: Utc::now(),
                },
            ];

            assert!(
                Validator::validate_blueprint_nodes(&nodes).is_ok(),
                "Failed for action type: {}",
                action_type
            );
        }
    }

    #[test]
    fn test_validate_publish_webhook_valid_config() {
        let config = json!({
            "url": "https://example.com/webhook"
        });
        assert!(Validator::validate_publish_webhook_config(&config).is_ok());
    }

    #[test]
    fn test_validate_publish_webhook_empty_headers() {
        let config = json!({
            "url": "https://example.com/webhook",
            "headers": {}
        });
        assert!(Validator::validate_publish_webhook_config(&config).is_ok());
    }

    #[test]
    fn test_validate_publish_webhook_missing_url() {
        let config = json!({});
        let result = Validator::validate_publish_webhook_config(&config);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("must contain 'url' field")
        );
    }

    #[test]
    fn test_validate_publish_webhook_non_https_url() {
        let config = json!({
            "url": "http://example.com/webhook"
        });
        let result = Validator::validate_publish_webhook_config(&config);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("must use HTTPS protocol")
        );
    }

    #[test]
    fn test_validate_publish_webhook_invalid_url() {
        let config = json!({
            "url": "https://not a valid url"
        });
        let result = Validator::validate_publish_webhook_config(&config);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Invalid URL format")
        );
    }

    #[test]
    fn test_validate_publish_webhook_invalid_headers() {
        let config = json!({
            "url": "https://example.com/webhook",
            "headers": "invalid-not-object"
        });
        let result = Validator::validate_publish_webhook_config(&config);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("'headers' field must be an object")
        );
    }

    #[test]
    fn test_validate_publish_webhook_with_valid_headers() {
        let config = json!({
            "url": "https://example.com/webhook",
            "headers": {
                "Authorization": "Bearer token",
                "X-Custom-Header": "value"
            }
        });
        assert!(Validator::validate_publish_webhook_config(&config).is_ok());
    }

    #[test]
    fn test_validate_node_payload_publish_webhook() {
        // publish_webhook accepts string payload
        assert!(Validator::validate_node_payload("publish_webhook", &json!("data")).is_ok());
        assert!(
            Validator::validate_node_payload("publish_webhook_direct", &json!("record")).is_ok()
        );
        assert!(Validator::validate_node_payload("publish_webhook_queue", &json!("body")).is_ok());

        // publish_webhook accepts object payload
        assert!(
            Validator::validate_node_payload("publish_webhook", &json!({"val": ["data"]})).is_ok()
        );
        assert!(Validator::validate_node_payload("publish_webhook_direct", &json!({})).is_ok());
        assert!(Validator::validate_node_payload("publish_webhook_queue", &json!({})).is_ok());
    }

    #[test]
    fn test_validate_publish_webhook_config_not_object() {
        let config = json!("not-an-object");
        let result = Validator::validate_publish_webhook_config(&config);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Configuration must be an object")
        );
    }

    #[test]
    fn test_validate_node_config_publish_webhook() {
        let config = json!({
            "url": "https://example.com/webhook"
        });
        assert!(
            Validator::validate_node_config("publish_webhook", &config, "did:plc:test").is_ok()
        );
    }

    #[test]
    fn test_validate_node_config_publish_webhook_invalid() {
        let config = json!({
            "url": "http://example.com/webhook"
        });
        let result = Validator::validate_node_config("publish_webhook", &config, "did:plc:test");
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_oauth_scopes_publish_record_disabled() {
        // If publish_record is disabled, no validation needed
        let mut disabled_types = HashSet::new();
        disabled_types.insert("publish_record".to_string());
        let allowed_collections = HashSet::new();

        let result = Validator::validate_oauth_scopes_for_collections(
            None,
            &disabled_types,
            &allowed_collections,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_oauth_scopes_no_oauth_but_publish_record_enabled() {
        // If publish_record is enabled but no OAuth scopes, should fail
        let disabled_types = HashSet::new();
        let allowed_collections = HashSet::new();

        let result = Validator::validate_oauth_scopes_for_collections(
            None,
            &disabled_types,
            &allowed_collections,
        );
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("no OAuth scopes are configured")
        );
    }

    #[test]
    fn test_validate_oauth_scopes_empty_scopes() {
        // Empty scope string should fail
        let disabled_types = HashSet::new();
        let allowed_collections = HashSet::new();

        let result = Validator::validate_oauth_scopes_for_collections(
            Some("   "), // Whitespace only
            &disabled_types,
            &allowed_collections,
        );
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("no OAuth scopes are configured")
        );
    }

    #[test]
    fn test_validate_oauth_scopes_no_repo_scopes() {
        // If no repo scopes at all, should fail
        let disabled_types = HashSet::new();
        let allowed_collections = HashSet::new();

        let result = Validator::validate_oauth_scopes_for_collections(
            Some("openid profile email"),
            &disabled_types,
            &allowed_collections,
        );
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("no repository access permissions")
        );
    }

    #[test]
    fn test_validate_oauth_scopes_blanket_repo_star() {
        // repo:* should satisfy all constraints
        let disabled_types = HashSet::new();
        let mut allowed_collections = HashSet::new();
        allowed_collections.insert("app.bsky.feed.post".to_string());
        allowed_collections.insert("app.bsky.feed.like".to_string());

        let result = Validator::validate_oauth_scopes_for_collections(
            Some("openid profile repo:*"),
            &disabled_types,
            &allowed_collections,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_oauth_scopes_blanket_transition_generic() {
        // transition:generic should satisfy all constraints
        let disabled_types = HashSet::new();
        let mut allowed_collections = HashSet::new();
        allowed_collections.insert("app.bsky.feed.post".to_string());
        allowed_collections.insert("app.bsky.feed.like".to_string());

        let result = Validator::validate_oauth_scopes_for_collections(
            Some("openid transition:generic profile"),
            &disabled_types,
            &allowed_collections,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_oauth_scopes_specific_collections_match() {
        // Specific repo scopes should match allowed collections
        let disabled_types = HashSet::new();
        let mut allowed_collections = HashSet::new();
        allowed_collections.insert("app.bsky.feed.post".to_string());
        allowed_collections.insert("app.bsky.feed.like".to_string());

        let result = Validator::validate_oauth_scopes_for_collections(
            Some("openid repo:app.bsky.feed.post repo:app.bsky.feed.like profile"),
            &disabled_types,
            &allowed_collections,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_oauth_scopes_missing_collection_scope() {
        // Missing scope for one of the allowed collections should fail
        let disabled_types = HashSet::new();
        let mut allowed_collections = HashSet::new();
        allowed_collections.insert("app.bsky.feed.post".to_string());
        allowed_collections.insert("app.bsky.feed.like".to_string());

        let result = Validator::validate_oauth_scopes_for_collections(
            Some("openid repo:app.bsky.feed.post profile"), // Missing repo:app.bsky.feed.like
            &disabled_types,
            &allowed_collections,
        );
        assert!(result.is_err());
        let error_message = result.unwrap_err().to_string();
        assert!(error_message.contains("app.bsky.feed.like"));
        assert!(error_message.contains("missing 'repo:app.bsky.feed.like' permission"));
    }

    #[test]
    fn test_validate_oauth_scopes_no_collection_constraints() {
        // If no collection constraints, any repo scope should be fine
        let disabled_types = HashSet::new();
        let allowed_collections = HashSet::new(); // Empty = no constraints

        let result = Validator::validate_oauth_scopes_for_collections(
            Some("openid repo:some.random.collection profile"),
            &disabled_types,
            &allowed_collections,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_oauth_scopes_extra_scopes_allowed() {
        // Having extra scopes beyond what's required should be fine
        let disabled_types = HashSet::new();
        let mut allowed_collections = HashSet::new();
        allowed_collections.insert("app.bsky.feed.post".to_string());

        let result = Validator::validate_oauth_scopes_for_collections(
            Some(
                "openid repo:app.bsky.feed.post repo:app.bsky.feed.like repo:some.other.collection profile",
            ),
            &disabled_types,
            &allowed_collections,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_node_payload_jetstream_entry_boolean() {
        // Test that jetstream_entry accepts boolean payloads
        assert!(Validator::validate_node_payload("jetstream_entry", &json!(true)).is_ok());
        assert!(Validator::validate_node_payload("jetstream_entry", &json!(false)).is_ok());
    }

    #[test]
    fn test_validate_node_payload_webhook_entry_boolean() {
        // Test that webhook_entry accepts boolean payloads
        assert!(Validator::validate_node_payload("webhook_entry", &json!(true)).is_ok());
        assert!(Validator::validate_node_payload("webhook_entry", &json!(false)).is_ok());
    }

    #[test]
    fn test_validate_node_payload_zap_entry_boolean() {
        // Test that zap_entry accepts boolean payloads
        assert!(Validator::validate_node_payload("zap_entry", &json!(true)).is_ok());
        assert!(Validator::validate_node_payload("zap_entry", &json!(false)).is_ok());
    }

    #[test]
    fn test_validate_node_payload_entry_nodes_object() {
        // Test that entry nodes still accept object payloads
        let object_payload = json!({
            "collection": "app.bsky.feed.post",
            "operation": "create"
        });

        assert!(Validator::validate_node_payload("jetstream_entry", &object_payload).is_ok());
        assert!(Validator::validate_node_payload("webhook_entry", &object_payload).is_ok());
        assert!(Validator::validate_node_payload("zap_entry", &object_payload).is_ok());
    }

    #[test]
    fn test_validate_node_payload_entry_nodes_invalid_types() {
        // Test that entry nodes reject other types (string, number, array, null)
        let invalid_payloads = vec![
            json!("string"),
            json!(123),
            json!(45.67),
            json!([1, 2, 3]),
            json!(null),
        ];

        for payload in &invalid_payloads {
            let result = Validator::validate_node_payload("jetstream_entry", payload);
            assert!(
                result.is_err(),
                "jetstream_entry should reject payload: {:?}",
                payload
            );
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("Payload must be a boolean or an object")
            );

            let result = Validator::validate_node_payload("webhook_entry", payload);
            assert!(
                result.is_err(),
                "webhook_entry should reject payload: {:?}",
                payload
            );
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("Payload must be a boolean or an object")
            );

            let result = Validator::validate_node_payload("zap_entry", payload);
            assert!(
                result.is_err(),
                "zap_entry should reject payload: {:?}",
                payload
            );
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("Payload must be a boolean or an object")
            );
        }
    }

    #[test]
    fn test_validate_node_payload_periodic_entry_boolean() {
        // Test that periodic_entry accepts boolean payloads
        assert!(Validator::validate_node_payload("periodic_entry", &json!(true)).is_ok());
        assert!(Validator::validate_node_payload("periodic_entry", &json!(false)).is_ok());

        // And it should still accept objects
        assert!(Validator::validate_node_payload("periodic_entry", &json!({})).is_ok());
    }

    #[test]
    fn test_validate_node_payload_condition_boolean() {
        // Test that condition accepts boolean payloads
        assert!(Validator::validate_node_payload("condition", &json!(true)).is_ok());
        assert!(Validator::validate_node_payload("condition", &json!(false)).is_ok());

        // And it should still accept objects
        assert!(Validator::validate_node_payload("condition", &json!({})).is_ok());
    }

    #[test]
    fn test_validate_node_payload_non_entry_nodes_no_boolean() {
        // Test that non-entry nodes that require objects reject boolean payloads

        // publish_record accepts string or object but not boolean
        let result = Validator::validate_node_payload("publish_record", &json!(true));
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Payload must be a string or an object")
        );

        let result = Validator::validate_node_payload("publish_record", &json!(false));
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Payload must be a string or an object")
        );

        // publish_webhook_direct accepts string or object but not boolean
        let result = Validator::validate_node_payload("publish_webhook_direct", &json!(true));
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Payload must be a string or an object")
        );

        let result = Validator::validate_node_payload("publish_webhook_direct", &json!(false));
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Payload must be a string or an object")
        );

        // facet_text accepts string or object but not boolean
        let result = Validator::validate_node_payload("facet_text", &json!(true));
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Payload must be a string or an object")
        );

        let result = Validator::validate_node_payload("facet_text", &json!(false));
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Payload must be a string or an object")
        );

        // Note: condition accepts boolean or object payloads
        // debug_action nodes currently accept any non-null payload
    }

    #[test]
    fn test_validate_node_payload_transform() {
        // Transform accepts object payload (single transformation)
        assert!(Validator::validate_node_payload("transform", &json!({"key": "value"})).is_ok());
        assert!(Validator::validate_node_payload("transform", &json!({})).is_ok());

        // Transform accepts array of objects payload (chained transformations)
        assert!(
            Validator::validate_node_payload(
                "transform",
                &json!([{"step1": "value1"}, {"step2": "value2"}])
            )
            .is_ok()
        );
        assert!(
            Validator::validate_node_payload("transform", &json!([{"single": "step"}])).is_ok()
        );

        // Transform rejects empty array
        let result = Validator::validate_node_payload("transform", &json!([]));
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Transform array payload cannot be empty")
        );

        // Transform rejects array with non-object elements
        let result = Validator::validate_node_payload(
            "transform",
            &json!([{"valid": "object"}, "invalid string", {"another": "object"}]),
        );
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Transform array element 1 must be an object")
        );

        let result =
            Validator::validate_node_payload("transform", &json!([123, {"object": "here"}]));
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Transform array element 0 must be an object")
        );

        // Transform rejects non-object, non-array payloads
        let result = Validator::validate_node_payload("transform", &json!(true));
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Transform node payload must be an object or array of objects")
        );

        let result = Validator::validate_node_payload("transform", &json!(false));
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Transform node payload must be an object or array of objects")
        );

        let result = Validator::validate_node_payload("transform", &json!("string"));
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Transform node payload must be an object or array of objects")
        );

        let result = Validator::validate_node_payload("transform", &json!(123));
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Transform node payload must be an object or array of objects")
        );

        let result = Validator::validate_node_payload("transform", &json!(null));
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Transform node payload must be an object or array of objects")
        );
    }

    #[test]
    fn test_validate_node_config_transform() {
        // Transform configuration must be an object
        assert!(Validator::validate_node_config("transform", &json!({}), "did:plc:test").is_ok());
        assert!(
            Validator::validate_node_config(
                "transform",
                &json!({"unused": "field"}),
                "did:plc:test"
            )
            .is_ok()
        );

        // Transform rejects non-object configurations
        let result = Validator::validate_node_config("transform", &json!(true), "did:plc:test");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Transform node configuration must be an object")
        );

        let result = Validator::validate_node_config("transform", &json!("string"), "did:plc:test");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Transform node configuration must be an object")
        );

        let result = Validator::validate_node_config("transform", &json!([1, 2]), "did:plc:test");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Transform node configuration must be an object")
        );
    }

    #[test]
    fn test_validate_node_payload_publish_record() {
        // publish_record accepts string payload
        assert!(Validator::validate_node_payload("publish_record", &json!("record")).is_ok());
        assert!(Validator::validate_node_payload("publish_record", &json!("data")).is_ok());

        // publish_record accepts object payload
        assert!(
            Validator::validate_node_payload("publish_record", &json!({"val": ["record"]})).is_ok()
        );
        assert!(Validator::validate_node_payload("publish_record", &json!({})).is_ok());

        // publish_record rejects other types
        let result = Validator::validate_node_payload("publish_record", &json!(true));
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Payload must be a string or an object")
        );

        let result = Validator::validate_node_payload("publish_record", &json!(false));
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Payload must be a string or an object")
        );

        let result = Validator::validate_node_payload("publish_record", &json!(123));
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Payload must be a string or an object")
        );

        let result = Validator::validate_node_payload("publish_record", &json!([1, 2, 3]));
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Payload must be a string or an object")
        );

        let result = Validator::validate_node_payload("publish_record", &json!(null));
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Payload must be a string or an object")
        );
    }

    #[test]
    fn test_validate_node_config_publish_record() {
        // publish_record configuration must be an object
        assert!(
            Validator::validate_node_config("publish_record", &json!({}), "did:plc:test").is_ok()
        );

        // With optional record_key field
        assert!(
            Validator::validate_node_config(
                "publish_record",
                &json!({"record_key": "key123"}),
                "did:plc:test"
            )
            .is_ok()
        );

        // record_key must be a string
        let result = Validator::validate_node_config(
            "publish_record",
            &json!({"record_key": 123}),
            "did:plc:test",
        );
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("'record_key' field must be a string")
        );

        let result = Validator::validate_node_config(
            "publish_record",
            &json!({"record_key": true}),
            "did:plc:test",
        );
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("'record_key' field must be a string")
        );

        let result = Validator::validate_node_config(
            "publish_record",
            &json!({"record_key": {"nested": "object"}}),
            "did:plc:test",
        );
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("'record_key' field must be a string")
        );

        // Configuration must be an object
        let result =
            Validator::validate_node_config("publish_record", &json!("string"), "did:plc:test");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Configuration must be an object")
        );

        let result =
            Validator::validate_node_config("publish_record", &json!(true), "did:plc:test");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Configuration must be an object")
        );
    }
}
