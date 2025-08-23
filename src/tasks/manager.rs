//! Task management utilities for consistent background task handling
//!
//! This module provides helpers for spawning and managing background tasks with:
//! - Consistent start/stop logging
//! - Automatic shutdown on task failure
//! - Graceful shutdown support

use std::future::Future;
use tokio_util::{sync::CancellationToken, task::TaskTracker};
use tracing::{error, info};

/// Spawn a background task with consistent lifecycle management
///
/// This function:
/// 1. Logs when the task starts
/// 2. Logs when the task completes (success or failure)
/// 3. Triggers application shutdown on task failure
/// 4. Supports graceful shutdown via cancellation token
pub fn spawn_managed_task<F>(
    tracker: &TaskTracker,
    app_token: CancellationToken,
    task_name: &'static str,
    task_future: F,
) where
    F: Future<Output = anyhow::Result<()>> + Send + 'static,
{
    info!(task = task_name, "Starting background task");

    let task_token = app_token.clone();

    tracker.spawn(async move {
        // Run the task and handle its result
        match task_future.await {
            Ok(()) => {
                info!(task = task_name, "Background task completed successfully");
            }
            Err(e) => {
                error!(task = task_name, error = ?e, "Background task failed unexpectedly");
                // Trigger application shutdown on task failure
                task_token.cancel();
            }
        }
    });
}

/// Spawn a background task with cancellation support
///
/// This version allows the task to be cancelled via the token and handles
/// both graceful shutdown and unexpected failures
pub fn spawn_cancellable_task<F, Fut>(
    tracker: &TaskTracker,
    app_token: CancellationToken,
    task_builder: F,
) where
    F: FnOnce(CancellationToken) -> Fut + Send + 'static,
    Fut: Future<Output = anyhow::Result<()>> + Send + 'static,
{
    info!("Starting cancellable background task");

    let task_token = app_token.clone();
    let cancel_token = app_token.clone();

    tracker.spawn(async move {
        tokio::select! {
            result = task_builder(cancel_token.clone()) => {
                match result {
                    Ok(()) => {
                        info!("Background task completed successfully");
                    }
                    Err(e) => {
                        error!(error = ?e, "Background task failed unexpectedly");
                        // Trigger application shutdown on task failure
                        task_token.cancel();
                    }
                }
            }
            () = task_token.cancelled() => {
                info!("Background task shutting down gracefully");
            }
        }
    });
}

/// Helper for tasks that need both cancellation and custom shutdown logic
pub fn spawn_task_with_shutdown<F, S>(
    tracker: &TaskTracker,
    app_token: CancellationToken,
    task_name: &'static str,
    task_future: F,
    shutdown_handler: S,
) where
    F: Future<Output = anyhow::Result<()>> + Send + 'static,
    S: Future<Output = ()> + Send + 'static,
{
    info!(
        task = task_name,
        "Starting background task with custom shutdown"
    );

    let task_token = app_token.clone();

    tracker.spawn(async move {
        tokio::select! {
            result = task_future => {
                match result {
                    Ok(()) => {
                        info!(task = task_name, "Background task completed successfully");
                    }
                    Err(e) => {
                        error!(task = task_name, error = ?e, "Background task failed unexpectedly");
                        // Trigger application shutdown on task failure
                        task_token.cancel();
                    }
                }
            }
            () = task_token.cancelled() => {
                info!(task = task_name, "Background task shutting down gracefully");
                shutdown_handler.await;
                info!(task = task_name, "Background task shutdown complete");
            }
        }
    });
}

/// Macro for consistent task error handling within a task
#[macro_export]
macro_rules! task_try {
    ($expr:expr, $task_name:expr) => {
        match $expr {
            Ok(val) => val,
            Err(e) => {
                tracing::error!(task = $task_name, error = ?e, "Task operation failed");
                return Err(e.into());
            }
        }
    };
}

/// Macro for logging task checkpoints
#[macro_export]
macro_rules! task_checkpoint {
    ($task_name:expr, $checkpoint:expr) => {
        tracing::debug!(
            task = $task_name,
            checkpoint = $checkpoint,
            "Task checkpoint reached"
        );
    };
}
