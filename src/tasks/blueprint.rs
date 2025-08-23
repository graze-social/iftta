//! Blueprint evaluation with optional tracing support.
//!
//! This module provides blueprint evaluation task that supports
//! optional tracing with span context serialization.

use serde_json::Value;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Instant;
use thiserror::Error;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{Instrument, debug, error, info, info_span, warn};

/// Blueprint evaluation errors following the error-iftta-<domain>-<number> format
#[derive(Error, Debug)]
pub enum BlueprintEvaluationError {
    #[error("error-iftta-blueprint-eval-1 Blueprint storage operation failed: {0}")]
    StorageError(#[from] crate::errors::StorageError),

    #[error("error-iftta-blueprint-eval-2 Blueprint not found: {blueprint}")]
    BlueprintNotFound { blueprint: String },

    #[error("error-iftta-blueprint-eval-3 Blueprint evaluation queue closed")]
    QueueClosed,

    #[error("error-iftta-blueprint-eval-4 Queue adapter operation failed: {0}")]
    QueueAdapterError(String),

    #[error(
        "error-iftta-blueprint-eval-5 Node evaluation failed: node_type={node_type}, index={index}, error={error}"
    )]
    NodeEvaluationFailed {
        node_type: String,
        index: usize,
        error: String,
    },

    #[error("error-iftta-blueprint-eval-6 Invalid blueprint state: {reason}")]
    InvalidBlueprintState { reason: String },
}

use crate::{
    engine::node_evaluator_factory::NodeEvaluatorFactory,
    leadership::LeadershipElection,
    queue_adapter::QueueAdapter,
    storage::{
        blueprint::BlueprintStorage,
        evaluation_result::{EvaluationResult, EvaluationResultStorage},
        node::NodeStorage,
    },
    validation::Validator,
};

/// Work item that supports both traced and non-traced modes (re-export from adapter module)
pub use crate::tasks::blueprint_adapter::BlueprintWork;

/// Blueprint evaluation task with optional tracing and leadership election.
pub struct BlueprintEvaluationTask {
    factory: Arc<NodeEvaluatorFactory>,
    blueprint_storage: Arc<dyn BlueprintStorage>,
    node_storage: Arc<dyn NodeStorage>,
    queue_adapter: Arc<dyn QueueAdapter<BlueprintWork>>,
    leadership_election: Option<Arc<dyn LeadershipElection>>,
    cancel_token: CancellationToken,
    _enable_tracing: bool,
    disabled_node_types: HashSet<String>,
    allowed_publish_collections: HashSet<String>,
    evaluation_storage: Arc<dyn EvaluationResultStorage>,
}

impl BlueprintEvaluationTask {
    /// Create with a queue adapter, optional tracing, and optional leadership election.
    pub fn with_adapter(
        factory: Arc<NodeEvaluatorFactory>,
        blueprint_storage: Arc<dyn BlueprintStorage>,
        node_storage: Arc<dyn NodeStorage>,
        queue_adapter: Arc<dyn QueueAdapter<BlueprintWork>>,
        leadership_election: Option<Arc<dyn LeadershipElection>>,
        cancel_token: CancellationToken,
        enable_tracing: bool,
        disabled_node_types: HashSet<String>,
        allowed_publish_collections: HashSet<String>,
        evaluation_storage: Arc<dyn EvaluationResultStorage>,
    ) -> Self {
        Self {
            factory,
            blueprint_storage,
            node_storage,
            queue_adapter,
            leadership_election,
            cancel_token,
            _enable_tracing: enable_tracing,
            disabled_node_types,
            allowed_publish_collections,
            evaluation_storage,
        }
    }

    /// Run the evaluation task.
    pub async fn run(self) -> Result<(), BlueprintEvaluationError> {
        info!(
            leadership_enabled = self.leadership_election.is_some(),
            "Starting blueprint evaluation task"
        );

        while !self.cancel_token.is_cancelled() {
            tokio::select! {
                Some(work) = self.queue_adapter.pull() => {
                    // Check leadership before processing work
                    if let Some(ref leadership) = self.leadership_election {
                        match leadership.is_leader().await {
                            Ok(true) => {
                                // We are the leader, process the work
                                debug!("Processing work as leader");
                            }
                            Ok(false) => {
                                // We are not the leader, skip processing but still ack
                                debug!(blueprint = %work.blueprint, "Skipping work - not leader");
                                let _ = self.queue_adapter.ack(&work).await;
                                continue;
                            }
                            Err(e) => {
                                // Leadership check failed, assume not leader and skip
                                warn!(error = ?e, "Leadership check failed, skipping work");
                                let _ = self.queue_adapter.ack(&work).await;
                                continue;
                            }
                        }
                    }
                    // If no leadership election configured, always process work

                    let work_result = self.evaluate_work(work.clone()).await;
                    // Acknowledge work after processing (ignore ack errors)
                    let _ = self.queue_adapter.ack(&work).await;
                    if let Err(e) = work_result {
                        error!(error = ?e, "Error evaluating work");
                    }
                }
                () = self.cancel_token.cancelled() => {
                    info!("Task cancelled");
                    break;
                }
            }
        }

        info!("Blueprint evaluation task stopped");
        Ok(())
    }

    /// Evaluate a single work item.
    async fn evaluate_work(&self, mut work: BlueprintWork) -> Result<(), BlueprintEvaluationError> {
        // Retrieve the blueprint from storage
        let blueprint = self
            .blueprint_storage
            .get_blueprint(&work.blueprint)
            .await?;

        let blueprint = match blueprint {
            Some(bp) => bp,
            None => {
                warn!(blueprint = %work.blueprint, "Blueprint not found");
                return Ok(());
            }
        };

        // Check if blueprint is enabled
        if !blueprint.enabled {
            debug!(blueprint = %work.blueprint, "Blueprint is disabled, skipping");
            return Ok(());
        }

        // Clear any previous error if blueprint is being re-processed
        if blueprint.error.is_some()
            && let Err(e) = self
                .blueprint_storage
                .update_blueprint_error(&work.blueprint, true, None)
                .await
        {
            warn!(error = ?e, blueprint = %work.blueprint, "Failed to clear blueprint error");
        }

        // Retrieve nodes for this blueprint
        let nodes = self.node_storage.list_nodes(&work.blueprint).await?;

        if nodes.is_empty() {
            warn!(blueprint = %work.blueprint, "No nodes found for blueprint");
            return Ok(());
        }

        // Check if blueprint contains any disabled node types
        let disabled_nodes: Vec<_> = nodes
            .iter()
            .filter(|node| self.disabled_node_types.contains(&node.node_type))
            .collect();

        if !disabled_nodes.is_empty() {
            let disabled_types: Vec<_> = disabled_nodes
                .iter()
                .map(|node| node.node_type.as_str())
                .collect();

            warn!(
                blueprint = %work.blueprint,
                disabled_node_types = ?disabled_types,
                "Blueprint contains disabled node types, skipping evaluation"
            );
            return Ok(());
        }

        // Validate publish_record collections if constraints are configured
        if let Err(e) =
            Validator::validate_publish_collections(&nodes, &self.allowed_publish_collections)
        {
            warn!(
                blueprint = %work.blueprint,
                error = ?e,
                "Blueprint disabled due to collection constraints"
            );

            // Disable the blueprint
            let error_message = format!("Blueprint disabled: {}", e);
            if let Err(storage_err) = self
                .blueprint_storage
                .update_blueprint_error(&work.blueprint, false, Some(error_message))
                .await
            {
                error!(
                    blueprint = %work.blueprint,
                    error = ?storage_err,
                    "Failed to update blueprint error status"
                );
            }
            return Ok(());
        }

        // Evaluate node at the current index
        if work.node_index >= nodes.len() {
            debug!(
                blueprint = %work.blueprint,
                node_index = work.node_index,
                nodes_count = nodes.len(),
                "Node index out of bounds"
            );
            return Ok(());
        }

        let node = &nodes[work.node_index];

        // Create tracing span if enabled
        let span = Some(info_span!("evaluate_node",
            blueprint = %work.blueprint,
            node.type = %node.node_type,
            node.index = work.node_index,
            trace_id = ?work.trace_id,
        ));

        // Evaluate the node
        let start = Instant::now();

        let result = if let Some(span) = span {
            self.factory
                .evaluate(node, &work.payload)
                .instrument(span)
                .await
        } else {
            self.factory.evaluate(node, &work.payload).await
        };

        let duration = start.elapsed();
        work.nodes_evaluated += 1;

        match result {
            Ok(Some(output)) => {
                debug!(
                    blueprint = %work.blueprint,
                    node.type = %node.node_type,
                    node.index = work.node_index,
                    duration_ms = duration.as_millis(),
                    nodes_evaluated = work.nodes_evaluated,
                    "Node evaluation successful"
                );

                // Schedule next node if not at end
                if work.node_index + 1 < nodes.len() {
                    let next_work = BlueprintWork {
                        blueprint: work.blueprint.clone(),
                        node_index: work.node_index + 1,
                        payload: output,
                        trace_id: work.trace_id.clone(),
                        blueprint_start: work.blueprint_start,
                        nodes_evaluated: work.nodes_evaluated,
                    };

                    // Use try_push to avoid potential deadlock
                    match self.queue_adapter.try_push(next_work.clone()).await {
                        Ok(()) => {}
                        Err(e) if e.to_string().contains("full") => {
                            // If queue is full, wait a bit and retry once
                            tracing::debug!("Blueprint queue full, waiting before retry");
                            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                            // Final attempt - if this fails, drop the work
                            if let Err(e) = self.queue_adapter.try_push(next_work).await {
                                tracing::warn!(error = ?e, "Failed to queue next node after retry, dropping work");
                            }
                        }
                        Err(e) if e.to_string().contains("closed") => {
                            return Err(BlueprintEvaluationError::QueueClosed);
                        }
                        Err(e) => {
                            tracing::warn!(error = ?e, "Failed to queue next node");
                        }
                    }
                } else {
                    // Blueprint evaluation complete
                    let total_duration = work
                        .blueprint_start
                        .map(|start| (chrono::Utc::now() - start).num_milliseconds())
                        .unwrap_or(0);

                    info!(
                        blueprint = %work.blueprint,
                        nodes_evaluated = work.nodes_evaluated,
                        total_duration_ms = total_duration,
                        trace_id = ?work.trace_id,
                        "Blueprint evaluation completed"
                    );

                    // Record successful evaluation result
                    let queued_at = work.blueprint_start.unwrap_or_else(chrono::Utc::now);
                    let result = EvaluationResult::success(work.blueprint.clone(), queued_at);
                    if let Err(e) = self.evaluation_storage.record(result).await {
                        error!(error = ?e, "Failed to record evaluation success");
                    }
                }
            }
            Ok(None) => {
                debug!(
                    blueprint = %work.blueprint,
                    node.type = %node.node_type,
                    node.index = work.node_index,
                    duration_ms = duration.as_millis(),
                    "Node evaluation returned None (stopping pipeline)"
                );
            }
            Err(e) => {
                error!(
                    blueprint = %work.blueprint,
                    node.type = %node.node_type,
                    node.index = work.node_index,
                    error = ?e,
                    duration_ms = duration.as_millis(),
                    "Node evaluation failed"
                );

                // Disable the blueprint and record the error
                let error_message = format!(
                    "Node {} ({}) failed: {}",
                    work.node_index, node.node_type, e
                );

                if let Err(storage_err) = self
                    .blueprint_storage
                    .update_blueprint_error(&work.blueprint, false, Some(error_message.clone()))
                    .await
                {
                    error!(
                        blueprint = %work.blueprint,
                        error = ?storage_err,
                        "Failed to update blueprint error status"
                    );
                }

                // Record failed evaluation result
                let queued_at = work.blueprint_start.unwrap_or_else(chrono::Utc::now);
                let result = EvaluationResult::failure(
                    work.blueprint.clone(),
                    queued_at,
                    &node.aturi,
                    &e.to_string(),
                );
                if let Err(e) = self.evaluation_storage.record(result).await {
                    error!(error = ?e, "Failed to record evaluation failure");
                }
            }
        }

        Ok(())
    }
}

/// Submit a blueprint for evaluation.
pub async fn submit_blueprint(
    sender: &mpsc::Sender<BlueprintWork>,
    blueprint_aturi: String,
    data: Value,
    trace_id: Option<String>,
) -> Result<(), BlueprintEvaluationError> {
    let work = BlueprintWork {
        blueprint: blueprint_aturi.clone(),
        node_index: 0,
        payload: data,
        trace_id: trace_id.or_else(|| Some(uuid::Uuid::new_v4().to_string())),
        blueprint_start: Some(chrono::Utc::now()),
        nodes_evaluated: 0,
    };

    // Use try_send to avoid blocking the caller if the channel is full
    // This prevents backpressure from blocking the jetstream consumer
    match sender.try_send(work) {
        Ok(()) => Ok(()),
        Err(mpsc::error::TrySendError::Full(_)) => {
            // Channel is full - log and drop the work to prevent blocking
            tracing::warn!(
                blueprint = %blueprint_aturi,
                "Blueprint evaluation queue is full, dropping work item. Consider increasing buffer size or optimizing evaluation performance."
            );
            // Return Ok to prevent blocking - the event is dropped but processing continues
            Ok(())
        }
        Err(mpsc::error::TrySendError::Closed(_)) => {
            // Channel is closed - the evaluation task has stopped
            Err(BlueprintEvaluationError::QueueClosed)
        }
    }
}

/// Create a test work item for testing purposes.
pub fn create_test_work(blueprint_aturi: String, data: Value) -> BlueprintWork {
    BlueprintWork {
        blueprint: blueprint_aturi,
        node_index: 0,
        payload: data,
        trace_id: None,
        blueprint_start: Some(chrono::Utc::now()),
        nodes_evaluated: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_create_test_work() {
        let work = create_test_work("test_blueprint".to_string(), json!({"test": "data"}));
        assert_eq!(work.blueprint, "test_blueprint");
        assert_eq!(work.node_index, 0);
        assert!(work.trace_id.is_none());
        assert!(work.blueprint_start.is_some());
        assert_eq!(work.nodes_evaluated, 0);
    }
}
