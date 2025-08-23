//! Task management and execution system.
//!
//! This module provides the core task execution infrastructure for ifthisthenat,
//! organizing different types of background tasks that process events, webhooks,
//! and scheduled operations.
//!
//! # Architecture
//!
//! The task system follows a modular design where each task type:
//! - Has its own execution logic and error handling
//! - Can use different queue adapters for work distribution
//! - Supports graceful shutdown via cancellation tokens
//! - Provides health checks and metrics
//!
//! # Task Types
//!
//! ## Blueprint Evaluation
//! Processes automation blueprints by evaluating nodes sequentially.
//! Supports distributed processing with Redis queues.
//!
//! ## Webhook Delivery
//! Manages outbound webhook delivery with retry logic and backoff.
//! Handles both synchronous and queued webhook processing.
//!
//! ## Scheduler
//! Executes periodic tasks based on cron schedules.
//! Manages blueprint evaluation triggered by time-based events.
//!
//! ## Task Manager
//! Coordinates multiple task instances and manages their lifecycle.
//! Provides centralized task monitoring and control.
//!
//! # Example
//!
//! ```rust,ignore
//! use ifthisthenat::tasks::{
//!     BlueprintEvaluationTask,
//!     WebhookTask,
//!     SchedulerTask,
//!     TaskManager,
//! };
//! use tokio_util::sync::CancellationToken;
//!
//! #[tokio::main]
//! async fn main() -> Result<()> {
//!     let cancel_token = CancellationToken::new();
//!     
//!     // Create task manager
//!     let mut manager = TaskManager::new(cancel_token.clone());
//!     
//!     // Add blueprint evaluation task
//!     let blueprint_task = BlueprintEvaluationTask::new(/* ... */);
//!     manager.add_task("blueprint", blueprint_task);
//!     
//!     // Add webhook task
//!     let webhook_task = WebhookTask::new(/* ... */);
//!     manager.add_task("webhook", webhook_task);
//!     
//!     // Run all tasks
//!     manager.run().await?;
//!     
//!     Ok(())
//! }
//! ```

// Task implementations
pub mod blueprint;
pub mod manager;
pub mod scheduler;
pub mod webhook;

// Blueprint queue adapter
pub mod blueprint_adapter;

// Re-export main types for convenience
pub use blueprint::{
    BlueprintEvaluationError, BlueprintEvaluationTask, BlueprintWork, create_test_work,
    submit_blueprint,
};

pub use webhook::{WebhookError, WebhookMetadata, WebhookTask, WebhookTaskConfig, WebhookWork};

pub use scheduler::{SchedulerError, SchedulerTask};

// Re-export manager functions
pub use manager::{spawn_cancellable_task, spawn_managed_task};

pub use blueprint_adapter::{
    BlueprintQueueAdapter, MpscBlueprintQueueAdapter, RedisBlueprintQueueAdapter,
    create_blueprint_queue_adapter,
};

/// Common trait for all background tasks.
///
/// This trait defines the interface that all task types must implement
/// to be managed by the TaskManager.
use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

#[async_trait]
pub trait Task: Send + Sync {
    /// Run the task until cancelled or an error occurs.
    ///
    /// Tasks should:
    /// - Check the cancellation token periodically
    /// - Handle errors gracefully
    /// - Clean up resources on shutdown
    async fn run(&self, cancel_token: CancellationToken) -> anyhow::Result<()>;

    /// Check if the task is healthy.
    ///
    /// Used for health checks and monitoring.
    async fn is_healthy(&self) -> bool {
        true
    }

    /// Get task-specific metrics.
    ///
    /// Returns key-value pairs of metrics for monitoring.
    async fn get_metrics(&self) -> Vec<(String, f64)> {
        vec![]
    }

    /// Get a human-readable name for the task.
    fn name(&self) -> &str;
}
