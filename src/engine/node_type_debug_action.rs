//! Debug action node for logging and debugging blueprint execution.
//!
//! This module implements a debugging node that logs data as it flows through
//! the pipeline. It's essential for development, testing, and troubleshooting
//! blueprint behavior.
//!
//! # Usage
//!
//! Debug action nodes can be placed anywhere in a blueprint to inspect data
//! at that point in the pipeline. They log the data and pass it through
//! unchanged, allowing normal pipeline execution to continue.
//!
//! # Example Blueprints
//!
//! ## Debug filtered events:
//! ```json
//! {
//!   "nodes": [
//!     {
//!       "node_type": "jetstream_entry",
//!       "configuration": {"collection": ["app.bsky.feed.post"]},
//!       "payload": true
//!     },
//!     {
//!       "node_type": "condition",
//!       "payload": {
//!         "contains": [{"val": ["commit", "record", "text"]}, "debug"]
//!       }
//!     },
//!     {
//!       "node_type": "debug_action",
//!       "configuration": {},
//!       "payload": {}
//!     }
//!   ]
//! }
//! ```
//!
//! ## Debug transformation results:
//! ```json
//! {
//!   "nodes": [
//!     {
//!       "node_type": "webhook_entry",
//!       "payload": true
//!     },
//!     {
//!       "node_type": "transform",
//!       "payload": {
//!         "processed": true,
//!         "original_path": {"val": ["path"]},
//!         "timestamp": {"now": []}
//!       }
//!     },
//!     {
//!       "node_type": "debug_action",
//!       "configuration": {},
//!       "payload": {}
//!     },
//!     {
//!       "node_type": "publish_webhook",
//!       "configuration": {"url": "https://example.com/webhook"},
//!       "payload": {}
//!     }
//!   ]
//! }
//! ```
//!
//! ## Debug multiple points in pipeline:
//! ```json
//! {
//!   "nodes": [
//!     {
//!       "node_type": "jetstream_entry",
//!       "configuration": {},
//!       "payload": true
//!     },
//!     {
//!       "node_type": "debug_action",
//!       "configuration": {},
//!       "payload": {}
//!     },
//!     {
//!       "node_type": "transform",
//!       "payload": {
//!         "simplified": {"val": ["commit", "record"]}
//!       }
//!     },
//!     {
//!       "node_type": "debug_action",
//!       "configuration": {},
//!       "payload": {}
//!     }
//!   ]
//! }
//! ```

use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;

use crate::storage::node::Node;

use super::evaluator::NodeEvaluator;

/// Evaluator for debug_action nodes.
///
/// This evaluator logs the input data at the debug level using tracing.
/// It's designed for testing and debugging blueprints during development.
/// The node passes through input data unchanged, allowing it to be inserted
/// anywhere in a pipeline without disrupting data flow.
///
/// # Configuration
///
/// No configuration required. Use an empty object:
/// ```json
/// {}
/// ```
///
/// # Payload
///
/// No payload required. Use an empty object:
/// ```json
/// {}
/// ```
///
/// # Output
///
/// The debug_action node outputs the exact input it received, unchanged.
/// This allows it to be chained with other nodes for continued processing.
///
/// # Log Output
///
/// Debug logs include:
/// - Node AT-URI
/// - Blueprint AT-URI
/// - Pretty-printed JSON of the input data
///
/// # Use Cases
///
/// 1. **Development**: Inspect data structure at various pipeline stages
/// 2. **Testing**: Verify transformations are working correctly
/// 3. **Troubleshooting**: Identify where data filtering or transformation issues occur
/// 4. **Validation**: Ensure data matches expected format before actions
/// 5. **Monitoring**: Log specific events for analysis
///
/// # Tips
///
/// - Place after transforms to verify output format
/// - Place before conditions to see what data is being evaluated
/// - Place before action nodes to verify final data structure
/// - Use multiple debug nodes to trace data flow through complex pipelines
/// - Remove or comment out debug nodes in production blueprints
pub struct DebugActionEvaluator {}

impl DebugActionEvaluator {
    /// Create a new debug_action evaluator
    pub fn new() -> Self {
        Self {}
    }
}

impl Default for DebugActionEvaluator {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl NodeEvaluator for DebugActionEvaluator {
    async fn evaluate(&self, node: &Node, input: &Value) -> Result<Option<Value>> {
        tracing::info!(
            node_aturi = %node.aturi,
            blueprint = %node.blueprint,
            input = %serde_json::to_string_pretty(input).unwrap_or_else(|_| input.to_string()),
            "debug_action: Blueprint evaluation debug output"
        );

        // Return the input unchanged so it can be chained if needed
        Ok(Some(input.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use serde_json::json;

    #[tokio::test]
    async fn test_debug_action_evaluator() {
        let evaluator = DebugActionEvaluator::new();

        let node = Node {
            aturi: "at://did:example/debug.action/123".to_string(),
            blueprint: "at://did:example/blueprint/456".to_string(),
            node_type: "debug_action".to_string(),
            payload: json!({}),
            configuration: json!({}),
            created_at: Utc::now(),
        };

        let input = json!({
            "test": "data",
            "number": 42,
            "nested": {
                "field": "value"
            }
        });

        let result = evaluator.evaluate(&node, &input).await;
        assert!(result.is_ok());

        let output = result.unwrap();
        assert!(output.is_some());

        // Should return input unchanged
        assert_eq!(output.unwrap(), input);
    }
}
