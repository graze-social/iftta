//! Factory pattern implementation for managing node evaluators.
//!
//! The `NodeEvaluatorFactory` provides a centralized registry for all node type
//! evaluators in the system. It uses a type-based dispatch pattern to route
//! evaluation requests to the appropriate evaluator implementation.
//!
//! # Design Pattern
//!
//! This module implements the Factory and Registry patterns, allowing dynamic
//! registration of evaluators and runtime dispatch based on node types.
//!
//! # Usage Example
//!
//! ```rust
//! use std::sync::Arc;
//! use ifthisthenat::engine::node_evaluator_factory::NodeEvaluatorFactory;
//! use ifthisthenat::engine::node_type_condition::ConditionEvaluator;
//! use ifthisthenat::engine::node_type_transform::TransformEvaluator;
//! use ifthisthenat::engine::node_type_debug_action::DebugActionEvaluator;
//!
//! // Create and configure the factory
//! let factory = Arc::new(
//!     NodeEvaluatorFactory::new()
//!         .register("condition", Arc::new(ConditionEvaluator::new()))
//!         .register("transform", Arc::new(TransformEvaluator::new()))
//!         .register("debug_action", Arc::new(DebugActionEvaluator::new()))
//! );
//!
//! // The factory can now evaluate any registered node type
//! // based on the node's type field
//! ```

use anyhow::Result;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

use super::evaluator::NodeEvaluator;
use crate::errors::EngineError;
use crate::storage::node::Node;

/// Factory for managing and dispatching to different node evaluators.
///
/// The factory maintains a registry of node type evaluators and provides
/// type-based dispatch for node evaluation. This allows the blueprint engine
/// to support multiple node types without hard-coding the evaluation logic.
///
/// # Thread Safety
///
/// The factory is designed to be shared across threads using `Arc`. All
/// registered evaluators must also be thread-safe (`Send + Sync`).
///
/// # Supported Node Types
///
/// The factory supports any node type for which an evaluator has been registered:
/// - `jetstream_entry`: Filters Jetstream events
/// - `webhook_entry`: Filters webhook requests
/// - `condition`: Conditional flow control
/// - `transform`: Data transformation
/// - `facet_text`: Text facet extraction for AT Protocol
/// - `publish_record`: AT Protocol record creation
/// - `publish_webhook`: Webhook publishing
/// - `debug_action`: Debug logging
pub struct NodeEvaluatorFactory {
    evaluators: HashMap<String, Arc<dyn NodeEvaluator>>,
}

impl NodeEvaluatorFactory {
    /// Creates a new factory with no evaluators registered.
    ///
    /// # Example
    ///
    /// ```rust
    /// use ifthisthenat::engine::node_evaluator_factory::NodeEvaluatorFactory;
    ///
    /// let factory = NodeEvaluatorFactory::new();
    /// assert!(!factory.supports("condition")); // No evaluators registered yet
    /// ```
    pub fn new() -> Self {
        Self {
            evaluators: HashMap::new(),
        }
    }

    /// Registers an evaluator for a specific node type.
    ///
    /// This method uses the builder pattern, allowing chained registration
    /// of multiple evaluators.
    ///
    /// # Arguments
    ///
    /// * `node_type` - The string identifier for the node type (e.g., "condition")
    /// * `evaluator` - The evaluator implementation for this node type
    ///
    /// # Returns
    ///
    /// Returns `self` to allow method chaining.
    ///
    /// # Example
    ///
    /// ```rust
    /// use std::sync::Arc;
    /// use ifthisthenat::engine::node_evaluator_factory::NodeEvaluatorFactory;
    /// use ifthisthenat::engine::node_type_condition::ConditionEvaluator;
    ///
    /// let factory = NodeEvaluatorFactory::new()
    ///     .register("condition", Arc::new(ConditionEvaluator::new()));
    ///
    /// assert!(factory.supports("condition"));
    /// ```
    pub fn register(mut self, node_type: &str, evaluator: Arc<dyn NodeEvaluator>) -> Self {
        self.evaluators.insert(node_type.to_string(), evaluator);
        self
    }

    /// Evaluates a node using the appropriate registered evaluator.
    ///
    /// This method looks up the evaluator based on the node's type and
    /// delegates the evaluation to it. If no evaluator is registered for
    /// the node type, an error is returned.
    ///
    /// # Arguments
    ///
    /// * `node` - The node to evaluate
    /// * `input` - The input data for evaluation
    ///
    /// # Returns
    ///
    /// * `Ok(Some(value))` - Evaluation succeeded with output
    /// * `Ok(None)` - Evaluation succeeded but filtered out the data
    /// * `Err` - No evaluator registered or evaluation failed
    ///
    /// # Example
    ///
    /// ```rust
    /// # async fn example() -> anyhow::Result<()> {
    /// use std::sync::Arc;
    /// use serde_json::json;
    /// use ifthisthenat::engine::node_evaluator_factory::NodeEvaluatorFactory;
    /// use ifthisthenat::engine::node_type_transform::TransformEvaluator;
    /// use ifthisthenat::storage::node::Node;
    ///
    /// let factory = Arc::new(
    ///     NodeEvaluatorFactory::new()
    ///         .register("transform", Arc::new(TransformEvaluator::new()))
    /// );
    ///
    /// let node = Node {
    ///     aturi: "test".to_string(),
    ///     blueprint: "test".to_string(),
    ///     node_type: "transform".to_string(),
    ///     payload: json!({"output": {"result": true}}),
    ///     configuration: json!({}),
    ///     created_at: chrono::Utc::now(),
    /// };
    ///
    /// let input = json!({"data": "test"});
    /// let result = factory.evaluate(&node, &input).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn evaluate(&self, node: &Node, input: &Value) -> Result<Option<Value>> {
        let evaluator = self
            .evaluators
            .get(&node.node_type)
            .ok_or_else(|| EngineError::NodeFactoryFailed {
                node_type: node.node_type.clone(),
                details: "No evaluator registered for node type".to_string(),
            })?;

        evaluator.evaluate(node, input).await
    }

    /// Checks if a node type is supported by the factory.
    ///
    /// # Arguments
    ///
    /// * `node_type` - The node type identifier to check
    ///
    /// # Returns
    ///
    /// `true` if an evaluator is registered for this node type, `false` otherwise.
    ///
    /// # Example
    ///
    /// ```rust
    /// use std::sync::Arc;
    /// use ifthisthenat::engine::node_evaluator_factory::NodeEvaluatorFactory;
    /// use ifthisthenat::engine::node_type_condition::ConditionEvaluator;
    ///
    /// let factory = NodeEvaluatorFactory::new()
    ///     .register("condition", Arc::new(ConditionEvaluator::new()));
    ///
    /// assert!(factory.supports("condition"));
    /// assert!(!factory.supports("unknown_type"));
    /// ```
    pub fn supports(&self, node_type: &str) -> bool {
        self.evaluators.contains_key(node_type)
    }
}

impl Default for NodeEvaluatorFactory {
    fn default() -> Self {
        Self::new()
    }
}
