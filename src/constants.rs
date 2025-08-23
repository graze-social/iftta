//! Application-wide constants

/// Valid node types for blueprints
pub(crate) const NODE_TYPE_JETSTREAM_ENTRY: &str = "jetstream_entry";
pub(crate) const NODE_TYPE_WEBHOOK_ENTRY: &str = "webhook_entry";
pub(crate) const NODE_TYPE_PERIODIC_ENTRY: &str = "periodic_entry";
pub(crate) const NODE_TYPE_ZAP_ENTRY: &str = "zap_entry";
pub(crate) const NODE_TYPE_CONDITION: &str = "condition";
pub(crate) const NODE_TYPE_TRANSFORM: &str = "transform";
pub(crate) const NODE_TYPE_PUBLISH_RECORD: &str = "publish_record";
pub(crate) const NODE_TYPE_PUBLISH_WEBHOOK: &str = "publish_webhook";
pub(crate) const NODE_TYPE_DEBUG_ACTION: &str = "debug_action";
pub(crate) const NODE_TYPE_FACET_TEXT: &str = "facet_text";
pub(crate) const NODE_TYPE_PARSE_ATURI: &str = "parse_aturi";
pub(crate) const NODE_TYPE_GET_RECORD: &str = "get_record";
pub(crate) const NODE_TYPE_SENTIMENT_ANALYSIS: &str = "sentiment_analysis";

/// All valid node types
pub(crate) const VALID_NODE_TYPES: &[&str] = &[
    NODE_TYPE_JETSTREAM_ENTRY,
    NODE_TYPE_WEBHOOK_ENTRY,
    NODE_TYPE_PERIODIC_ENTRY,
    NODE_TYPE_ZAP_ENTRY,
    NODE_TYPE_CONDITION,
    NODE_TYPE_TRANSFORM,
    NODE_TYPE_FACET_TEXT,
    NODE_TYPE_PARSE_ATURI,
    NODE_TYPE_GET_RECORD,
    NODE_TYPE_SENTIMENT_ANALYSIS,
    NODE_TYPE_PUBLISH_RECORD,
    NODE_TYPE_PUBLISH_WEBHOOK,
    NODE_TYPE_DEBUG_ACTION,
];

/// Entry node types (must be first node in blueprint)
pub(crate) const ENTRY_NODE_TYPES: &[&str] = &[
    NODE_TYPE_JETSTREAM_ENTRY,
    NODE_TYPE_WEBHOOK_ENTRY,
    NODE_TYPE_PERIODIC_ENTRY,
    NODE_TYPE_ZAP_ENTRY,
];

/// Action node types (blueprint must have at least one)
pub(crate) const ACTION_NODE_TYPES: &[&str] = &[
    NODE_TYPE_PUBLISH_RECORD,
    NODE_TYPE_PUBLISH_WEBHOOK,
    NODE_TYPE_DEBUG_ACTION,
];

/// Check if a node type is valid
pub(crate) fn is_valid_node_type(node_type: &str) -> bool {
    VALID_NODE_TYPES.contains(&node_type)
}

/// Check if a node type is an entry node
pub(crate) fn is_entry_node_type(node_type: &str) -> bool {
    ENTRY_NODE_TYPES.contains(&node_type)
}

/// Check if a node type is an action node
pub(crate) fn is_action_node_type(node_type: &str) -> bool {
    ACTION_NODE_TYPES.contains(&node_type)
}
