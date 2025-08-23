//! Core trait and implementations for composable blueprint evaluation.
//!
//! This module provides a composable architecture for blueprint evaluation using
//! the decorator pattern. Different concerns (caching, rate limiting, metrics, etc.)
//! are implemented as wrappers around a base evaluator, allowing them to be
//! composed together in a flexible way.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use std::time::{Duration, Instant};
use thiserror::Error;
use tracing::{debug, error, info, trace, warn};

use crate::storage::{Blueprint, Node};

/// Errors that can occur during blueprint evaluation
#[derive(Error, Debug)]
pub enum BlueprintEvaluatorError {
    #[error("error-iftta-evaluator-1 Blueprint not found: {0}")]
    BlueprintNotFound(String),

    #[error("error-iftta-evaluator-2 Node not found: {0}")]
    NodeNotFound(String),

    #[error("error-iftta-evaluator-3 Evaluation failed: {0}")]
    EvaluationFailed(String),

    #[error("error-iftta-evaluator-4 Rate limit exceeded")]
    RateLimitExceeded,

    #[error("error-iftta-evaluator-5 Cache operation failed: {0}")]
    CacheFailed(String),

    #[error("error-iftta-evaluator-6 Storage error: {0}")]
    StorageError(String),

    #[error("error-iftta-evaluator-7 Invalid blueprint state: {0}")]
    InvalidState(String),

    #[error("error-iftta-evaluator-8 Timeout: {0}ms")]
    Timeout(u64),
}

/// Result of blueprint evaluation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvaluationResult {
    /// The blueprint that was evaluated
    pub blueprint_uri: String,
    
    /// Whether the evaluation succeeded
    pub success: bool,
    
    /// The final output value (if any)
    pub output: Option<Value>,
    
    /// Number of nodes evaluated
    pub nodes_evaluated: usize,
    
    /// Total evaluation time in milliseconds
    pub duration_ms: u64,
    
    /// Individual node results (for debugging)
    pub node_results: Vec<NodeResult>,
    
    /// Any error that occurred
    pub error: Option<String>,
    
    /// Whether this result was served from cache
    pub from_cache: bool,
    
    /// Cache TTL remaining (if cached)
    pub cache_ttl_remaining: Option<u64>,
}

/// Result of evaluating a single node
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeResult {
    /// The node URI
    pub node_uri: String,
    
    /// The node type
    pub node_type: String,
    
    /// Whether the node evaluation succeeded
    pub success: bool,
    
    /// The output value (if any)
    pub output: Option<Value>,
    
    /// Evaluation time in milliseconds
    pub duration_ms: u64,
    
    /// Any error that occurred
    pub error: Option<String>,
}

/// Context for blueprint evaluation
#[derive(Debug, Clone)]
pub struct EvaluationContext {
    /// The blueprint to evaluate
    pub blueprint: Blueprint,
    
    /// The nodes in the blueprint
    pub nodes: Vec<Node>,
    
    /// Initial input data
    pub input: Value,
    
    /// Optional timeout for evaluation
    pub timeout: Option<Duration>,
    
    /// Tags for metrics
    pub tags: Vec<(String, String)>,
}

/// Core trait for blueprint evaluation
///
/// This trait defines the interface for evaluating blueprints. Implementations
/// can be composed together using the decorator pattern to add functionality
/// like caching, rate limiting, metrics, etc.
#[async_trait]
pub trait BlueprintEvaluator: Send + Sync {
    /// Evaluate a blueprint with the given context
    ///
    /// # Arguments
    /// * `context` - The evaluation context containing the blueprint, nodes, and input
    ///
    /// # Returns
    /// * `Ok(result)` - The evaluation result
    /// * `Err(error)` - An error occurred during evaluation
    async fn evaluate(&self, context: &EvaluationContext) -> Result<EvaluationResult, BlueprintEvaluatorError>;

    /// Check if the evaluator is healthy
    ///
    /// This can be used for health checks and monitoring.
    async fn is_healthy(&self) -> bool {
        true
    }

    /// Get metrics about the evaluator
    ///
    /// Returns key-value pairs of metrics that can be reported.
    async fn get_metrics(&self) -> Vec<(String, f64)> {
        vec![]
    }

    /// Invalidate any caches for a specific blueprint
    ///
    /// This is a no-op for evaluators that don't cache.
    async fn invalidate_cache(&self, _blueprint_uri: &str) -> Result<(), BlueprintEvaluatorError> {
        Ok(())
    }
}

/// Extension trait for BlueprintEvaluator with helper methods
#[async_trait]
pub trait BlueprintEvaluatorExt: BlueprintEvaluator {
    /// Evaluate a blueprint by URI
    ///
    /// This is a convenience method that loads the blueprint and nodes
    /// before calling the main evaluate method.
    async fn evaluate_by_uri(
        &self,
        blueprint_uri: &str,
        input: Value,
        blueprint_storage: &dyn crate::storage::BlueprintStorage,
        node_storage: &dyn crate::storage::NodeStorage,
    ) -> Result<EvaluationResult, BlueprintEvaluatorError> {
        // Load blueprint
        let blueprint = blueprint_storage
            .get_blueprint(blueprint_uri)
            .await
            .map_err(|e| BlueprintEvaluatorError::StorageError(e.to_string()))?
            .ok_or_else(|| BlueprintEvaluatorError::BlueprintNotFound(blueprint_uri.to_string()))?;

        // Load nodes
        let nodes = node_storage
            .list_nodes(blueprint_uri)
            .await
            .map_err(|e| BlueprintEvaluatorError::StorageError(e.to_string()))?;

        // Create context
        let context = EvaluationContext {
            blueprint,
            nodes,
            input,
            timeout: None,
            tags: vec![],
        };

        // Evaluate
        self.evaluate(&context).await
    }

    /// Evaluate with timeout
    async fn evaluate_with_timeout(
        &self,
        context: &EvaluationContext,
        timeout: Duration,
    ) -> Result<EvaluationResult, BlueprintEvaluatorError> {
        let mut context = context.clone();
        context.timeout = Some(timeout);
        
        match tokio::time::timeout(timeout, self.evaluate(&context)).await {
            Ok(result) => result,
            Err(_) => Err(BlueprintEvaluatorError::Timeout(timeout.as_millis() as u64)),
        }
    }

    /// Evaluate with metrics tags
    async fn evaluate_with_tags(
        &self,
        context: &EvaluationContext,
        tags: Vec<(String, String)>,
    ) -> Result<EvaluationResult, BlueprintEvaluatorError> {
        let mut context = context.clone();
        context.tags = tags;
        self.evaluate(&context).await
    }
}

// Implement the extension trait for all BlueprintEvaluator implementations
impl<T: BlueprintEvaluator + ?Sized> BlueprintEvaluatorExt for T {}

// Allow Arc<dyn BlueprintEvaluator> to be used as BlueprintEvaluator
#[async_trait]
impl BlueprintEvaluator for Arc<dyn BlueprintEvaluator> {
    async fn evaluate(&self, context: &EvaluationContext) -> Result<EvaluationResult, BlueprintEvaluatorError> {
        self.as_ref().evaluate(context).await
    }

    async fn is_healthy(&self) -> bool {
        self.as_ref().is_healthy().await
    }

    async fn get_metrics(&self) -> Vec<(String, f64)> {
        self.as_ref().get_metrics().await
    }

    async fn invalidate_cache(&self, blueprint_uri: &str) -> Result<(), BlueprintEvaluatorError> {
        self.as_ref().invalidate_cache(blueprint_uri).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Mock evaluator for testing
    struct MockEvaluator {
        should_succeed: bool,
        output: Value,
    }

    #[async_trait]
    impl BlueprintEvaluator for MockEvaluator {
        async fn evaluate(&self, context: &EvaluationContext) -> Result<EvaluationResult, BlueprintEvaluatorError> {
            if self.should_succeed {
                Ok(EvaluationResult {
                    blueprint_uri: context.blueprint.aturi.clone(),
                    success: true,
                    output: Some(self.output.clone()),
                    nodes_evaluated: context.nodes.len(),
                    duration_ms: 100,
                    node_results: vec![],
                    error: None,
                    from_cache: false,
                    cache_ttl_remaining: None,
                })
            } else {
                Err(BlueprintEvaluatorError::EvaluationFailed("Mock failure".to_string()))
            }
        }
    }

    #[tokio::test]
    async fn test_mock_evaluator_success() {
        let evaluator = MockEvaluator {
            should_succeed: true,
            output: json!({"test": "value"}),
        };

        let context = EvaluationContext {
            blueprint: crate::storage::Blueprint {
                aturi: "at://test/blueprint".to_string(),
                did: "did:test".to_string(),
                node_order: vec![],
                enabled: true,
                error: None,
                created_at: chrono::Utc::now(),
            },
            nodes: vec![],
            input: json!({}),
            timeout: None,
            tags: vec![],
        };

        let result = evaluator.evaluate(&context).await.unwrap();
        assert!(result.success);
        assert_eq!(result.output, Some(json!({"test": "value"})));
    }

    #[tokio::test]
    async fn test_mock_evaluator_failure() {
        let evaluator = MockEvaluator {
            should_succeed: false,
            output: json!({}),
        };

        let context = EvaluationContext {
            blueprint: crate::storage::Blueprint {
                aturi: "at://test/blueprint".to_string(),
                did: "did:test".to_string(),
                node_order: vec![],
                enabled: true,
                error: None,
                created_at: chrono::Utc::now(),
            },
            nodes: vec![],
            input: json!({}),
            timeout: None,
            tags: vec![],
        };

        let result = evaluator.evaluate(&context).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_arc_evaluator() {
        let evaluator: Arc<dyn BlueprintEvaluator> = Arc::new(MockEvaluator {
            should_succeed: true,
            output: json!({"arc": "test"}),
        });

        let context = EvaluationContext {
            blueprint: crate::storage::Blueprint {
                aturi: "at://test/blueprint".to_string(),
                did: "did:test".to_string(),
                node_order: vec![],
                enabled: true,
                error: None,
                created_at: chrono::Utc::now(),
            },
            nodes: vec![],
            input: json!({}),
            timeout: None,
            tags: vec![],
        };

        let result = evaluator.evaluate(&context).await.unwrap();
        assert!(result.success);
        assert_eq!(result.output, Some(json!({"arc": "test"})));
    }
}