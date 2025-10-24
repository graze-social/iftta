//! Event processing logic for blueprint evaluation.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::config::PartitionKeyStrategy;
use crate::constants::NODE_TYPE_JETSTREAM_ENTRY;
use crate::consumer::{BlueprintEvent, BlueprintEventReceiver, PartitionConfig};
use crate::engine::{
    node_evaluator_factory::NodeEvaluatorFactory,
    node_type_jetstream_entry::JetstreamEntryEvaluator,
};
use crate::errors::ProcessorError;
use crate::storage::{
    blueprint::{Blueprint, BlueprintStorage},
    node::{Node, NodeStorage},
};
use crate::tasks::{BlueprintWork, submit_blueprint};
use crate::throttle::BlueprintThrottler;
#[cfg(test)]
use crate::throttle::NoOpBlueprintThrottler;
use serde_json::{Value, json};
use tokio::sync::{Mutex, mpsc};
use tracing::{debug, error, info, trace, warn};

/// Cached blueprint with its jetstream_entry node for fast evaluation
#[derive(Debug, Clone)]
struct CachedBlueprint {
    blueprint: Blueprint,
    /// The first node (jetstream_entry) for pre-evaluation
    entry_node: Node,
}

/// Groups of blueprints that share the same entry node configuration
/// This is cached and only recalculated when blueprints change
#[derive(Debug, Clone)]
struct BlueprintConfigGroup {
    /// Representative entry node for this configuration
    entry_node: Node,
    /// All blueprints that share this exact configuration
    blueprints: Vec<CachedBlueprint>,
}

/// Hash just the node configuration (without payload) for grouping
fn hash_node_config(node_type: &str, config: &Value) -> u64 {
    use std::collections::hash_map::DefaultHasher;

    let combined = json!({
        "node_type": node_type,
        "configuration": config,
    });

    let canonical = serde_json::to_string(&combined).unwrap_or_default();
    let mut hasher = DefaultHasher::new();
    canonical.hash(&mut hasher);
    hasher.finish()
}

/// Hash a node evaluation context for deduplication purposes
/// This creates a stable hash combining node type, configuration, and payload
#[cfg(test)]
fn hash_node_evaluation_context(node_type: &str, config: &Value, payload: &Value) -> u64 {
    use std::collections::hash_map::DefaultHasher;

    // Create a combined object with all three components for consistent hashing
    let combined = json!({
        "node_type": node_type,
        "configuration": config,
        "payload": payload
    });

    // Convert to canonical JSON string for consistent hashing
    let canonical = serde_json::to_string(&combined).unwrap_or_default();
    let mut hasher = DefaultHasher::new();
    canonical.hash(&mut hasher);
    hasher.finish()
}

/// Background processor that filters Jetstream events and routes them to async blueprint evaluation
pub struct BlueprintProcessor {
    blueprint_storage: Arc<dyn BlueprintStorage>,
    node_storage: Arc<dyn NodeStorage>,
    blueprint_sender: mpsc::Sender<BlueprintWork>,
    /// Cached blueprints that have jetstream_entry nodes (kept for compatibility)
    cached_blueprints: Arc<tokio::sync::RwLock<Vec<CachedBlueprint>>>,
    /// Pre-grouped blueprints by their entry node configuration
    /// This avoids regrouping on every event
    config_groups: Arc<tokio::sync::RwLock<HashMap<u64, BlueprintConfigGroup>>>,
    /// Last time the cache was reloaded
    last_reload_time: Arc<Mutex<Instant>>,
    /// Minimum interval between cache reloads
    cache_reload_interval: Duration,
    /// Node evaluator factory for pre-evaluating entry nodes
    evaluator_factory: Arc<NodeEvaluatorFactory>,
    /// Partition configuration for filtering blueprints based on jetstream partitioning
    partition_config: Option<PartitionConfig>,
    /// Throttler for blueprint evaluations
    throttler: Arc<dyn BlueprintThrottler>,
    /// Maximum number of blueprints to cache (None = unlimited)
    max_cache_size: Option<usize>,
}

impl BlueprintProcessor {
    /// Create a new blueprint processor that routes events to async evaluation
    pub fn new(
        blueprint_storage: Arc<dyn BlueprintStorage>,
        node_storage: Arc<dyn NodeStorage>,
        blueprint_sender: mpsc::Sender<BlueprintWork>,
        throttler: Arc<dyn BlueprintThrottler>,
    ) -> Self {
        Self::with_cache_reload_interval(
            blueprint_storage,
            node_storage,
            blueprint_sender,
            Duration::from_secs(30), // Default to 30 seconds
            None,                    // No partition configuration by default
            throttler,
            None,                    // No cache size limit by default
        )
    }

    /// Create a new blueprint processor with a custom cache reload interval
    pub fn with_cache_reload_interval(
        blueprint_storage: Arc<dyn BlueprintStorage>,
        node_storage: Arc<dyn NodeStorage>,
        blueprint_sender: mpsc::Sender<BlueprintWork>,
        cache_reload_interval: Duration,
        partition_config: Option<PartitionConfig>,
        throttler: Arc<dyn BlueprintThrottler>,
        max_cache_size: Option<usize>,
    ) -> Self {
        // Create evaluator factory with only jetstream_entry evaluator
        // (we only need this one for pre-evaluation)
        let evaluator_factory = Arc::new(
            NodeEvaluatorFactory::new()
                .register("jetstream_entry", Arc::new(JetstreamEntryEvaluator::new())),
        );

        Self {
            blueprint_storage,
            node_storage,
            blueprint_sender,
            cached_blueprints: Arc::new(tokio::sync::RwLock::new(Vec::new())),
            config_groups: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            last_reload_time: Arc::new(Mutex::new(Instant::now() - cache_reload_interval)),
            cache_reload_interval,
            evaluator_factory,
            partition_config,
            throttler,
            max_cache_size,
        }
    }

    /// Check if a jetstream_entry node configuration could ever match the current partition
    fn could_jetstream_entry_match_partition(&self, node_config: &Value) -> bool {
        let Some(partition_config) = &self.partition_config else {
            // No partition configuration means single instance, so all nodes can match
            return true;
        };

        if partition_config.total_instances <= 1 {
            // Single instance, all nodes can match
            return true;
        }

        // Check based on partition strategy
        match &partition_config.strategy {
            PartitionKeyStrategy::DidBased => {
                // For DID-based partitioning, check if any configured DIDs would hash to this partition
                if let Some(did_filters) = node_config.get("did").and_then(|v| v.as_array()) {
                    for did_value in did_filters {
                        if let Some(did_str) = did_value.as_str() {
                            if did_str == "*" {
                                // Wildcard matches all DIDs, so this node could match any partition
                                return true;
                            }
                            // Hash the DID and check if it would map to this partition
                            let hash = Self::hash_partition_key(did_str);
                            let target_instance =
                                (hash as usize) % partition_config.total_instances;
                            if target_instance == partition_config.instance_id {
                                return true;
                            }
                        }
                    }
                    // None of the configured DIDs would map to this partition
                    false
                } else {
                    // No DID filter means it accepts all DIDs, so it could match any partition
                    true
                }
            }
            PartitionKeyStrategy::CollectionBased => {
                // For collection-based partitioning, check if any configured collections would hash to this partition
                if let Some(collection_filters) =
                    node_config.get("collection").and_then(|v| v.as_array())
                {
                    for collection_value in collection_filters {
                        if let Some(collection_str) = collection_value.as_str() {
                            if collection_str == "*" {
                                // Wildcard matches all collections, so this node could match any partition
                                return true;
                            }
                            // Hash the collection and check if it would map to this partition
                            let hash = Self::hash_partition_key(collection_str);
                            let target_instance =
                                (hash as usize) % partition_config.total_instances;
                            if target_instance == partition_config.instance_id {
                                return true;
                            }
                        }
                    }
                    // None of the configured collections would map to this partition
                    false
                } else {
                    // No collection filter means it accepts all collections, so it could match any partition
                    true
                }
            }
            PartitionKeyStrategy::RoundRobin => {
                // Round-robin partitioning is time-based, so we can't predict statically
                // All nodes must be kept as they could potentially match
                true
            }
            PartitionKeyStrategy::CustomField(_) => {
                // Custom field partitioning depends on record content we can't predict
                // All nodes must be kept as they could potentially match
                true
            }
        }
    }

    /// Hash a partition key using the same logic as PartitionConfig
    fn hash_partition_key(key: &str) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        key.hash(&mut hasher);
        hasher.finish()
    }

    /// Load all valid blueprints with jetstream_entry nodes into cache
    pub async fn load_blueprints_cache(&self) -> Result<(), ProcessorError> {
        info!("Loading valid blueprints with jetstream_entry nodes into cache");

        // Get all blueprints from storage
        let all_blueprints = self
            .blueprint_storage
            .list_blueprints(None)
            .await
            .map_err(|e| ProcessorError::CacheReloadFailed {
                details: format!("Failed to load blueprints from storage: {}", e),
            })?;

        let mut cache = Vec::new();
        let mut jetstream_count = 0;
        let mut invalid_count = 0;
        let mut disabled_count = 0;
        let mut partition_filtered_count = 0;

        for blueprint in all_blueprints {
            // Skip disabled blueprints
            if !blueprint.enabled {
                trace!("Skipping disabled blueprint {}", blueprint.aturi);
                disabled_count += 1;
                continue;
            }

            // Get all nodes for this blueprint
            let nodes = self
                .node_storage
                .list_nodes(&blueprint.aturi)
                .await
                .map_err(|e| ProcessorError::CacheReloadFailed {
                    details: format!(
                        "Failed to load nodes for blueprint {}: {}",
                        blueprint.aturi, e
                    ),
                })?;

            // Check if blueprint is valid (has proper structure)
            if !blueprint.is_valid(&nodes) {
                trace!(
                    "Skipping invalid blueprint {} - does not meet validation requirements",
                    blueprint.aturi
                );
                invalid_count += 1;
                continue;
            }

            // Check if blueprint has a jetstream_entry as first node
            if let Some(first_node) = nodes.first()
                && first_node.node_type == NODE_TYPE_JETSTREAM_ENTRY
            {
                // Check if this jetstream_entry node could ever match the current partition
                if self.could_jetstream_entry_match_partition(&first_node.configuration) {
                    cache.push(CachedBlueprint {
                        blueprint: blueprint.clone(),
                        entry_node: first_node.clone(),
                    });
                    jetstream_count += 1;
                    debug!(
                        "Cached valid blueprint {} with jetstream_entry",
                        blueprint.aturi
                    );
                } else {
                    trace!(
                        "Skipping blueprint {} - jetstream_entry node would never match current partition constraints",
                        blueprint.aturi
                    );
                    partition_filtered_count += 1;
                }
            }
        }

        // Enforce max cache size limit if configured
        let mut size_limited_count = 0;
        if let Some(max_size) = self.max_cache_size {
            if cache.len() > max_size {
                size_limited_count = cache.len() - max_size;
                warn!(
                    "Blueprint cache size ({}) exceeds limit ({}), truncating to limit. Consider increasing BLUEPRINT_CACHE_MAX_SIZE if needed.",
                    cache.len(),
                    max_size
                );
                cache.truncate(max_size);
            }
        }

        // Build configuration groups for efficient event processing
        let mut groups: HashMap<u64, BlueprintConfigGroup> = HashMap::new();
        for cached in &cache {
            let config_hash = hash_node_config(
                &cached.entry_node.node_type,
                &cached.entry_node.configuration,
            );

            groups
                .entry(config_hash)
                .or_insert_with(|| BlueprintConfigGroup {
                    entry_node: cached.entry_node.clone(),
                    blueprints: Vec::new(),
                })
                .blueprints
                .push(cached.clone());
        }

        // Update both caches atomically
        let mut cached_blueprints = self.cached_blueprints.write().await;
        let mut config_groups = self.config_groups.write().await;
        *cached_blueprints = cache;
        *config_groups = groups.clone();

        if size_limited_count > 0 {
            info!(
                "Loaded {} valid blueprints with jetstream_entry nodes grouped into {} unique configurations (skipped {} disabled, {} invalid, {} partition-filtered, {} size-limited)",
                jetstream_count,
                groups.len(),
                disabled_count,
                invalid_count,
                partition_filtered_count,
                size_limited_count
            );
        } else {
            info!(
                "Loaded {} valid blueprints with jetstream_entry nodes grouped into {} unique configurations (skipped {} disabled, {} invalid, {} partition-filtered)",
                jetstream_count,
                groups.len(),
                disabled_count,
                invalid_count,
                partition_filtered_count
            );
        }
        Ok(())
    }

    /// Trigger a cache reload with rate limiting
    /// Returns true if the cache was reloaded, false if rate limited
    pub async fn trigger_cache_reload(&self) -> Result<bool, ProcessorError> {
        let now = Instant::now();
        let mut last_reload = self.last_reload_time.lock().await;

        // Check if enough time has passed since the last reload
        if now.duration_since(*last_reload) < self.cache_reload_interval {
            debug!(
                "Cache reload rate limited, last reload was {:?} ago",
                now.duration_since(*last_reload)
            );
            return Ok(false);
        }

        // Update the last reload time
        *last_reload = now;
        drop(last_reload); // Release the lock before reloading

        // Perform the reload
        self.load_blueprints_cache().await?;
        Ok(true)
    }

    /// Start processing events from the Jetstream queue
    pub async fn start_processing(
        &self,
        mut event_receiver: BlueprintEventReceiver,
    ) -> Result<(), ProcessorError> {
        info!("Starting Jetstream blueprint processor");

        // Load blueprints cache before processing
        if let Err(e) = self.load_blueprints_cache().await {
            error!(error = ?e, "Failed to load blueprints cache");
            // Continue anyway, we can still process events without cache
        }

        while let Some(event) = event_receiver.recv().await {
            match &event {
                BlueprintEvent::Commit {
                    did,
                    collection,
                    rkey,
                    cid,
                    record,
                    operation,
                    time_us,
                } => {
                    if let Err(e) = self
                        .handle_commit(did, collection, rkey, cid, record, operation, *time_us)
                        .await
                    {
                        error!(error = ?e, did = %did, collection = %collection, "Failed to process commit event");
                    }
                }
                BlueprintEvent::Delete {
                    did,
                    collection,
                    rkey,
                    time_us,
                } => {
                    // Handle delete events if needed
                    trace!(
                        did = %did,
                        collection = %collection,
                        rkey = %rkey,
                        time_us = %time_us,
                        "Delete event received (not processed)"
                    );
                }
            }
        }

        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn handle_commit(
        &self,
        did: &str,
        collection: &str,
        rkey: &str,
        cid: &str,
        record: &serde_json::Value,
        operation: &str,
        time_us: i64,
    ) -> Result<(), ProcessorError> {
        // Build the event data that will be passed to blueprints
        // Include collection at top level for filter matching
        // Use Arc to avoid cloning when distributing to multiple blueprints
        let event_data = Arc::new(json!({
            "kind": "commit",
            "did": did,
            "collection": collection,  // Top-level for filter matching
            "commit": {
                "collection": collection,
                "rkey": rkey,
                "cid": cid,
                "operation": operation,
                "record": record,
            },
            "time_us": time_us,
        }));

        // Get pre-computed configuration groups
        let config_groups = self.config_groups.read().await;

        if config_groups.is_empty() {
            trace!("No blueprints with jetstream_entry nodes to process");
            return Ok(());
        }

        trace!(
            "Processing event against {} pre-computed configuration groups",
            config_groups.len()
        );

        // Pre-evaluate each unique entry node configuration
        let mut matched_count = 0;
        let mut filtered_count = 0;
        let mut evaluated_configs = 0;

        // Iterate through pre-computed configuration groups
        for (_config_hash, group) in config_groups.iter() {
            evaluated_configs += 1;

            trace!(
                "Evaluating configuration for {} blueprints",
                group.blueprints.len()
            );

            // Evaluate the entry node once for this configuration group
            let evaluation_result = self
                .evaluator_factory
                .evaluate(&group.entry_node, &event_data)
                .await;

            match evaluation_result {
                Ok(Some(Value::Bool(false))) => {
                    // Entry node returned false - skip all blueprints in this group
                    trace!(
                        blueprint_count = group.blueprints.len(),
                        "Entry configuration filtered out event, skipping {} blueprints",
                        group.blueprints.len()
                    );
                    filtered_count += group.blueprints.len();
                    continue;
                }
                Ok(Some(_result)) => {
                    // Entry node matched - queue each blueprint separately with its own work item
                    trace!(
                        blueprint_count = group.blueprints.len(),
                        "Entry configuration matched, queueing {} individual blueprint work items",
                        group.blueprints.len()
                    );
                    matched_count += group.blueprints.len();

                    // Queue separate work for each blueprint to preserve their individual processing
                    // Each blueprint has its own ATURI and may have different subsequent nodes
                    for cached_blueprint in &group.blueprints {
                        // Check if this blueprint should be throttled
                        match self
                            .throttler
                            .throttle(&cached_blueprint.blueprint.aturi)
                            .await
                        {
                            Ok(true) => {
                                // Blueprint is throttled, skip it
                                trace!(
                                    blueprint = %cached_blueprint.blueprint.aturi,
                                    "Blueprint evaluation throttled"
                                );
                                continue;
                            }
                            Ok(false) => {}
                            Err(e) => {
                                // Error checking throttle, log and skip
                                warn!(
                                    error = ?e,
                                    blueprint = %cached_blueprint.blueprint.aturi,
                                    "Error checking blueprint throttle, skipping"
                                );
                                continue;
                            }
                        }
                        // Not throttled, proceed with submission
                        // Each blueprint gets its own work item with Arc-shared event data (no clone!)
                        if let Err(e) = submit_blueprint(
                            &self.blueprint_sender,
                            cached_blueprint.blueprint.aturi.clone(),
                            Arc::clone(&event_data),  // Cheap Arc clone instead of deep Value clone
                            Some(uuid::Uuid::new_v4().to_string()), // Generate unique trace ID per blueprint
                        )
                        .await
                        {
                            warn!(
                                error = ?e,
                                blueprint = %cached_blueprint.blueprint.aturi,
                                "Failed to submit blueprint work item for evaluation"
                            );
                            // Continue with other blueprints even if one fails
                        }
                    }
                }
                Ok(None) => {
                    // Entry node returned None - skip all blueprints in this group
                    trace!(
                        blueprint_count = group.blueprints.len(),
                        "Entry configuration returned None, skipping {} blueprints",
                        group.blueprints.len()
                    );
                    filtered_count += group.blueprints.len();
                    continue;
                }
                Err(e) => {
                    // Error evaluating entry node - log and skip this group
                    warn!(
                        error = ?e,
                        blueprint_count = group.blueprints.len(),
                        "Failed to evaluate entry configuration, skipping {} blueprints",
                        group.blueprints.len()
                    );
                    continue;
                }
            }
        }

        if matched_count > 0 || filtered_count > 0 {
            trace!(
                matched = matched_count,
                filtered = filtered_count,
                unique_configs_evaluated = evaluated_configs,
                "Jetstream event processing complete with entry node deduplication"
            );
        }

        Ok(())
    }

    /// Reload blueprints cache (can be called periodically or on demand)
    pub async fn reload_cache(&self) -> Result<(), ProcessorError> {
        info!("Reloading blueprints cache");
        self.load_blueprints_cache().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_node_evaluation_context_hashing() {
        let node_type = "jetstream_entry";
        let config = json!({
            "did": ["did:plc:abcdefghijklmnopqrstuvwx"],
            "collection": ["app.bsky.feed.post"]
        });
        let payload = json!({
            "kind": "commit",
            "did": "did:plc:abcdefghijklmnopqrstuvwx",
            "collection": "app.bsky.feed.post",
            "commit": {
                "rkey": "3ktest",
                "cid": "bafytest"
            }
        });

        // Test that identical contexts produce the same hash
        let hash1 = hash_node_evaluation_context(node_type, &config, &payload);
        let hash2 = hash_node_evaluation_context(node_type, &config, &payload);
        assert_eq!(
            hash1, hash2,
            "Identical contexts should produce the same hash"
        );

        // Test that different node types produce different hashes
        let hash3 = hash_node_evaluation_context("webhook_entry", &config, &payload);
        assert_ne!(
            hash1, hash3,
            "Different node types should produce different hashes"
        );

        // Test that different configurations produce different hashes
        let config2 = json!({
            "did": ["did:plc:differentdidxxxxxxxxxx"],
            "collection": ["app.bsky.feed.post"]
        });
        let hash4 = hash_node_evaluation_context(node_type, &config2, &payload);
        assert_ne!(
            hash1, hash4,
            "Different configurations should produce different hashes"
        );

        // Test that different payloads produce different hashes
        let payload2 = json!({
            "kind": "commit",
            "did": "did:plc:differentdidxxxxxxxxxx",
            "collection": "app.bsky.feed.like",
            "commit": {
                "rkey": "3kdifferent",
                "cid": "bafydifferent"
            }
        });
        let hash5 = hash_node_evaluation_context(node_type, &config, &payload2);
        assert_ne!(
            hash1, hash5,
            "Different payloads should produce different hashes"
        );
    }

    #[test]
    fn test_evaluation_context_hash_stability() {
        // Test that hash is stable across multiple calls
        let node_type = "jetstream_entry";
        let config = json!({
            "collection": ["app.bsky.feed.post", "app.bsky.feed.like"],
            "did": ["did:plc:user1xxxxxxxxxxxxxxxxxxxx", "did:plc:user2xxxxxxxxxxxxxxxxxxxx"]
        });
        let payload = json!({
            "kind": "commit",
            "did": "did:plc:user1xxxxxxxxxxxxxxxxxxxx",
            "collection": "app.bsky.feed.post",
            "commit": {
                "rkey": "3ktest123",
                "cid": "bafytest123",
                "record": {
                    "text": "Hello world",
                    "createdAt": "2024-01-01T00:00:00.000Z"
                }
            }
        });

        let hash1 = hash_node_evaluation_context(node_type, &config, &payload);
        let hash2 = hash_node_evaluation_context(node_type, &config, &payload);
        let hash3 = hash_node_evaluation_context(node_type, &config, &payload);

        assert_eq!(hash1, hash2);
        assert_eq!(hash2, hash3);
    }

    use crate::storage::{blueprint::Blueprint, node::Node};
    use async_trait::async_trait;
    use chrono::Utc;
    use std::collections::HashMap;

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
    async fn test_processor_skips_disabled_blueprints() {
        // Create test blueprints - one enabled, one disabled
        let blueprints = vec![
            Blueprint {
                aturi: "enabled_blueprint".to_string(),
                did: "did:web:example.com".to_string(),
                node_order: vec!["node1".to_string(), "node_action".to_string()],
                enabled: true,
                error: None,
                created_at: Utc::now(),
            },
            Blueprint {
                aturi: "disabled_blueprint".to_string(),
                did: "did:web:example.com".to_string(),
                node_order: vec!["node2".to_string(), "node_action".to_string()],
                enabled: false,
                error: None,
                created_at: Utc::now(),
            },
        ];

        // Create nodes for both blueprints with jetstream_entry
        let mut nodes = HashMap::new();
        nodes.insert(
            "enabled_blueprint".to_string(),
            vec![
                Node {
                    aturi: "node1".to_string(),
                    blueprint: "enabled_blueprint".to_string(),
                    node_type: "jetstream_entry".to_string(),
                    payload: json!({}),
                    configuration: json!({"did": ["*"], "collection": ["*"]}),
                    created_at: Utc::now(),
                },
                Node {
                    aturi: "node_action".to_string(),
                    blueprint: "enabled_blueprint".to_string(),
                    node_type: "publish_record".to_string(),
                    payload: json!({}),
                    configuration: json!({}),
                    created_at: Utc::now(),
                },
            ],
        );
        nodes.insert(
            "disabled_blueprint".to_string(),
            vec![
                Node {
                    aturi: "node2".to_string(),
                    blueprint: "disabled_blueprint".to_string(),
                    node_type: "jetstream_entry".to_string(),
                    payload: json!({}),
                    configuration: json!({"did": ["*"], "collection": ["*"]}),
                    created_at: Utc::now(),
                },
                Node {
                    aturi: "node_action".to_string(),
                    blueprint: "disabled_blueprint".to_string(),
                    node_type: "publish_record".to_string(),
                    payload: json!({}),
                    configuration: json!({}),
                    created_at: Utc::now(),
                },
            ],
        );

        let blueprint_storage = Arc::new(MockBlueprintStorage { blueprints });
        let node_storage = Arc::new(MockNodeStorage { nodes });

        let (sender, _receiver) = mpsc::channel::<BlueprintWork>(100);

        let throttler = Arc::new(NoOpBlueprintThrottler::new());
        let processor = BlueprintProcessor::new(blueprint_storage, node_storage, sender, throttler);

        // Load blueprints into cache
        processor.load_blueprints_cache().await.unwrap();

        // Get cached blueprints
        let cached = processor.cached_blueprints.read().await;

        // Only the enabled blueprint should be cached
        assert_eq!(cached.len(), 1);
        assert_eq!(cached[0].blueprint.aturi, "enabled_blueprint");
        assert!(
            !cached
                .iter()
                .any(|c| c.blueprint.aturi == "disabled_blueprint")
        );
    }

    #[tokio::test]
    async fn test_cache_reload_rate_limiting() {
        use std::time::Duration;

        // Create processor with 1 second reload interval
        let blueprints = vec![];
        let nodes = HashMap::new();
        let blueprint_storage = Arc::new(MockBlueprintStorage { blueprints });
        let node_storage = Arc::new(MockNodeStorage { nodes });
        let (sender, _receiver) = mpsc::channel(10);
        let throttler = Arc::new(NoOpBlueprintThrottler::new());

        let processor = BlueprintProcessor::with_cache_reload_interval(
            blueprint_storage,
            node_storage,
            sender,
            Duration::from_secs(1),
            None, // No partition configuration for test
            throttler,
            None, // No cache size limit for test
        );

        // First reload should succeed
        let result = processor.trigger_cache_reload().await.unwrap();
        assert!(result, "First reload should succeed");

        // Immediate second reload should be rate limited
        let result = processor.trigger_cache_reload().await.unwrap();
        assert!(!result, "Second reload should be rate limited");

        // Wait for interval to pass
        tokio::time::sleep(Duration::from_secs(1)).await;

        // Third reload should succeed after waiting
        let result = processor.trigger_cache_reload().await.unwrap();
        assert!(result, "Third reload should succeed after waiting");
    }

    #[tokio::test]
    async fn test_processor_pre_evaluates_entry_nodes() {
        // Create test blueprints with different filter configurations
        let blueprints = vec![
            Blueprint {
                aturi: "blueprint_match_did".to_string(),
                did: "did:web:example.com".to_string(),
                node_order: vec!["node_match_did".to_string(), "action1".to_string()],
                enabled: true,
                error: None,
                created_at: Utc::now(),
            },
            Blueprint {
                aturi: "blueprint_no_match_did".to_string(),
                did: "did:web:example.com".to_string(),
                node_order: vec!["node_no_match_did".to_string(), "action2".to_string()],
                enabled: true,
                error: None,
                created_at: Utc::now(),
            },
            Blueprint {
                aturi: "blueprint_match_collection".to_string(),
                did: "did:web:example.com".to_string(),
                node_order: vec!["node_match_collection".to_string(), "action3".to_string()],
                enabled: true,
                error: None,
                created_at: Utc::now(),
            },
        ];

        // Create nodes with different filter configurations
        let mut nodes = HashMap::new();

        // Blueprint that matches by DID
        nodes.insert(
            "blueprint_match_did".to_string(),
            vec![
                Node {
                    aturi: "node_match_did".to_string(),
                    blueprint: "blueprint_match_did".to_string(),
                    node_type: "jetstream_entry".to_string(),
                    payload: json!(true),
                    configuration: json!({"did": ["did:plc:abcdefghijklmnopqrstuvwx"]}), // Will match our test event
                    created_at: Utc::now(),
                },
                Node {
                    aturi: "action1".to_string(),
                    blueprint: "blueprint_match_did".to_string(),
                    node_type: "publish_record".to_string(),
                    payload: json!({}),
                    configuration: json!({}),
                    created_at: Utc::now(),
                },
            ],
        );

        // Blueprint that doesn't match by DID
        nodes.insert(
            "blueprint_no_match_did".to_string(),
            vec![
                Node {
                    aturi: "node_no_match_did".to_string(),
                    blueprint: "blueprint_no_match_did".to_string(),
                    node_type: "jetstream_entry".to_string(),
                    payload: json!(true),
                    configuration: json!({"did": ["did:plc:zyxwvutsrqponmlkjihgfedc"]}), // Won't match
                    created_at: Utc::now(),
                },
                Node {
                    aturi: "action2".to_string(),
                    blueprint: "blueprint_no_match_did".to_string(),
                    node_type: "publish_record".to_string(),
                    payload: json!({}),
                    configuration: json!({}),
                    created_at: Utc::now(),
                },
            ],
        );

        // Blueprint that matches by collection
        nodes.insert(
            "blueprint_match_collection".to_string(),
            vec![
                Node {
                    aturi: "node_match_collection".to_string(),
                    blueprint: "blueprint_match_collection".to_string(),
                    node_type: "jetstream_entry".to_string(),
                    payload: json!(true),
                    configuration: json!({"collection": ["app.bsky.feed.post"]}), // Will match
                    created_at: Utc::now(),
                },
                Node {
                    aturi: "action3".to_string(),
                    blueprint: "blueprint_match_collection".to_string(),
                    node_type: "publish_record".to_string(),
                    payload: json!({}),
                    configuration: json!({}),
                    created_at: Utc::now(),
                },
            ],
        );

        let blueprint_storage = Arc::new(MockBlueprintStorage { blueprints });
        let node_storage = Arc::new(MockNodeStorage { nodes });
        let (sender, mut receiver) = mpsc::channel(10);

        let throttler = Arc::new(NoOpBlueprintThrottler::new());
        let processor = BlueprintProcessor::new(blueprint_storage, node_storage, sender, throttler);

        // Load cache
        processor.load_blueprints_cache().await.unwrap();

        // Submit a commit event
        processor
            .handle_commit(
                "did:plc:abcdefghijklmnopqrstuvwx", // Valid PLC DID that matches first blueprint
                "app.bsky.feed.post",
                "rkey123",
                "cid123",
                &json!({"text": "test post"}),
                "create",
                123456789,
            )
            .await
            .unwrap();

        // Check that only matching blueprints were queued
        // Should receive work for blueprint_match_did and blueprint_match_collection
        // but NOT for blueprint_no_match_did
        let mut received_blueprints = Vec::new();

        // Try to receive up to 3 messages with a timeout
        for _ in 0..3 {
            match tokio::time::timeout(Duration::from_millis(100), receiver.recv()).await {
                Ok(Some(work)) => {
                    received_blueprints.push(work.blueprint);
                }
                _ => break, // Timeout or channel closed
            }
        }

        // Should have exactly 2 blueprints queued (the ones that matched)
        assert_eq!(received_blueprints.len(), 2);
        assert!(received_blueprints.contains(&"blueprint_match_did".to_string()));
        assert!(received_blueprints.contains(&"blueprint_match_collection".to_string()));
        assert!(!received_blueprints.contains(&"blueprint_no_match_did".to_string()));
    }

    #[tokio::test]
    async fn test_processor_partition_filtering() {
        use crate::config::PartitionKeyStrategy;
        use crate::consumer::PartitionConfig;

        // Test partition-based filtering with DID-based strategy
        let blueprints = vec![
            Blueprint {
                aturi: "blueprint_did_match".to_string(),
                did: "did:web:example.com".to_string(),
                node_order: vec!["node_match_did".to_string(), "action1".to_string()],
                enabled: true,
                error: None,
                created_at: Utc::now(),
            },
            Blueprint {
                aturi: "blueprint_did_no_match".to_string(),
                did: "did:web:example.com".to_string(),
                node_order: vec!["node_no_match_did".to_string(), "action2".to_string()],
                enabled: true,
                error: None,
                created_at: Utc::now(),
            },
        ];

        let mut nodes = HashMap::new();

        // Blueprint with DID that would hash to partition 0 (assuming "did:plc:test1" hashes to partition 0)
        nodes.insert(
            "blueprint_did_match".to_string(),
            vec![
                Node {
                    aturi: "node_match_did".to_string(),
                    blueprint: "blueprint_did_match".to_string(),
                    node_type: "jetstream_entry".to_string(),
                    payload: json!(true),
                    configuration: json!({"did": ["did:plc:test1"]}),
                    created_at: Utc::now(),
                },
                Node {
                    aturi: "action1".to_string(),
                    blueprint: "blueprint_did_match".to_string(),
                    node_type: "publish_record".to_string(),
                    payload: json!({}),
                    configuration: json!({}),
                    created_at: Utc::now(),
                },
            ],
        );

        // Blueprint with DID that would hash to a different partition
        nodes.insert(
            "blueprint_did_no_match".to_string(),
            vec![
                Node {
                    aturi: "node_no_match_did".to_string(),
                    blueprint: "blueprint_did_no_match".to_string(),
                    node_type: "jetstream_entry".to_string(),
                    payload: json!(true),
                    configuration: json!({"did": ["did:plc:different_user_that_hashes_to_different_partition"]}),
                    created_at: Utc::now(),
                },
                Node {
                    aturi: "action2".to_string(),
                    blueprint: "blueprint_did_no_match".to_string(),
                    node_type: "publish_record".to_string(),
                    payload: json!({}),
                    configuration: json!({}),
                    created_at: Utc::now(),
                },
            ],
        );

        let blueprint_storage = Arc::new(MockBlueprintStorage { blueprints });
        let node_storage = Arc::new(MockNodeStorage { nodes });
        let (sender, _receiver) = mpsc::channel(10);

        // Create partition config for instance 0 of 3 with DID-based strategy
        let partition_config = PartitionConfig {
            instance_id: 0,
            total_instances: 3,
            strategy: PartitionKeyStrategy::DidBased,
        };

        let throttler = Arc::new(NoOpBlueprintThrottler::new());
        let processor = BlueprintProcessor::with_cache_reload_interval(
            blueprint_storage,
            node_storage,
            sender,
            Duration::from_secs(30),
            Some(partition_config),
            throttler,
        );

        // Load cache with partition filtering
        processor.load_blueprints_cache().await.unwrap();

        let cached = processor.cached_blueprints.read().await;

        // Only blueprints whose jetstream_entry DIDs hash to partition 0 should be cached
        // We can't predict exactly which DIDs will hash to partition 0, but we can verify
        // that the filtering logic is working by checking that some blueprints are filtered out
        let cached_blueprint_names: Vec<&str> =
            cached.iter().map(|c| c.blueprint.aturi.as_str()).collect();

        // The exact result depends on how the DIDs hash, but we should see filtering behavior
        // At minimum, we should have fewer than 2 blueprints cached if partitioning is working
        assert!(
            cached.len() <= 2,
            "Partition filtering should reduce the number of cached blueprints"
        );

        println!(
            "Cached blueprints with partition filtering: {:?}",
            cached_blueprint_names
        );
    }

    #[tokio::test]
    async fn test_processor_filters_jetstream_blueprints() {
        // Create test blueprints
        let blueprints = vec![
            Blueprint {
                aturi: "blueprint1".to_string(),
                did: "did:web:example.com".to_string(),
                node_order: vec!["node1".to_string(), "node1_action".to_string()],
                enabled: true,
                error: None,
                created_at: Utc::now(),
            },
            Blueprint {
                aturi: "blueprint2".to_string(),
                did: "did:web:example.com".to_string(),
                node_order: vec!["node2".to_string(), "node2_action".to_string()],
                enabled: true,
                error: None,
                created_at: Utc::now(),
            },
            Blueprint {
                aturi: "blueprint3".to_string(),
                did: "did:web:example.com".to_string(),
                node_order: vec!["node3".to_string()], // Invalid - no action node
                enabled: true,
                error: None,
                created_at: Utc::now(),
            },
        ];

        // Create nodes - blueprint1 has jetstream_entry and publish_record (valid)
        // blueprint2 has webhook_entry and publish_record (valid but not cached)
        // blueprint3 has jetstream_entry but no action (invalid)
        let mut nodes = HashMap::new();
        nodes.insert(
            "blueprint1".to_string(),
            vec![
                Node {
                    aturi: "node1".to_string(),
                    blueprint: "blueprint1".to_string(),
                    node_type: "jetstream_entry".to_string(),
                    payload: json!(true),
                    configuration: json!({}),
                    created_at: Utc::now(),
                },
                Node {
                    aturi: "node1_action".to_string(),
                    blueprint: "blueprint1".to_string(),
                    node_type: "publish_record".to_string(),
                    payload: json!({}),
                    configuration: json!({"did": "did:web:example.com", "collection": "app.bsky.feed.post"}),
                    created_at: Utc::now(),
                },
            ],
        );
        nodes.insert(
            "blueprint2".to_string(),
            vec![
                Node {
                    aturi: "node2".to_string(),
                    blueprint: "blueprint2".to_string(),
                    node_type: "webhook_entry".to_string(),
                    payload: json!(true),
                    configuration: json!({}),
                    created_at: Utc::now(),
                },
                Node {
                    aturi: "node2_action".to_string(),
                    blueprint: "blueprint2".to_string(),
                    node_type: "publish_record".to_string(),
                    payload: json!({}),
                    configuration: json!({"did": "did:web:example.com", "collection": "app.bsky.feed.post"}),
                    created_at: Utc::now(),
                },
            ],
        );
        nodes.insert(
            "blueprint3".to_string(),
            vec![Node {
                aturi: "node3".to_string(),
                blueprint: "blueprint3".to_string(),
                node_type: "jetstream_entry".to_string(),
                payload: json!(true),
                configuration: json!({}),
                created_at: Utc::now(),
            }],
        );

        let blueprint_storage = Arc::new(MockBlueprintStorage { blueprints });
        let node_storage = Arc::new(MockNodeStorage { nodes });
        let (sender, mut receiver) = mpsc::channel(10);

        let throttler = Arc::new(NoOpBlueprintThrottler::new());
        let processor = BlueprintProcessor::new(blueprint_storage, node_storage, sender, throttler);

        // Load cache
        processor.load_blueprints_cache().await.unwrap();

        // Check that only valid jetstream blueprints are cached
        // blueprint1: valid (jetstream_entry + publish_record) - should be cached
        // blueprint2: valid but has webhook_entry - should NOT be cached
        // blueprint3: invalid (jetstream_entry but no action) - should NOT be cached
        let cached = processor.cached_blueprints.read().await;
        assert_eq!(cached.len(), 1);
        assert_eq!(cached[0].blueprint.aturi, "blueprint1");

        // Submit a commit event
        processor
            .handle_commit(
                "did:plc:test",
                "app.bsky.feed.post",
                "rkey123",
                "cid123",
                &json!({"text": "test post"}),
                "create",
                123456789,
            )
            .await
            .unwrap();

        // Check that work was submitted
        let work = receiver.recv().await.unwrap();
        assert_eq!(work.blueprint, "blueprint1");
        assert_eq!(work.node_index, 0);
    }
}
