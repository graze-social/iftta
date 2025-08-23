//! Background task for scheduling periodic blueprint evaluations.
//!
//! This module implements a scheduler that monitors blueprints with periodic_entry
//! nodes and triggers their evaluation based on cron schedules.

use chrono::Utc;
use croner::Cron;
use serde_json::Value;
use std::collections::HashSet;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use thiserror::Error;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, instrument, warn};

/// Scheduler task errors following the error-iftta-<domain>-<number> format
#[derive(Error, Debug)]
pub enum SchedulerError {
    #[error("error-iftta-scheduler-1 Blueprint storage operation failed: {0}")]
    BlueprintStorageError(#[from] crate::errors::StorageError),

    #[error("error-iftta-scheduler-2 Node storage operation failed: {0}")]
    NodeStorageError(String),

    #[error("error-iftta-scheduler-3 Scheduler state operation failed: {0}")]
    SchedulerStateError(String),

    #[error("error-iftta-scheduler-4 Queue adapter operation failed: {0}")]
    QueueAdapterError(String),

    #[error(
        "error-iftta-scheduler-5 Cron expression parsing failed: expression={expression} - {error}"
    )]
    CronParseError { expression: String, error: String },

    #[error("error-iftta-scheduler-6 Blueprint cache reload failed: {0}")]
    CacheReloadError(String),

    #[error("error-iftta-scheduler-7 Periodic input generation failed: {0}")]
    InputGenerationError(String),
}

use crate::{
    engine::node_type_periodic_entry::PeriodicEntryEvaluator,
    leadership::LeadershipElection,
    queue_adapter::QueueAdapter,
    storage::{
        blueprint::{Blueprint, BlueprintStorage},
        node::{Node, NodeStorage},
        scheduler_state::{SchedulerState, SchedulerStateStorage},
    },
    tasks::blueprint_adapter::BlueprintWork,
    validation::Validator,
};

/// Cached blueprint with its periodic_entry node for fast access
#[derive(Clone)]
struct CachedPeriodicBlueprint {
    blueprint: Blueprint,
    /// The periodic_entry node for this blueprint
    #[allow(dead_code)]
    periodic_node: Node,
    /// The cron expression string (for storage/display)
    cron_expression: String,
    /// The parsed cron object (for evaluation)
    cron: Cron,
}

/// Background task that manages scheduled blueprint evaluations.
///
/// The SchedulerTask monitors blueprints with periodic_entry nodes and triggers
/// their evaluation based on cron schedules. It maintains state about when each
/// blueprint was last run and when it should next be evaluated.
///
/// # Behavior
///
/// On start:
/// - Loads all enabled blueprints with periodic_entry nodes
/// - Initializes scheduler state for new blueprints
///
/// In the main loop (every 10 seconds):
/// 1. Evaluates blueprints that have never run before
/// 2. Evaluates blueprints where the next run time has passed
/// 3. Updates the scheduler state with new run times
/// 4. Sleeps for 10 seconds
pub struct SchedulerTask {
    blueprint_storage: Arc<dyn BlueprintStorage>,
    node_storage: Arc<dyn NodeStorage>,
    scheduler_state: Arc<dyn SchedulerStateStorage>,
    queue_adapter: Arc<dyn QueueAdapter<BlueprintWork>>,
    cancel_token: CancellationToken,
    /// Cached blueprints with periodic_entry nodes for efficient processing
    cached_blueprints: Arc<RwLock<Vec<CachedPeriodicBlueprint>>>,
    /// Last time the cache was reloaded
    last_cache_reload: Arc<RwLock<Instant>>,
    /// Minimum interval between cache reloads (default: 60 seconds)
    cache_reload_interval: Duration,
    /// Optional leadership election for distributed scheduling
    leadership_election: Option<Arc<dyn LeadershipElection>>,
    /// Allowed collections for publish_record nodes
    allowed_publish_collections: HashSet<String>,
}

impl SchedulerTask {
    /// Create a new scheduler task.
    pub fn new(
        blueprint_storage: Arc<dyn BlueprintStorage>,
        node_storage: Arc<dyn NodeStorage>,
        scheduler_state: Arc<dyn SchedulerStateStorage>,
        queue_adapter: Arc<dyn QueueAdapter<BlueprintWork>>,
        cancel_token: CancellationToken,
        allowed_publish_collections: HashSet<String>,
    ) -> Self {
        Self {
            blueprint_storage,
            node_storage,
            scheduler_state,
            queue_adapter,
            cancel_token,
            cached_blueprints: Arc::new(RwLock::new(Vec::new())),
            last_cache_reload: Arc::new(RwLock::new(Instant::now() - Duration::from_secs(60))),
            cache_reload_interval: Duration::from_secs(60),
            leadership_election: None,
            allowed_publish_collections,
        }
    }

    /// Create a new scheduler task with leadership election.
    pub fn with_leadership(
        blueprint_storage: Arc<dyn BlueprintStorage>,
        node_storage: Arc<dyn NodeStorage>,
        scheduler_state: Arc<dyn SchedulerStateStorage>,
        queue_adapter: Arc<dyn QueueAdapter<BlueprintWork>>,
        cancel_token: CancellationToken,
        allowed_publish_collections: HashSet<String>,
        leadership_election: Option<Arc<dyn LeadershipElection>>,
    ) -> Self {
        Self {
            blueprint_storage,
            node_storage,
            scheduler_state,
            queue_adapter,
            cancel_token,
            cached_blueprints: Arc::new(RwLock::new(Vec::new())),
            last_cache_reload: Arc::new(RwLock::new(Instant::now() - Duration::from_secs(60))),
            cache_reload_interval: Duration::from_secs(60),
            leadership_election,
            allowed_publish_collections,
        }
    }

    /// Run the scheduler task.
    #[instrument(skip_all)]
    pub async fn run(self) -> Result<(), SchedulerError> {
        info!(
            leadership_enabled = self.leadership_election.is_some(),
            "Starting scheduler task"
        );

        // Initial load of blueprints with periodic_entry nodes
        self.load_blueprints_cache().await?;
        self.initialize_scheduler_state().await?;

        // Main scheduler loop
        while !self.cancel_token.is_cancelled() {
            tokio::select! {
                () = tokio::time::sleep(Duration::from_secs(10)) => {
                    // Check leadership before processing
                    if let Some(ref leadership) = self.leadership_election {
                        match leadership.is_leader().await {
                            Ok(true) => {
                                // We are the leader, process scheduled blueprints
                                debug!("Processing scheduled blueprints as leader");
                            }
                            Ok(false) => {
                                // We are not the leader, skip processing
                                debug!("Skipping scheduled blueprints - not leader");
                                continue;
                            }
                            Err(e) => {
                                // Leadership check failed, assume not leader and skip
                                warn!(error = ?e, "Leadership check failed, skipping scheduled blueprints");
                                continue;
                            }
                        }
                    }
                    // If no leadership election configured, always process

                    // Check if cache needs reloading
                    if self.should_reload_cache().await
                        && let Err(e) = self.load_blueprints_cache().await {
                            error!(error = ?e, "Failed to reload blueprints cache");
                        }

                    if let Err(e) = self.process_scheduled_blueprints().await {
                        error!(error = ?e, "Error processing scheduled blueprints");
                    }
                }
                () = self.cancel_token.cancelled() => {
                    info!("Scheduler task cancelled");
                    break;
                }
            }
        }

        info!("Scheduler task stopped");
        Ok(())
    }

    /// Check if the cache should be reloaded
    async fn should_reload_cache(&self) -> bool {
        let last_reload = self.last_cache_reload.read().await;
        Instant::now().duration_since(*last_reload) > self.cache_reload_interval
    }

    /// Load all blueprints with periodic_entry nodes into cache
    async fn load_blueprints_cache(&self) -> Result<(), SchedulerError> {
        info!("Loading blueprints with periodic_entry nodes into cache");

        // Get all blueprints from storage
        let all_blueprints = self.blueprint_storage.list_blueprints(None).await?;

        let mut cached = Vec::new();
        let mut periodic_count = 0;
        let mut disabled_count = 0;
        let mut invalid_count = 0;

        for blueprint in all_blueprints {
            // Skip disabled blueprints
            if !blueprint.enabled {
                disabled_count += 1;
                continue;
            }

            // Get nodes for this blueprint
            let nodes = self.node_storage.list_nodes(&blueprint.aturi).await?;

            // Check if blueprint is valid
            if !blueprint.is_valid(&nodes) {
                invalid_count += 1;
                continue;
            }

            // Validate publish_record collections if constraints are configured
            if let Err(e) =
                Validator::validate_publish_collections(&nodes, &self.allowed_publish_collections)
            {
                warn!(
                    blueprint = %blueprint.aturi,
                    error = ?e,
                    "Blueprint skipped due to collection constraints"
                );
                invalid_count += 1;
                continue;
            }

            // Find periodic_entry node (should be first node)
            if let Some(first_node) = nodes.first()
                && first_node.node_type == "periodic_entry"
            {
                // Extract cron expression
                if let Some(cron_expr) = first_node
                    .configuration
                    .get("cron")
                    .and_then(|v| v.as_str())
                {
                    // Validate and parse the cron expression
                    if PeriodicEntryEvaluator::validate_cron_schedule(cron_expr).is_ok() {
                        if let Ok(cron) = Cron::from_str(cron_expr) {
                            cached.push(CachedPeriodicBlueprint {
                                blueprint: blueprint.clone(),
                                periodic_node: first_node.clone(),
                                cron_expression: cron_expr.to_string(),
                                cron,
                            });
                        } else {
                            warn!(
                                blueprint = %blueprint.aturi,
                                cron = %cron_expr,
                                "Failed to parse cron expression"
                            );
                            continue;
                        }
                        periodic_count += 1;
                        debug!(
                            blueprint = %blueprint.aturi,
                            cron = %cron_expr,
                            "Cached blueprint with periodic_entry"
                        );
                    } else {
                        warn!(
                            blueprint = %blueprint.aturi,
                            cron = %cron_expr,
                            "Invalid cron expression in blueprint"
                        );
                    }
                }
            }
        }

        // Update cache
        let mut cache = self.cached_blueprints.write().await;
        *cache = cached;

        // Update last reload time
        let mut last_reload = self.last_cache_reload.write().await;
        *last_reload = Instant::now();

        info!(
            periodic_count,
            disabled_count, invalid_count, "Loaded blueprints with periodic_entry nodes"
        );

        Ok(())
    }

    /// Initialize scheduler state for all cached blueprints with periodic_entry nodes.
    async fn initialize_scheduler_state(&self) -> Result<(), SchedulerError> {
        debug!("Initializing scheduler state");

        let cached_blueprints = self.cached_blueprints.read().await;

        for cached in cached_blueprints.iter() {
            // Check if we already have state for this blueprint
            if let Ok(existing_state) = self
                .scheduler_state
                .get_state(&cached.blueprint.aturi)
                .await
                && existing_state.is_some()
            {
                debug!(blueprint = %cached.blueprint.aturi, "Scheduler state already exists");
                continue;
            }

            // Calculate the initial next run time using the cached Cron object
            // Do not use fallback - if cron fails, the blueprint should be disabled
            let next_run_result = cached.cron.find_next_occurrence(&Utc::now(), false);

            match next_run_result {
                Ok(next_run) => {
                    let state = SchedulerState {
                        blueprint_id: cached.blueprint.aturi.clone(),
                        last_run: None,
                        next_run,
                        cron_expression: cached.cron_expression.clone(),
                        enabled: true,
                    };

                    if let Err(e) = self.scheduler_state.save_state(&state).await {
                        error!(
                            blueprint = %cached.blueprint.aturi,
                            error = ?e,
                            "Failed to save scheduler state"
                        );
                    } else {
                        info!(
                            blueprint = %cached.blueprint.aturi,
                            next_run = %next_run,
                            "Initialized scheduler state for blueprint"
                        );
                    }
                }
                Err(e) => {
                    // Do not create state for blueprints with invalid cron expressions
                    // This effectively disables the blueprint
                    error!(
                        blueprint = %cached.blueprint.aturi,
                        cron = %cached.cron_expression,
                        error = ?e,
                        "Failed to calculate initial run time - blueprint will be disabled"
                    );
                }
            }
        }

        Ok(())
    }

    /// Process blueprints that are ready to be evaluated.
    async fn process_scheduled_blueprints(&self) -> Result<(), SchedulerError> {
        let ready_blueprints = self
            .scheduler_state
            .get_ready_blueprints()
            .await
            .map_err(|e| SchedulerError::SchedulerStateError(e.to_string()))?;

        if !ready_blueprints.is_empty() {
            debug!(
                count = ready_blueprints.len(),
                "Processing scheduled blueprints"
            );
        }

        // Get cached blueprints for quick lookup
        let cached_blueprints = self.cached_blueprints.read().await;

        for state in ready_blueprints {
            // Find the cached blueprint for this state
            let cached_bp = cached_blueprints
                .iter()
                .find(|c| c.blueprint.aturi == state.blueprint_id);

            let cached_bp = match cached_bp {
                Some(bp) => bp,
                None => {
                    warn!(
                        blueprint = %state.blueprint_id,
                        "Blueprint not found in cache, skipping"
                    );
                    continue;
                }
            };

            // Queue the blueprint for evaluation
            let work = BlueprintWork {
                blueprint: state.blueprint_id.clone(),
                node_index: 0,
                payload: self.generate_periodic_input(&state).await?,
                trace_id: None,
                blueprint_start: None,
                nodes_evaluated: 0,
            };

            match self.queue_adapter.push(work).await {
                Ok(()) => {
                    info!(
                        blueprint = %state.blueprint_id,
                        "Queued periodic blueprint for evaluation"
                    );

                    // Update the scheduler state with new run times
                    let now = Utc::now();
                    // Use the cached Cron object for better performance
                    // No fallback - if cron fails, disable the blueprint
                    match cached_bp.cron.find_next_occurrence(&now, false) {
                        Ok(next_run) => {
                            if let Err(e) = self
                                .scheduler_state
                                .update_run_times(&state.blueprint_id, now, next_run)
                                .await
                            {
                                error!(
                                    blueprint = %state.blueprint_id,
                                    error = ?e,
                                    "Failed to update scheduler state"
                                );
                            } else {
                                debug!(
                                    blueprint = %state.blueprint_id,
                                    last_run = %now,
                                    next_run = %next_run,
                                    "Updated scheduler state"
                                );
                            }
                        }
                        Err(e) => {
                            // Disable the blueprint if we can't calculate next run time
                            error!(
                                blueprint = %state.blueprint_id,
                                cron = %state.cron_expression,
                                error = ?e,
                                "Failed to calculate next run time - disabling blueprint"
                            );

                            // Mark the blueprint as disabled in scheduler state
                            if let Ok(Some(mut current_state)) =
                                self.scheduler_state.get_state(&state.blueprint_id).await
                            {
                                current_state.enabled = false;
                                if let Err(e) =
                                    self.scheduler_state.save_state(&current_state).await
                                {
                                    error!(
                                        blueprint = %state.blueprint_id,
                                        error = ?e,
                                        "Failed to disable blueprint in scheduler state"
                                    );
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    error!(
                        blueprint = %state.blueprint_id,
                        error = ?e,
                        "Failed to queue blueprint for evaluation"
                    );
                }
            }
        }

        Ok(())
    }

    /// Generate the input data for a periodic blueprint evaluation.
    ///
    /// This creates an empty context that will be used by the periodic_entry
    /// node to generate its payload using DataLogic.
    async fn generate_periodic_input(
        &self,
        state: &SchedulerState,
    ) -> Result<Value, SchedulerError> {
        // For periodic entries, we provide minimal context
        // The periodic_entry node will generate its own payload
        Ok(serde_json::json!({
            "scheduler": {
                "blueprint_id": state.blueprint_id,
                "last_run": state.last_run.map(|dt| dt.to_rfc3339()),
                "cron_expression": state.cron_expression,
                "triggered_at": Utc::now().to_rfc3339()
            }
        }))
    }

    /// Clean up stale scheduler states for blueprints that no longer exist or
    /// no longer have periodic_entry nodes.
    pub async fn cleanup_stale_states(&self) -> Result<(), SchedulerError> {
        debug!("Cleaning up stale scheduler states");

        let all_states = self
            .scheduler_state
            .get_all_states()
            .await
            .map_err(|e| SchedulerError::SchedulerStateError(e.to_string()))?;
        let cached_blueprints = self.cached_blueprints.read().await;
        let mut removed_count = 0;

        for state in all_states {
            // Check if this blueprint is still in our cache (meaning it's valid and has periodic_entry)
            let still_valid = cached_blueprints
                .iter()
                .any(|c| c.blueprint.aturi == state.blueprint_id);

            if !still_valid {
                // Blueprint is no longer valid or doesn't have periodic_entry
                if let Err(e) = self.scheduler_state.delete_state(&state.blueprint_id).await {
                    error!(
                        blueprint = %state.blueprint_id,
                        error = ?e,
                        "Failed to delete stale scheduler state"
                    );
                } else {
                    info!(
                        blueprint = %state.blueprint_id,
                        "Removed scheduler state for invalid/missing blueprint"
                    );
                    removed_count += 1;
                }
            }
        }

        if removed_count > 0 {
            info!(count = removed_count, "Removed stale scheduler states");
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        queue_adapter::MpscQueueAdapter, storage::scheduler_state::MemorySchedulerStateStorage,
    };
    use async_trait::async_trait;
    use std::collections::HashMap;

    // Test-specific mock storage implementations
    struct MockBlueprintStorage {
        blueprints: Vec<Blueprint>,
    }

    #[async_trait]
    impl BlueprintStorage for MockBlueprintStorage {
        async fn get_blueprint(
            &self,
            aturi: &str,
        ) -> crate::storage::StorageResult<Option<Blueprint>> {
            Ok(self.blueprints.iter().find(|b| b.aturi == aturi).cloned())
        }

        async fn list_blueprints(
            &self,
            _did: Option<&str>,
        ) -> crate::storage::StorageResult<Vec<Blueprint>> {
            Ok(self.blueprints.clone())
        }

        async fn create_blueprint(
            &self,
            _blueprint: &Blueprint,
        ) -> crate::storage::StorageResult<()> {
            Ok(())
        }

        async fn update_blueprint(
            &self,
            _blueprint: &Blueprint,
        ) -> crate::storage::StorageResult<()> {
            Ok(())
        }

        async fn delete_blueprint(&self, _aturi: &str) -> crate::storage::StorageResult<()> {
            Ok(())
        }

        async fn update_blueprint_error(
            &self,
            _aturi: &str,
            _enabled: bool,
            _error: Option<String>,
        ) -> crate::storage::StorageResult<()> {
            Ok(())
        }

        async fn list_blueprints_paginated(
            &self,
            did: Option<&str>,
            limit: usize,
            offset: usize,
        ) -> crate::storage::StorageResult<Vec<Blueprint>> {
            let all = self.list_blueprints(did).await?;
            Ok(all.into_iter().skip(offset).take(limit).collect())
        }

        async fn count_blueprints(&self, did: Option<&str>) -> crate::storage::StorageResult<u32> {
            let all = self.list_blueprints(did).await?;
            Ok(all.len() as u32)
        }
    }

    struct MockNodeStorage {
        nodes: HashMap<String, Vec<Node>>,
    }

    #[async_trait]
    impl NodeStorage for MockNodeStorage {
        async fn get_node(&self, _aturi: &str) -> crate::storage::StorageResult<Option<Node>> {
            Ok(None)
        }

        async fn list_nodes(&self, blueprint: &str) -> crate::storage::StorageResult<Vec<Node>> {
            Ok(self.nodes.get(blueprint).cloned().unwrap_or_default())
        }

        async fn upsert_node(&self, _node: &Node) -> crate::storage::StorageResult<()> {
            Ok(())
        }

        async fn delete_node(&self, _aturi: &str) -> crate::storage::StorageResult<()> {
            Ok(())
        }

        async fn delete_nodes_for_blueprint(
            &self,
            _blueprint: &str,
        ) -> crate::storage::StorageResult<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_scheduler_initialization() {
        // Create in-memory storages
        let blueprint_storage = Arc::new(MockBlueprintStorage {
            blueprints: vec![Blueprint {
                aturi: "test_blueprint".to_string(),
                did: "did:web:example.com".to_string(),
                node_order: vec!["test_node".to_string(), "action_node".to_string()],
                enabled: true,
                error: None,
                created_at: Utc::now(),
            }],
        });

        let mut nodes = HashMap::new();
        nodes.insert(
            "test_blueprint".to_string(),
            vec![
                Node {
                    aturi: "test_node".to_string(),
                    blueprint: "test_blueprint".to_string(),
                    node_type: "periodic_entry".to_string(),
                    configuration: serde_json::json!({
                        "cron": "15 * * * *" // At minute 15 of every hour
                    }),
                    payload: serde_json::json!({"event": "scheduled"}),
                    created_at: Utc::now(),
                },
                Node {
                    aturi: "action_node".to_string(),
                    blueprint: "test_blueprint".to_string(),
                    node_type: "publish_record".to_string(),
                    configuration: serde_json::json!({}),
                    payload: serde_json::json!({}),
                    created_at: Utc::now(),
                },
            ],
        );

        let node_storage = Arc::new(MockNodeStorage { nodes });
        let scheduler_state = Arc::new(MemorySchedulerStateStorage::new());
        let queue_adapter = Arc::new(MpscQueueAdapter::<BlueprintWork>::new(100));

        // Create and run scheduler task briefly
        let cancel_token = CancellationToken::new();
        let task = SchedulerTask::new(
            blueprint_storage,
            node_storage,
            scheduler_state.clone(),
            queue_adapter,
            cancel_token.clone(),
            HashSet::new(), // No collection restrictions for test
        );

        // Load cache and initialize state
        task.load_blueprints_cache().await.unwrap();
        task.initialize_scheduler_state().await.unwrap();

        // Verify state was created
        let state = scheduler_state
            .get_state("test_blueprint")
            .await
            .unwrap()
            .expect("State should exist");

        assert_eq!(state.blueprint_id, "test_blueprint");
        assert_eq!(state.cron_expression, "15 * * * *");
        assert!(state.last_run.is_none());
        assert!(state.enabled);

        // Verify blueprint was cached
        let cached = task.cached_blueprints.read().await;
        assert_eq!(cached.len(), 1);
        assert_eq!(cached[0].blueprint.aturi, "test_blueprint");
        assert_eq!(cached[0].cron_expression, "15 * * * *");
    }

    #[tokio::test]
    async fn test_cleanup_stale_states() {
        // Create in-memory storages
        let blueprint_storage = Arc::new(MockBlueprintStorage {
            blueprints: vec![], // No blueprints
        });
        let node_storage = Arc::new(MockNodeStorage {
            nodes: HashMap::new(),
        });
        let scheduler_state = Arc::new(MemorySchedulerStateStorage::new());
        let queue_adapter = Arc::new(MpscQueueAdapter::<BlueprintWork>::new(100));

        // Add a state for a non-existent blueprint
        let stale_state = SchedulerState {
            blueprint_id: "non_existent".to_string(),
            last_run: None,
            next_run: Utc::now(),
            cron_expression: "0 * * * * *".to_string(),
            enabled: true,
        };
        scheduler_state.save_state(&stale_state).await.unwrap();

        // Create scheduler task
        let cancel_token = CancellationToken::new();
        let task = SchedulerTask::new(
            blueprint_storage,
            node_storage,
            scheduler_state.clone(),
            queue_adapter,
            cancel_token,
            HashSet::new(), // No collection restrictions for test
        );

        // Load cache (will be empty) and run cleanup
        task.load_blueprints_cache().await.unwrap();
        task.cleanup_stale_states().await.unwrap();

        // Verify state was removed
        let state = scheduler_state.get_state("non_existent").await.unwrap();
        assert!(state.is_none());
    }

    #[tokio::test]
    async fn test_scheduler_filters_invalid_blueprints() {
        // Create test blueprints - one valid, one without action node (invalid)
        let blueprint_storage = Arc::new(MockBlueprintStorage {
            blueprints: vec![
                Blueprint {
                    aturi: "valid_blueprint".to_string(),
                    did: "did:web:example.com".to_string(),
                    node_order: vec!["node1".to_string(), "action1".to_string()],
                    enabled: true,
                    error: None,
                    created_at: Utc::now(),
                },
                Blueprint {
                    aturi: "invalid_blueprint".to_string(),
                    did: "did:web:example.com".to_string(),
                    node_order: vec!["node2".to_string()], // No action node
                    enabled: true,
                    error: None,
                    created_at: Utc::now(),
                },
            ],
        });

        let mut nodes = HashMap::new();

        // Valid blueprint with periodic_entry and action
        nodes.insert(
            "valid_blueprint".to_string(),
            vec![
                Node {
                    aturi: "node1".to_string(),
                    blueprint: "valid_blueprint".to_string(),
                    node_type: "periodic_entry".to_string(),
                    configuration: serde_json::json!({
                        "cron": "0 * * * *" // Every hour (5-field format)
                    }),
                    payload: serde_json::json!({}),
                    created_at: Utc::now(),
                },
                Node {
                    aturi: "action1".to_string(),
                    blueprint: "valid_blueprint".to_string(),
                    node_type: "publish_record".to_string(),
                    configuration: serde_json::json!({}),
                    payload: serde_json::json!({}),
                    created_at: Utc::now(),
                },
            ],
        );

        // Invalid blueprint with only periodic_entry (no action)
        nodes.insert(
            "invalid_blueprint".to_string(),
            vec![Node {
                aturi: "node2".to_string(),
                blueprint: "invalid_blueprint".to_string(),
                node_type: "periodic_entry".to_string(),
                configuration: serde_json::json!({
                    "cron": "0 * * * *"
                }),
                payload: serde_json::json!({}),
                created_at: Utc::now(),
            }],
        );

        let node_storage = Arc::new(MockNodeStorage { nodes });
        let scheduler_state = Arc::new(MemorySchedulerStateStorage::new());
        let queue_adapter = Arc::new(MpscQueueAdapter::<BlueprintWork>::new(100));

        let cancel_token = CancellationToken::new();
        let task = SchedulerTask::new(
            blueprint_storage,
            node_storage,
            scheduler_state,
            queue_adapter,
            cancel_token,
            HashSet::new(), // No collection restrictions for test
        );

        // Load cache
        task.load_blueprints_cache().await.unwrap();

        // Check that only valid blueprint was cached
        let cached = task.cached_blueprints.read().await;
        assert_eq!(cached.len(), 1);
        assert_eq!(cached[0].blueprint.aturi, "valid_blueprint");
    }
}
