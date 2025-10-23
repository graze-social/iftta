//! Prototype storage and management.
//!
//! Prototypes are blueprint templates with placeholders that can be instantiated
//! into concrete blueprints by providing values for the placeholders.

use crate::errors::ValidationError;
use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{FromRow, PgPool};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// A prototype is a blueprint template with placeholders.
///
/// Prototypes allow users to create reusable blueprint templates that can be
/// instantiated with different values. Placeholders in the prototype are marked
/// with `$[PLACEHOLDER_NAME]` syntax and must be defined in the placeholders field.
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Prototype {
    /// Unique AT-URI for the prototype
    pub aturi: String,

    /// DID of the prototype owner
    pub did: String,

    /// Human-readable name for the prototype
    pub name: String,

    /// Description of what this prototype does
    pub description: String,

    /// The node templates with placeholders
    pub nodes: Value,

    /// Placeholder definitions
    pub placeholders: Vec<PlaceholderDef>,

    /// Whether this prototype is publicly available to all users
    pub shared: bool,

    /// Creation timestamp
    pub created_at: DateTime<Utc>,

    /// Last update timestamp
    pub updated_at: DateTime<Utc>,
}

/// Definition of a placeholder in a prototype.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaceholderDef {
    /// Unique identifier for the placeholder (e.g., "YOU", "TARGET_DID")
    pub id: String,

    /// Human-readable label for the placeholder
    pub label: String,

    /// Description of what this placeholder is for
    pub description: String,

    /// Type hint for the placeholder value
    pub value_type: PlaceholderType,

    /// Optional default value
    pub default_value: Option<String>,

    /// Whether this placeholder is required
    pub required: bool,

    /// Optional validation pattern (regex)
    pub validation_pattern: Option<String>,

    /// Example value for documentation
    pub example: Option<String>,
}

/// Type hints for placeholder values.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlaceholderType {
    /// A DID (e.g., did:plc:xyz)
    Did,

    /// An AT-URI (e.g., at://did:plc:xyz/app.bsky.feed.post/123)
    AtUri,

    /// A URL (e.g., https://example.com)
    Url,

    /// A collection name (e.g., app.bsky.feed.post)
    Collection,

    /// A cron expression (e.g., 0 * * * *)
    Cron,

    /// Arbitrary text
    Text,

    /// A number
    Number,

    /// A boolean value
    Boolean,

    /// JSON value
    Json,
}

impl PlaceholderType {
    /// Validate a value against this placeholder type.
    pub fn validate(&self, value: &str) -> Result<()> {
        match self {
            PlaceholderType::Did => {
                if !value.starts_with("did:") {
                    return Err(ValidationError::InvalidFieldType {
                        field_name: "value".to_string(),
                        context: "DID validation".to_string(),
                        expected_type: "DID starting with 'did:'".to_string(),
                    }
                    .into());
                }
            }
            PlaceholderType::AtUri => {
                if !value.starts_with("at://") {
                    return Err(ValidationError::InvalidFieldType {
                        field_name: "value".to_string(),
                        context: "AT-URI validation".to_string(),
                        expected_type: "AT-URI starting with 'at://'".to_string(),
                    }
                    .into());
                }
            }
            PlaceholderType::Url => {
                if !value.starts_with("http://") && !value.starts_with("https://") {
                    return Err(ValidationError::InvalidFieldType {
                        field_name: "value".to_string(),
                        context: "URL validation".to_string(),
                        expected_type: "URL starting with 'http://' or 'https://'".to_string(),
                    }
                    .into());
                }
            }
            PlaceholderType::Collection => {
                // Basic validation for collection format
                if !value.contains('.') || value.contains('/') {
                    return Err(ValidationError::InvalidFieldType {
                        field_name: "value".to_string(),
                        context: "collection validation".to_string(),
                        expected_type: "collection name (e.g., app.bsky.feed.post)".to_string(),
                    }
                    .into());
                }
            }
            PlaceholderType::Cron => {
                // Basic cron validation - could use croner here
                let parts: Vec<&str> = value.split_whitespace().collect();
                if parts.len() < 5 && !value.starts_with('@') {
                    return Err(ValidationError::InvalidFieldType {
                        field_name: "value".to_string(),
                        context: "cron validation".to_string(),
                        expected_type: "valid cron expression".to_string(),
                    }
                    .into());
                }
            }
            PlaceholderType::Number => {
                value
                    .parse::<f64>()
                    .context("Value must be a valid number")?;
            }
            PlaceholderType::Boolean => {
                if value != "true" && value != "false" {
                    return Err(ValidationError::InvalidFieldType {
                        field_name: "value".to_string(),
                        context: "boolean validation".to_string(),
                        expected_type: "'true' or 'false'".to_string(),
                    }
                    .into());
                }
            }
            PlaceholderType::Json => {
                serde_json::from_str::<Value>(value).context("Value must be valid JSON")?;
            }
            PlaceholderType::Text => {
                // Text accepts anything
            }
        }
        Ok(())
    }
}

/// Prototype validation result.
#[derive(Debug, Clone)]
pub struct PrototypeValidation {
    /// Whether the prototype is valid
    pub is_valid: bool,

    /// Placeholders found in nodes but not defined
    pub undefined_placeholders: HashSet<String>,

    /// Defined placeholders that are never used
    pub unused_placeholders: HashSet<String>,

    /// Validation errors
    pub errors: Vec<String>,
}

impl Prototype {
    /// Extract all placeholder references from the nodes.
    ///
    /// Looks for patterns like `$[PLACEHOLDER_NAME]` in the JSON structure.
    pub fn extract_placeholder_refs(&self) -> HashSet<String> {
        let mut refs = HashSet::new();
        extract_placeholders_from_value(&self.nodes, &mut refs);
        refs
    }

    /// Validate the prototype structure.
    ///
    /// Ensures that:
    /// 1. All placeholder references in nodes are defined
    /// 2. All defined placeholders have unique IDs
    /// 3. Placeholder IDs are valid (alphanumeric + underscore)
    pub fn validate(&self) -> PrototypeValidation {
        let mut errors = Vec::new();

        // Extract all placeholder references from nodes
        let refs = self.extract_placeholder_refs();

        // Build set of defined placeholder IDs
        let mut defined: HashSet<String> = HashSet::new();
        for placeholder in &self.placeholders {
            // Validate placeholder ID format
            if !is_valid_placeholder_id(&placeholder.id) {
                errors.push(format!(
                    "Invalid placeholder ID '{}': must be alphanumeric with underscores only",
                    placeholder.id
                ));
            }

            // Check for duplicates
            if !defined.insert(placeholder.id.clone()) {
                errors.push(format!("Duplicate placeholder ID: {}", placeholder.id));
            }

            // Validate regex pattern if provided
            if let Some(pattern) = &placeholder.validation_pattern
                && Regex::new(pattern).is_err()
            {
                errors.push(format!(
                    "Invalid regex pattern for placeholder '{}': {}",
                    placeholder.id, pattern
                ));
            }
        }

        // Find undefined placeholders (referenced but not defined)
        let undefined: HashSet<String> = refs.difference(&defined).cloned().collect();
        if !undefined.is_empty() {
            errors.push(format!(
                "Undefined placeholders: {}",
                undefined.iter().cloned().collect::<Vec<_>>().join(", ")
            ));
        }

        // Find unused placeholders (defined but not referenced)
        let unused: HashSet<String> = defined.difference(&refs).cloned().collect();

        PrototypeValidation {
            is_valid: errors.is_empty() && undefined.is_empty(),
            undefined_placeholders: undefined,
            unused_placeholders: unused,
            errors,
        }
    }

    /// Instantiate a blueprint from this prototype by replacing placeholders.
    ///
    /// Takes a map of placeholder ID to value and returns the instantiated nodes.
    pub fn instantiate(&self, values: &HashMap<String, String>) -> Result<Value> {
        // Validate all required placeholders have values
        for placeholder in &self.placeholders {
            if placeholder.required {
                let value = values
                    .get(&placeholder.id)
                    .or(placeholder.default_value.as_ref())
                    .ok_or_else(|| ValidationError::MissingRequiredField {
                        field_name: placeholder.id.clone(),
                        context: "placeholder instantiation".to_string(),
                    })?;

                // Validate the value type
                placeholder.value_type.validate(value).with_context(|| {
                    format!("Invalid value for placeholder '{}'", placeholder.id)
                })?;

                // Validate against pattern if provided
                if let Some(pattern) = &placeholder.validation_pattern {
                    let re = Regex::new(pattern)
                        .with_context(|| format!("Invalid regex pattern: {}", pattern))?;
                    if !re.is_match(value) {
                        return Err(ValidationError::InvalidFieldType {
                            field_name: placeholder.id.clone(),
                            context: "placeholder pattern validation".to_string(),
                            expected_type: format!("value matching pattern: {}", pattern),
                        }
                        .into());
                    }
                }
            }
        }

        // Build complete value map including defaults
        let mut complete_values = HashMap::new();
        for placeholder in &self.placeholders {
            if let Some(value) = values.get(&placeholder.id) {
                complete_values.insert(placeholder.id.clone(), value.clone());
            } else if let Some(default) = &placeholder.default_value {
                complete_values.insert(placeholder.id.clone(), default.clone());
            }
        }

        // Perform the substitution
        let instantiated = replace_placeholders_in_value(&self.nodes, &complete_values)?;

        Ok(instantiated)
    }
}

/// Extract placeholder references from a JSON value recursively.
fn extract_placeholders_from_value(value: &Value, refs: &mut HashSet<String>) {
    match value {
        Value::String(s) => {
            // Look for $[PLACEHOLDER] pattern
            let re = Regex::new(r"\$\[([A-Z_][A-Z0-9_]*)\]").unwrap();
            for cap in re.captures_iter(s) {
                if let Some(placeholder) = cap.get(1) {
                    refs.insert(placeholder.as_str().to_string());
                }
            }
        }
        Value::Array(arr) => {
            for item in arr {
                extract_placeholders_from_value(item, refs);
            }
        }
        Value::Object(obj) => {
            for (_key, val) in obj {
                extract_placeholders_from_value(val, refs);
            }
        }
        _ => {}
    }
}

/// Replace placeholders in a JSON value recursively.
fn replace_placeholders_in_value(
    value: &Value,
    replacements: &HashMap<String, String>,
) -> Result<Value> {
    match value {
        Value::String(s) => {
            let mut result = s.clone();
            for (placeholder_id, replacement_value) in replacements {
                let pattern = format!("$[{}]", placeholder_id);
                result = result.replace(&pattern, replacement_value);
            }
            Ok(Value::String(result))
        }
        Value::Array(arr) => {
            let mut new_arr = Vec::new();
            for item in arr {
                new_arr.push(replace_placeholders_in_value(item, replacements)?);
            }
            Ok(Value::Array(new_arr))
        }
        Value::Object(obj) => {
            let mut new_obj = serde_json::Map::new();
            for (key, val) in obj {
                new_obj.insert(
                    key.clone(),
                    replace_placeholders_in_value(val, replacements)?,
                );
            }
            Ok(Value::Object(new_obj))
        }
        other => Ok(other.clone()),
    }
}

/// Check if a placeholder ID is valid (alphanumeric + underscore, starts with letter).
fn is_valid_placeholder_id(id: &str) -> bool {
    let re = Regex::new(r"^[A-Z_][A-Z0-9_]*$").unwrap();
    re.is_match(id)
}

/// Storage trait for prototypes.
#[async_trait]
pub trait PrototypeStorage: Send + Sync {
    /// Create a new prototype.
    async fn create_prototype(&self, prototype: &Prototype) -> Result<()>;

    /// Get a prototype by AT-URI.
    async fn get_prototype(&self, aturi: &str) -> Result<Option<Prototype>>;

    /// Update an existing prototype.
    async fn update_prototype(&self, prototype: &Prototype) -> Result<()>;

    /// Delete a prototype.
    async fn delete_prototype(&self, aturi: &str) -> Result<()>;

    /// List prototypes for a user.
    async fn list_prototypes(&self, did: Option<&str>) -> Result<Vec<Prototype>>;

    /// List all shared prototypes.
    async fn list_shared_prototypes(
        &self,
        limit: Option<i64>,
        offset: Option<i64>,
    ) -> Result<Vec<Prototype>>;
}

/// PostgreSQL implementation of prototype storage.
pub struct PostgresPrototypeStorage {
    pool: Arc<PgPool>,
}

impl PostgresPrototypeStorage {
    pub fn new(pool: Arc<PgPool>) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl PrototypeStorage for PostgresPrototypeStorage {
    async fn create_prototype(&self, prototype: &Prototype) -> Result<()> {
        let placeholders_json = serde_json::to_value(&prototype.placeholders)?;

        sqlx::query(
            r#"
            INSERT INTO prototypes (
                aturi, did, name, description, nodes, placeholders, shared,
                created_at, updated_at
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            "#,
        )
        .bind(&prototype.aturi)
        .bind(&prototype.did)
        .bind(&prototype.name)
        .bind(&prototype.description)
        .bind(&prototype.nodes)
        .bind(&placeholders_json)
        .bind(prototype.shared)
        .bind(prototype.created_at)
        .bind(prototype.updated_at)
        .execute(&*self.pool)
        .await
        .context("Failed to create prototype")?;

        Ok(())
    }

    async fn get_prototype(&self, aturi: &str) -> Result<Option<Prototype>> {
        let row = sqlx::query_as::<_, PrototypeRow>(
            r#"
            SELECT aturi, did, name, description, nodes, placeholders, shared,
                   created_at, updated_at
            FROM prototypes
            WHERE aturi = $1
            "#,
        )
        .bind(aturi)
        .fetch_optional(&*self.pool)
        .await
        .context("Failed to get prototype")?;

        Ok(row.map(|r| r.into_prototype()))
    }

    async fn update_prototype(&self, prototype: &Prototype) -> Result<()> {
        let placeholders_json = serde_json::to_value(&prototype.placeholders)?;

        sqlx::query(
            r#"
            UPDATE prototypes
            SET name = $2, description = $3, nodes = $4, placeholders = $5,
                shared = $6, updated_at = $7
            WHERE aturi = $1
            "#,
        )
        .bind(&prototype.aturi)
        .bind(&prototype.name)
        .bind(&prototype.description)
        .bind(&prototype.nodes)
        .bind(&placeholders_json)
        .bind(prototype.shared)
        .bind(Utc::now())
        .execute(&*self.pool)
        .await
        .context("Failed to update prototype")?;

        Ok(())
    }

    async fn delete_prototype(&self, aturi: &str) -> Result<()> {
        sqlx::query("DELETE FROM prototypes WHERE aturi = $1")
            .bind(aturi)
            .execute(&*self.pool)
            .await
            .context("Failed to delete prototype")?;

        Ok(())
    }

    async fn list_prototypes(&self, did: Option<&str>) -> Result<Vec<Prototype>> {
        let rows = if let Some(did) = did {
            sqlx::query_as::<_, PrototypeRow>(
                r#"
                SELECT aturi, did, name, description, nodes, placeholders, shared,
                       created_at, updated_at
                FROM prototypes
                WHERE did = $1
                ORDER BY created_at DESC
                "#,
            )
            .bind(did)
            .fetch_all(&*self.pool)
            .await?
        } else {
            sqlx::query_as::<_, PrototypeRow>(
                r#"
                SELECT aturi, did, name, description, nodes, placeholders, shared,
                       created_at, updated_at
                FROM prototypes
                ORDER BY created_at DESC
                "#,
            )
            .fetch_all(&*self.pool)
            .await?
        };

        Ok(rows.into_iter().map(|r| r.into_prototype()).collect())
    }

    async fn list_shared_prototypes(
        &self,
        limit: Option<i64>,
        offset: Option<i64>,
    ) -> Result<Vec<Prototype>> {
        let limit = limit.unwrap_or(20);
        let offset = offset.unwrap_or(0);

        let rows = sqlx::query_as::<_, PrototypeRow>(
            r#"
            SELECT aturi, did, name, description, nodes, placeholders, shared,
                   created_at, updated_at
            FROM prototypes
            WHERE shared = true
            ORDER BY created_at DESC
            LIMIT $1 OFFSET $2
            "#,
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(&*self.pool)
        .await
        .context("Failed to fetch shared prototypes")?;

        Ok(rows.into_iter().map(|r| r.into_prototype()).collect())
    }
}

/// Database row for prototypes.
#[derive(FromRow)]
struct PrototypeRow {
    aturi: String,
    did: String,
    name: String,
    description: String,
    nodes: Value,
    placeholders: Value,
    shared: bool,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl PrototypeRow {
    fn into_prototype(self) -> Prototype {
        Prototype {
            aturi: self.aturi,
            did: self.did,
            name: self.name,
            description: self.description,
            nodes: self.nodes,
            placeholders: serde_json::from_value(self.placeholders).unwrap_or_else(|_| Vec::new()),
            shared: self.shared,
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_extract_placeholders() {
        let nodes = json!({
            "entry": {
                "type": "jetstream_entry",
                "configuration": {
                    "did": ["$[USER_DID]"],
                    "collection": "$[COLLECTION_TYPE]"
                },
                "payload": {
                    "text": "Hello $[TARGET_USER]!",
                    "nested": {
                        "value": "$[NESTED_VALUE]"
                    }
                }
            }
        });

        let prototype = Prototype {
            aturi: "test".to_string(),
            did: "test".to_string(),
            name: "test".to_string(),
            description: "test".to_string(),
            nodes,
            placeholders: vec![],
            shared: false,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let refs = prototype.extract_placeholder_refs();
        assert_eq!(refs.len(), 4);
        assert!(refs.contains("USER_DID"));
        assert!(refs.contains("COLLECTION_TYPE"));
        assert!(refs.contains("TARGET_USER"));
        assert!(refs.contains("NESTED_VALUE"));
    }

    #[test]
    fn test_validate_prototype() {
        let nodes = json!({
            "config": {
                "did": "$[USER_DID]",
                "undefined": "$[UNDEFINED_PLACEHOLDER]"
            }
        });

        let placeholders = vec![
            PlaceholderDef {
                id: "USER_DID".to_string(),
                label: "User DID".to_string(),
                description: "Your DID".to_string(),
                value_type: PlaceholderType::Did,
                default_value: None,
                required: true,
                validation_pattern: None,
                example: Some("did:plc:example".to_string()),
            },
            PlaceholderDef {
                id: "UNUSED_PLACEHOLDER".to_string(),
                label: "Unused".to_string(),
                description: "Not used".to_string(),
                value_type: PlaceholderType::Text,
                default_value: None,
                required: false,
                validation_pattern: None,
                example: None,
            },
        ];

        let prototype = Prototype {
            aturi: "test".to_string(),
            did: "test".to_string(),
            name: "test".to_string(),
            description: "test".to_string(),
            nodes,
            placeholders,
            shared: false,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let validation = prototype.validate();
        assert!(!validation.is_valid);
        assert!(
            validation
                .undefined_placeholders
                .contains("UNDEFINED_PLACEHOLDER")
        );
        assert!(
            validation
                .unused_placeholders
                .contains("UNUSED_PLACEHOLDER")
        );
    }

    #[test]
    fn test_instantiate_prototype() {
        let nodes = json!({
            "entry": {
                "type": "jetstream_entry",
                "configuration": {
                    "did": ["$[USER_DID]"],
                    "collection": "$[COLLECTION]"
                },
                "payload": {
                    "message": "Hello from $[USER_DID]!"
                }
            }
        });

        let placeholders = vec![
            PlaceholderDef {
                id: "USER_DID".to_string(),
                label: "User DID".to_string(),
                description: "Your DID".to_string(),
                value_type: PlaceholderType::Did,
                default_value: None,
                required: true,
                validation_pattern: None,
                example: Some("did:plc:example".to_string()),
            },
            PlaceholderDef {
                id: "COLLECTION".to_string(),
                label: "Collection".to_string(),
                description: "Collection type".to_string(),
                value_type: PlaceholderType::Collection,
                default_value: Some("app.bsky.feed.post".to_string()),
                required: false,
                validation_pattern: None,
                example: None,
            },
        ];

        let prototype = Prototype {
            aturi: "test".to_string(),
            did: "test".to_string(),
            name: "test".to_string(),
            description: "test".to_string(),
            nodes,
            placeholders,
            shared: false,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let mut values = HashMap::new();
        values.insert("USER_DID".to_string(), "did:plc:testuser".to_string());

        let result = prototype.instantiate(&values).unwrap();

        let expected = json!({
            "entry": {
                "type": "jetstream_entry",
                "configuration": {
                    "did": ["did:plc:testuser"],
                    "collection": "app.bsky.feed.post"
                },
                "payload": {
                    "message": "Hello from did:plc:testuser!"
                }
            }
        });

        assert_eq!(result, expected);
    }

    #[test]
    fn test_placeholder_type_validation() {
        // Test DID validation
        assert!(PlaceholderType::Did.validate("did:plc:example").is_ok());
        assert!(PlaceholderType::Did.validate("not-a-did").is_err());

        // Test AT-URI validation
        assert!(
            PlaceholderType::AtUri
                .validate("at://did:plc:example/collection/record")
                .is_ok()
        );
        assert!(
            PlaceholderType::AtUri
                .validate("https://example.com")
                .is_err()
        );

        // Test URL validation
        assert!(PlaceholderType::Url.validate("https://example.com").is_ok());
        assert!(PlaceholderType::Url.validate("not-a-url").is_err());

        // Test Collection validation
        assert!(
            PlaceholderType::Collection
                .validate("app.bsky.feed.post")
                .is_ok()
        );
        assert!(PlaceholderType::Collection.validate("invalid").is_err());

        // Test Number validation
        assert!(PlaceholderType::Number.validate("42").is_ok());
        assert!(PlaceholderType::Number.validate("42.5").is_ok());
        assert!(PlaceholderType::Number.validate("not-a-number").is_err());

        // Test Boolean validation
        assert!(PlaceholderType::Boolean.validate("true").is_ok());
        assert!(PlaceholderType::Boolean.validate("false").is_ok());
        assert!(PlaceholderType::Boolean.validate("yes").is_err());

        // Test JSON validation
        assert!(
            PlaceholderType::Json
                .validate(r#"{"key": "value"}"#)
                .is_ok()
        );
        assert!(PlaceholderType::Json.validate("not-json").is_err());
    }
}
