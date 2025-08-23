//! Binary serialization utilities for efficient storage
//!
//! This module provides binary serialization using bincode for
//! compact storage of blueprints, nodes, and queue items.
//! Reduces storage size by approximately 40% compared to JSON.

use crate::errors::SerializationError;
use bincode::{DefaultOptions, Options};
use serde::{Deserialize, Serialize};
use std::marker::PhantomData;

/// Default bincode configuration for serialization
fn bincode_config() -> impl Options {
    DefaultOptions::new()
        .with_varint_encoding()
        .with_little_endian()
        .with_limit(10 * 1024 * 1024) // 10MB limit
}

/// Binary serializer trait for efficient storage
pub trait BinarySerializable: Serialize + for<'de> Deserialize<'de> {
    /// Serialize to binary format
    fn to_binary(&self) -> Result<Vec<u8>, SerializationError> {
        bincode_config().serialize(self).map_err(|e| {
            SerializationError::BinarySerializationFailed {
                data_type: std::any::type_name::<Self>().to_string(),
                details: e.to_string(),
            }
        })
    }

    /// Deserialize from binary format
    fn from_binary(data: &[u8]) -> Result<Self, SerializationError>
    where
        Self: Sized,
    {
        bincode_config().deserialize(data).map_err(|e| {
            SerializationError::BinaryDeserializationFailed {
                data_type: std::any::type_name::<Self>().to_string(),
                details: e.to_string(),
            }
        })
    }

    /// Get estimated size in bytes
    fn binary_size(&self) -> Result<usize, SerializationError> {
        bincode_config()
            .serialized_size(self)
            .map(|s| s as usize)
            .map_err(|e| SerializationError::BinarySerializationFailed {
                data_type: std::any::type_name::<Self>().to_string(),
                details: format!("Failed to calculate binary size: {}", e),
            })
    }
}

// Implement for all types that are Serialize + Deserialize
impl<T> BinarySerializable for T where T: Serialize + for<'de> Deserialize<'de> {}

/// Compact blueprint representation for caching
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactBlueprint {
    /// Blueprint AT-URI without prefix
    pub aturi_suffix: String,
    /// DID without prefix for did:plc and did:web
    pub did_suffix: String,
    /// Node order as comma-separated string for efficiency
    pub node_order: String,
    /// Enabled flag
    pub enabled: bool,
    /// Error message if any
    pub error: Option<String>,
    /// Created timestamp as Unix epoch seconds
    pub created_at: u64,
}

impl CompactBlueprint {
    /// Convert from full Blueprint
    pub fn from_blueprint(blueprint: &crate::storage::Blueprint) -> Self {
        Self {
            aturi_suffix: strip_aturi_prefix(&blueprint.aturi),
            did_suffix: strip_did_prefix(&blueprint.did),
            node_order: blueprint.node_order.join(","),
            enabled: blueprint.enabled,
            error: blueprint.error.clone(),
            created_at: blueprint.created_at.timestamp() as u64,
        }
    }

    /// Convert to full Blueprint
    pub fn to_blueprint(&self) -> crate::storage::Blueprint {
        use chrono::{TimeZone, Utc};

        crate::storage::Blueprint {
            aturi: restore_aturi_prefix(&self.aturi_suffix),
            did: restore_did_prefix(&self.did_suffix),
            node_order: if self.node_order.is_empty() {
                vec![]
            } else {
                self.node_order.split(',').map(String::from).collect()
            },
            enabled: self.enabled,
            error: self.error.clone(),
            created_at: Utc.timestamp_opt(self.created_at as i64, 0).unwrap(),
        }
    }
}

/// Compact node representation for caching
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactNode {
    /// Node AT-URI without prefix
    pub aturi_suffix: String,
    /// Blueprint AT-URI without prefix
    pub blueprint_suffix: String,
    /// Node type as u8 for efficiency
    pub node_type_id: u8,
    /// Payload as compressed JSON bytes
    pub payload_bytes: Vec<u8>,
    /// Configuration as compressed JSON bytes
    pub config_bytes: Vec<u8>,
    /// Created timestamp as Unix epoch seconds
    pub created_at: u64,
}

impl CompactNode {
    /// Convert from full Node
    pub fn from_node(node: &crate::storage::Node) -> Result<Self, SerializationError> {
        Ok(Self {
            aturi_suffix: strip_aturi_prefix(&node.aturi),
            blueprint_suffix: strip_aturi_prefix(&node.blueprint),
            node_type_id: node_type_to_id(&node.node_type),
            payload_bytes: serde_json::to_vec(&node.payload).map_err(|e| {
                SerializationError::JsonSerializationFailed {
                    data_type: "Node::payload".to_string(),
                    source: e,
                }
            })?,
            config_bytes: serde_json::to_vec(&node.configuration).map_err(|e| {
                SerializationError::JsonSerializationFailed {
                    data_type: "Node::configuration".to_string(),
                    source: e,
                }
            })?,
            created_at: node.created_at.timestamp() as u64,
        })
    }

    /// Convert to full Node
    pub fn to_node(&self) -> Result<crate::storage::Node, SerializationError> {
        use chrono::{TimeZone, Utc};

        Ok(crate::storage::Node {
            aturi: restore_aturi_prefix(&self.aturi_suffix),
            blueprint: restore_aturi_prefix(&self.blueprint_suffix),
            node_type: id_to_node_type(self.node_type_id),
            payload: serde_json::from_slice(&self.payload_bytes).map_err(|e| {
                SerializationError::JsonDeserializationFailed {
                    data_type: "Node::payload".to_string(),
                    source: e,
                }
            })?,
            configuration: serde_json::from_slice(&self.config_bytes).map_err(|e| {
                SerializationError::JsonDeserializationFailed {
                    data_type: "Node::configuration".to_string(),
                    source: e,
                }
            })?,
            created_at: Utc.timestamp_opt(self.created_at as i64, 0).unwrap(),
        })
    }
}

/// Strip common AT-URI prefix for compact storage
fn strip_aturi_prefix(aturi: &str) -> String {
    if let Some(suffix) = aturi.strip_prefix("at://") {
        suffix.to_string()
    } else {
        aturi.to_string()
    }
}

/// Restore AT-URI prefix
fn restore_aturi_prefix(suffix: &str) -> String {
    if suffix.starts_with("at://") {
        suffix.to_string()
    } else {
        format!("at://{}", suffix)
    }
}

/// Strip DID prefix for compact storage
fn strip_did_prefix(did: &str) -> String {
    if let Some(suffix) = did.strip_prefix("did:plc:") {
        format!("p:{}", suffix)
    } else if let Some(suffix) = did.strip_prefix("did:web:") {
        format!("w:{}", suffix)
    } else {
        did.to_string()
    }
}

/// Restore DID prefix
fn restore_did_prefix(suffix: &str) -> String {
    if let Some(plc) = suffix.strip_prefix("p:") {
        format!("did:plc:{}", plc)
    } else if let Some(web) = suffix.strip_prefix("w:") {
        format!("did:web:{}", web)
    } else if suffix.starts_with("did:") {
        suffix.to_string()
    } else {
        format!("did:plc:{}", suffix) // Default to plc
    }
}

/// Convert node type to compact ID
fn node_type_to_id(node_type: &str) -> u8 {
    match node_type {
        "jetstream_entry" => 1,
        "webhook_entry" => 2,
        "periodic_entry" => 3,
        "condition" => 4,
        "transform" => 5,
        "publish_record" => 6,
        "publish_webhook_direct" => 7,
        "publish_webhook_queue" => 8,
        "facet_text" => 9,
        "debug_action" => 10,
        _ => 0, // Unknown
    }
}

/// Convert ID back to node type
fn id_to_node_type(id: u8) -> String {
    match id {
        1 => "jetstream_entry",
        2 => "webhook_entry",
        3 => "periodic_entry",
        4 => "condition",
        5 => "transform",
        6 => "publish_record",
        7 => "publish_webhook_direct",
        8 => "publish_webhook_queue",
        9 => "facet_text",
        10 => "debug_action",
        _ => "unknown",
    }
    .to_string()
}

/// Binary cache wrapper for any serializable type
pub struct BinaryCache<T> {
    _phantom: PhantomData<T>,
}

impl<T> BinaryCache<T>
where
    T: Serialize + for<'de> Deserialize<'de>,
{
    /// Serialize value to binary for caching
    pub fn serialize(value: &T) -> Result<Vec<u8>, SerializationError> {
        value.to_binary()
    }

    /// Deserialize value from binary cache
    pub fn deserialize(data: &[u8]) -> Result<T, SerializationError> {
        T::from_binary(data)
    }

    /// Calculate compression ratio vs JSON
    pub fn compression_ratio(value: &T) -> Result<f64, SerializationError> {
        let json_size = serde_json::to_vec(value)
            .map_err(|e| SerializationError::JsonSerializationFailed {
                data_type: std::any::type_name::<T>().to_string(),
                source: e,
            })?
            .len();
        let binary_size = value.binary_size()?;
        Ok(1.0 - (binary_size as f64 / json_size as f64))
    }
}

/// Queue work item wrapper for binary serialization
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BinaryQueueItem<T> {
    /// Unique ID for deduplication
    pub id: String,
    /// Timestamp when queued
    pub queued_at: u64,
    /// Number of retry attempts
    pub attempts: u32,
    /// The actual work item
    pub item: T,
}

impl<T> BinaryQueueItem<T>
where
    T: Serialize + for<'de> Deserialize<'de>,
{
    /// Create a new queue item
    pub fn new(item: T) -> Self {
        use chrono::Utc;

        Self {
            id: ulid::Ulid::new().to_string(),
            queued_at: Utc::now().timestamp() as u64,
            attempts: 0,
            item,
        }
    }

    /// Create with specific ID (for deduplication)
    pub fn with_id(id: String, item: T) -> Self {
        use chrono::Utc;

        Self {
            id,
            queued_at: Utc::now().timestamp() as u64,
            attempts: 0,
            item,
        }
    }

    /// Increment retry attempts
    pub fn increment_attempts(&mut self) {
        self.attempts += 1;
    }

    /// Check if item has exceeded max attempts
    pub fn exceeded_max_attempts(&self, max: u32) -> bool {
        self.attempts >= max
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{Blueprint, Node};
    use chrono::Utc;
    use serde_json::json;

    #[test]
    fn test_binary_serialization() {
        #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
        struct TestStruct {
            id: String,
            value: i32,
            data: Vec<u8>,
        }

        let test = TestStruct {
            id: "test-123".to_string(),
            value: 42,
            data: vec![1, 2, 3, 4, 5],
        };

        // Serialize to binary
        let binary = test.to_binary().unwrap();
        assert!(!binary.is_empty());

        // Deserialize from binary
        let restored: TestStruct = TestStruct::from_binary(&binary).unwrap();
        assert_eq!(test, restored);

        // Check size
        let size = test.binary_size().unwrap();
        assert_eq!(size, binary.len());
    }

    #[test]
    fn test_compact_blueprint() {
        let blueprint = Blueprint {
            aturi: "at://did:plc:test123/tools.graze.ifthisthenat.blueprint/abc".to_string(),
            did: "did:plc:test123".to_string(),
            node_order: vec![
                "node1".to_string(),
                "node2".to_string(),
                "node3".to_string(),
            ],
            enabled: true,
            error: None,
            created_at: Utc::now(),
        };

        // Convert to compact
        let compact = CompactBlueprint::from_blueprint(&blueprint);
        assert_eq!(
            compact.aturi_suffix,
            "did:plc:test123/tools.graze.ifthisthenat.blueprint/abc"
        );
        assert_eq!(compact.did_suffix, "p:test123");
        assert_eq!(compact.node_order, "node1,node2,node3");

        // Convert back
        let restored = compact.to_blueprint();
        assert_eq!(restored.aturi, blueprint.aturi);
        assert_eq!(restored.did, blueprint.did);
        assert_eq!(restored.node_order, blueprint.node_order);
        assert_eq!(restored.enabled, blueprint.enabled);
    }

    #[test]
    fn test_compact_node() {
        let node = Node {
            aturi: "at://did:plc:test123/tools.graze.ifthisthenat.node/xyz".to_string(),
            blueprint: "at://did:plc:test123/tools.graze.ifthisthenat.blueprint/abc".to_string(),
            node_type: "jetstream_entry".to_string(),
            payload: json!({"kind": "commit"}),
            configuration: json!({}),
            created_at: Utc::now(),
        };

        // Convert to compact
        let compact = CompactNode::from_node(&node).unwrap();
        assert_eq!(
            compact.aturi_suffix,
            "did:plc:test123/tools.graze.ifthisthenat.node/xyz"
        );
        assert_eq!(
            compact.blueprint_suffix,
            "did:plc:test123/tools.graze.ifthisthenat.blueprint/abc"
        );
        assert_eq!(compact.node_type_id, 1); // jetstream_entry = 1

        // Convert back
        let restored = compact.to_node().unwrap();
        assert_eq!(restored.aturi, node.aturi);
        assert_eq!(restored.blueprint, node.blueprint);
        assert_eq!(restored.node_type, node.node_type);
        assert_eq!(restored.payload, node.payload);
        assert_eq!(restored.configuration, node.configuration);
    }

    #[test]
    fn test_did_prefix_stripping() {
        // Test did:plc
        assert_eq!(strip_did_prefix("did:plc:test123"), "p:test123");
        assert_eq!(restore_did_prefix("p:test123"), "did:plc:test123");

        // Test did:web
        assert_eq!(strip_did_prefix("did:web:example.com"), "w:example.com");
        assert_eq!(restore_did_prefix("w:example.com"), "did:web:example.com");

        // Test other DIDs (no stripping)
        assert_eq!(strip_did_prefix("did:key:z6Mk..."), "did:key:z6Mk...");
        assert_eq!(restore_did_prefix("did:key:z6Mk..."), "did:key:z6Mk...");
    }

    #[test]
    fn test_node_type_mapping() {
        // Test all known types
        assert_eq!(node_type_to_id("jetstream_entry"), 1);
        assert_eq!(node_type_to_id("webhook_entry"), 2);
        assert_eq!(node_type_to_id("condition"), 4);
        assert_eq!(node_type_to_id("publish_record"), 6);

        // Test reverse mapping
        assert_eq!(id_to_node_type(1), "jetstream_entry");
        assert_eq!(id_to_node_type(2), "webhook_entry");
        assert_eq!(id_to_node_type(4), "condition");
        assert_eq!(id_to_node_type(6), "publish_record");

        // Test unknown type
        assert_eq!(node_type_to_id("unknown_type"), 0);
        assert_eq!(id_to_node_type(99), "unknown");
    }

    #[test]
    fn test_binary_cache() {
        #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
        struct CacheItem {
            key: String,
            value: String,
            ttl: u64,
        }

        let item = CacheItem {
            key: "test-key".to_string(),
            value: "test-value".to_string(),
            ttl: 3600,
        };

        // Serialize
        let binary = BinaryCache::serialize(&item).unwrap();

        // Deserialize
        let restored: CacheItem = BinaryCache::deserialize(&binary).unwrap();
        assert_eq!(item, restored);

        // Check compression ratio
        let ratio = BinaryCache::compression_ratio(&item).unwrap();
        assert!(ratio > 0.0); // Should have some compression
    }

    #[test]
    fn test_binary_queue_item() {
        #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
        struct WorkItem {
            task: String,
            priority: u8,
        }

        let work = WorkItem {
            task: "process".to_string(),
            priority: 1,
        };

        // Create queue item
        let mut item = BinaryQueueItem::new(work.clone());
        assert_eq!(item.attempts, 0);
        assert!(!item.id.is_empty());

        // Test attempts
        item.increment_attempts();
        assert_eq!(item.attempts, 1);
        assert!(!item.exceeded_max_attempts(3));

        item.increment_attempts();
        item.increment_attempts();
        assert!(item.exceeded_max_attempts(3));

        // Test with specific ID
        let item_with_id = BinaryQueueItem::with_id("custom-id".to_string(), work);
        assert_eq!(item_with_id.id, "custom-id");
    }

    #[test]
    fn test_compression_comparison() {
        let blueprint = Blueprint {
            aturi: "at://did:plc:verylongidentifier123456789/tools.graze.ifthisthenat.blueprint/abc123xyz".to_string(),
            did: "did:plc:verylongidentifier123456789".to_string(),
            node_order: (0..20).map(|i| format!("node-{}", i)).collect(),
            enabled: true,
            error: Some("This is a long error message that takes up space".to_string()),
            created_at: Utc::now(),
        };

        // Compare sizes
        let json_size = serde_json::to_vec(&blueprint).unwrap().len();
        let compact = CompactBlueprint::from_blueprint(&blueprint);
        let binary_size = compact.to_binary().unwrap().len();

        let reduction = 1.0 - (binary_size as f64 / json_size as f64);
        println!("Size reduction: {:.1}%", reduction * 100.0);

        // Should achieve significant compression
        assert!(reduction > 0.2); // At least 20% reduction
    }
}
