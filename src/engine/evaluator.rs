//! Core trait definition for node evaluation in the blueprint engine.
//!
//! This module defines the fundamental `NodeEvaluator` trait that all node types
//! must implement. Node evaluators are responsible for processing data as it flows
//! through a blueprint, transforming, filtering, or taking action based on the
//! node's configuration and the input data.
//!
//! # Architecture
//!
//! The evaluation system follows a pipeline pattern where:
//! - Entry nodes (jetstream_entry, webhook_entry) filter incoming events
//! - Processing nodes (condition, transform, facet_text) modify or filter data
//! - Action nodes (publish_record, publish_webhook, debug_action) perform side effects
//!
//! # Example Implementation
//!
//! ```rust,ignore
//! use async_trait::async_trait;
//! use anyhow::Result;
//! use serde_json::{json, Value};
//! use ifthisthenat::engine::evaluator::NodeEvaluator;
//! use ifthisthenat::storage::node::Node;
//!
//! struct CustomEvaluator;
//!
//! #[async_trait]
//! impl NodeEvaluator for CustomEvaluator {
//!     async fn evaluate(&self, node: &Node, input: &Value) -> Result<Option<Value>> {
//!         // Process the input based on node configuration
//!         let config = &node.configuration;
//!         
//!         // Example: Extract a field specified in configuration
//!         let field = config.get("field")
//!             .and_then(|v| v.as_str())
//!             .unwrap_or("default");
//!         
//!         // Return transformed data or None to filter out
//!         Ok(Some(json!({
//!             "processed": input.get(field).cloned()
//!         })))
//!     }
//! }
//! ```

use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;

use crate::storage::node::Node;

/// Core trait for implementing node evaluation logic.
///
/// All node types in the blueprint engine must implement this trait to define
/// their behavior when processing data. The trait is async to support operations
/// that may require network calls or other I/O operations.
///
/// # Thread Safety
///
/// Implementors must be `Send + Sync` as evaluators are shared across async tasks
/// and may be called concurrently.
///
/// # Evaluation Flow
///
/// 1. Node receives input data from the previous node in the pipeline
/// 2. Node evaluates its payload (often DataLogic expressions) against the input
/// 3. Node performs its type-specific operation (filter, transform, action)
/// 4. Node returns the result for the next node or `None` to stop processing
#[async_trait]
pub trait NodeEvaluator: Send + Sync {
    /// Evaluates a node with the given input data.
    ///
    /// This method is called by the blueprint engine when processing data through
    /// a node. The implementation should use the node's configuration and payload
    /// to determine how to process the input.
    ///
    /// # Arguments
    ///
    /// * `node` - The node to evaluate, containing:
    ///   - `node_type`: The type identifier (e.g., "condition", "transform")
    ///   - `payload`: The node's logic (often DataLogic expressions)
    ///   - `configuration`: Type-specific configuration (e.g., DID, collection)
    /// * `input` - The JSON data to process, either from an event or previous node
    ///
    /// # Returns
    ///
    /// * `Ok(Some(value))` - Evaluation succeeded, pass `value` to the next node
    /// * `Ok(None)` - Evaluation succeeded but filtered out the data (stop processing)
    /// * `Err(error)` - Evaluation failed with an error
    ///
    /// # Examples
    ///
    /// ## Filtering (returning None)
    /// ```rust,ignore
    /// // A condition node might return None if the condition doesn't match
    /// if !condition_matches {
    ///     return Ok(None); // Stop processing this data
    /// }
    /// ```
    ///
    /// ## Transformation (returning modified data)
    /// ```rust,ignore
    /// // A transform node modifies and passes on the data
    /// let transformed = transform_data(input);
    /// Ok(Some(transformed))
    /// ```
    ///
    /// ## Side Effects (action nodes)
    /// ```rust,ignore
    /// // An action node performs side effects and passes through data
    /// perform_action(input).await?;
    /// Ok(Some(input.clone())) // Pass through the original data
    /// ```
    async fn evaluate(&self, node: &Node, input: &Value) -> Result<Option<Value>>;
}
