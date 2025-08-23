//! Prototype service for managing and instantiating prototypes.
//!
//! This service handles the conversion of prototypes to blueprints by
//! replacing placeholders with actual values.

use anyhow::{Context, Result};
use chrono::Utc;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use ulid::Ulid;

use crate::errors::ValidationError;
use crate::storage::{Blueprint, BlueprintStorage, Node, NodeStorage, Prototype, PrototypeStorage};

/// Service for managing prototypes and instantiating blueprints from them.
pub struct PrototypeService {
    prototype_storage: Arc<dyn PrototypeStorage>,
    blueprint_storage: Arc<dyn BlueprintStorage>,
    node_storage: Arc<dyn NodeStorage>,
}

impl PrototypeService {
    /// Create a new prototype service.
    pub fn new(
        prototype_storage: Arc<dyn PrototypeStorage>,
        blueprint_storage: Arc<dyn BlueprintStorage>,
        node_storage: Arc<dyn NodeStorage>,
    ) -> Self {
        Self {
            prototype_storage,
            blueprint_storage,
            node_storage,
        }
    }

    /// Create a new prototype with validation.
    pub async fn create_prototype(&self, prototype: &Prototype) -> Result<()> {
        // Validate the prototype
        let validation = prototype.validate();
        if !validation.is_valid {
            let errors = validation.errors.join("; ");
            anyhow::bail!("Invalid prototype: {}", errors);
        }

        // Warn about unused placeholders
        if !validation.unused_placeholders.is_empty() {
            tracing::warn!(
                "Prototype has unused placeholders: {:?}",
                validation.unused_placeholders
            );
        }

        // Store the prototype
        self.prototype_storage
            .create_prototype(prototype)
            .await
            .context("Failed to create prototype")?;

        Ok(())
    }

    /// Update an existing prototype with validation.
    pub async fn update_prototype(&self, prototype: &Prototype) -> Result<()> {
        // Validate the prototype
        let validation = prototype.validate();
        if !validation.is_valid {
            let errors = validation.errors.join("; ");
            anyhow::bail!("Invalid prototype: {}", errors);
        }

        // Update the prototype
        self.prototype_storage
            .update_prototype(prototype)
            .await
            .context("Failed to update prototype")?;

        Ok(())
    }

    /// Instantiate a blueprint from a prototype.
    ///
    /// Creates a new blueprint by replacing placeholders in the prototype
    /// with the provided values.
    pub async fn instantiate_blueprint(
        &self,
        prototype_aturi: &str,
        did: &str,
        placeholder_values: HashMap<String, String>,
    ) -> Result<String> {
        // Load the prototype
        let prototype = self
            .prototype_storage
            .get_prototype(prototype_aturi)
            .await
            .context("Failed to load prototype")?
            .ok_or_else(|| ValidationError::InvalidBlueprintStructure {
                details: format!("Prototype not found: {}", prototype_aturi),
            })?;

        // Instantiate the nodes with placeholder values
        let instantiated_nodes = prototype
            .instantiate(&placeholder_values)
            .context("Failed to instantiate prototype")?;

        // Generate a unique AT-URI for the new blueprint
        let blueprint_id = Ulid::new().to_string();
        let blueprint_aturi = format!(
            "at://{}/tools.graze.ifthisthenat.blueprint/{}",
            did,
            blueprint_id
        );

        // Create the blueprint
        let blueprint = Blueprint {
            aturi: blueprint_aturi.clone(),
            did: did.to_string(),
            enabled: true,
            node_order: Vec::new(), // Will be populated when nodes are added
            error: None,
            created_at: Utc::now(),
        };

        // Store the blueprint
        self.blueprint_storage
            .create_blueprint(&blueprint)
            .await
            .context("Failed to create blueprint")?;

        // Convert the instantiated nodes to Node objects and store them
        let nodes = self
            .convert_nodes_from_json(&blueprint_aturi, &instantiated_nodes)
            .await
            .context("Failed to convert nodes")?;

        // Update the blueprint with node order
        let node_order: Vec<String> = nodes.iter().map(|n| n.aturi.clone()).collect();
        let updated_blueprint = Blueprint {
            node_order,
            ..blueprint
        };

        self.blueprint_storage
            .update_blueprint(&updated_blueprint)
            .await
            .context("Failed to update blueprint with node order")?;

        // Record the instantiation for analytics
        self.record_instantiation(&prototype.aturi, &blueprint_aturi, did, &placeholder_values)
            .await?;

        Ok(blueprint_aturi)
    }

    /// Convert JSON nodes to Node objects and store them.
    async fn convert_nodes_from_json(
        &self,
        blueprint_aturi: &str,
        nodes_json: &Value,
    ) -> Result<Vec<Node>> {
        let nodes_array = nodes_json
            .as_array()
            .ok_or_else(|| ValidationError::InvalidBlueprintStructure {
                details: "Nodes must be an array".to_string(),
            })?;

        let mut nodes = Vec::new();

        for (index, node_json) in nodes_array.iter().enumerate() {
            let node_obj = node_json
                .as_object()
                .ok_or_else(|| ValidationError::InvalidBlueprintStructure {
                    details: format!("Node at index {} must be an object", index),
                })?;

            // Extract node fields
            let node_type = node_obj
                .get("node_type")
                .and_then(|v| v.as_str())
                .ok_or_else(|| ValidationError::MissingRequiredField {
                    field_name: "node_type".to_string(),
                    context: format!("node at index {}", index),
                })?
                .to_string();

            let configuration = node_obj
                .get("configuration")
                .cloned()
                .unwrap_or_else(|| Value::Object(serde_json::Map::new()));

            let payload = node_obj
                .get("payload")
                .cloned()
                .unwrap_or_else(|| Value::Object(serde_json::Map::new()));

            // Generate node AT-URI
            let node_id = Ulid::new().to_string();
            let node_aturi = format!("{}/node/{}", blueprint_aturi, node_id);

            // Create the node
            let node = Node {
                aturi: node_aturi.clone(),
                blueprint: blueprint_aturi.to_string(),
                node_type,
                configuration,
                payload,
                created_at: Utc::now(),
            };

            // Store the node
            self.node_storage
                .upsert_node(&node)
                .await
                .context("Failed to create node")?;

            nodes.push(node);
        }

        Ok(nodes)
    }

    /// Record a prototype instantiation for analytics.
    async fn record_instantiation(
        &self,
        prototype_aturi: &str,
        blueprint_aturi: &str,
        did: &str,
        _placeholder_values: &HashMap<String, String>,
    ) -> Result<()> {
        // This would insert into the prototype_instantiations table
        // For now, just log it
        tracing::info!(
            prototype = prototype_aturi,
            blueprint = blueprint_aturi,
            user = did,
            "Prototype instantiated"
        );

        Ok(())
    }

    /// Validate that a set of placeholder values is complete and valid for a prototype.
    pub async fn validate_placeholder_values(
        &self,
        prototype_aturi: &str,
        values: &HashMap<String, String>,
    ) -> Result<Vec<String>> {
        let prototype = self
            .prototype_storage
            .get_prototype(prototype_aturi)
            .await
            .context("Failed to load prototype")?
            .ok_or_else(|| ValidationError::InvalidBlueprintStructure {
                details: format!("Prototype not found: {}", prototype_aturi),
            })?;

        let mut errors = Vec::new();

        // Check each placeholder
        for placeholder in &prototype.placeholders {
            if let Some(value) = values.get(&placeholder.id) {
                // Validate the value type
                if let Err(e) = placeholder.value_type.validate(value) {
                    errors.push(format!(
                        "Invalid value for '{}': {}",
                        placeholder.id,
                        e
                    ));
                }

                // Validate against pattern if provided
                if let Some(pattern) = &placeholder.validation_pattern {
                    let re = regex::Regex::new(pattern)?;
                    if !re.is_match(value) {
                        errors.push(format!(
                            "Value for '{}' doesn't match required pattern",
                            placeholder.id
                        ));
                    }
                }
            } else if placeholder.required && placeholder.default_value.is_none() {
                errors.push(format!(
                    "Missing required placeholder: {}",
                    placeholder.id
                ));
            }
        }

        Ok(errors)
    }

    /// Clone an existing prototype for a user.
    pub async fn clone_prototype(
        &self,
        prototype_aturi: &str,
        new_owner_did: &str,
        new_name: Option<String>,
    ) -> Result<String> {
        // Load the original prototype
        let original = self
            .prototype_storage
            .get_prototype(prototype_aturi)
            .await
            .context("Failed to load prototype")?
            .ok_or_else(|| ValidationError::InvalidBlueprintStructure {
                details: format!("Prototype not found: {}", prototype_aturi),
            })?;

        // Check if it's owned by the requester (no public/private distinction anymore)
        if original.did != new_owner_did {
            anyhow::bail!("Cannot clone prototype owned by another user");
        }

        // Generate new AT-URI
        let prototype_id = Ulid::new().to_string();
        let new_aturi = format!(
            "at://{}/tools.graze.ifthisthenat.prototype/{}",
            new_owner_did,
            prototype_id
        );

        // Create the cloned prototype
        let cloned = Prototype {
            aturi: new_aturi.clone(),
            did: new_owner_did.to_string(),
            name: new_name.unwrap_or_else(|| format!("Copy of {}", original.name)),
            description: original.description.clone(),
            nodes: original.nodes.clone(),
            placeholders: original.placeholders.clone(),
            shared: false, // Cloned prototypes are private by default
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        // Store the cloned prototype
        self.prototype_storage
            .create_prototype(&cloned)
            .await
            .context("Failed to create cloned prototype")?;

        Ok(new_aturi)
    }
}

#[cfg(test)]
mod tests {
    // Mock implementations would go here for testing
    // For now, just test the validation logic

    #[test]
    fn test_validate_placeholder_values() {
        // This would test the validation logic with mock storage
        // Implementation omitted for brevity
    }
}
