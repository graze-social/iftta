use async_trait::async_trait;
use chrono::Utc;
use ifthisthenat::storage::StorageResult;
use ifthisthenat::storage::blueprint::{Blueprint, BlueprintStorage};
use std::sync::Arc;
use tokio::sync::Mutex;

/// Test storage that tracks blueprint error updates
struct TestBlueprintStorage {
    blueprints: Arc<Mutex<Vec<Blueprint>>>,
    error_updates: Arc<Mutex<Vec<(String, bool, Option<String>)>>>,
}

impl TestBlueprintStorage {
    fn new() -> Self {
        Self {
            blueprints: Arc::new(Mutex::new(Vec::new())),
            error_updates: Arc::new(Mutex::new(Vec::new())),
        }
    }

    async fn add_blueprint(&self, blueprint: Blueprint) {
        let mut blueprints = self.blueprints.lock().await;
        blueprints.push(blueprint);
    }

    async fn get_error_updates(&self) -> Vec<(String, bool, Option<String>)> {
        self.error_updates.lock().await.clone()
    }
}

#[async_trait]
impl BlueprintStorage for TestBlueprintStorage {
    async fn get_blueprint(&self, aturi: &str) -> StorageResult<Option<Blueprint>> {
        let blueprints = self.blueprints.lock().await;
        Ok(blueprints.iter().find(|b| b.aturi == aturi).cloned())
    }

    async fn list_blueprints(&self, did: Option<&str>) -> StorageResult<Vec<Blueprint>> {
        let blueprints = self.blueprints.lock().await;
        if let Some(did) = did {
            Ok(blueprints
                .iter()
                .filter(|b| b.did == did)
                .cloned()
                .collect())
        } else {
            Ok(blueprints.clone())
        }
    }

    async fn create_blueprint(&self, blueprint: &Blueprint) -> StorageResult<()> {
        let mut blueprints = self.blueprints.lock().await;
        blueprints.push(blueprint.clone());
        Ok(())
    }

    async fn update_blueprint(&self, blueprint: &Blueprint) -> StorageResult<()> {
        let mut blueprints = self.blueprints.lock().await;
        if let Some(existing) = blueprints.iter_mut().find(|b| b.aturi == blueprint.aturi) {
            *existing = blueprint.clone();
        }
        Ok(())
    }

    async fn delete_blueprint(&self, aturi: &str) -> StorageResult<()> {
        let mut blueprints = self.blueprints.lock().await;
        blueprints.retain(|b| b.aturi != aturi);
        Ok(())
    }

    async fn update_blueprint_error(
        &self,
        aturi: &str,
        enabled: bool,
        error: Option<String>,
    ) -> StorageResult<()> {
        // Record the update
        let mut updates = self.error_updates.lock().await;
        updates.push((aturi.to_string(), enabled, error.clone()));

        // Update the blueprint
        let mut blueprints = self.blueprints.lock().await;
        if let Some(blueprint) = blueprints.iter_mut().find(|b| b.aturi == aturi) {
            blueprint.enabled = enabled;
            blueprint.error = error;
        }
        Ok(())
    }

    async fn list_blueprints_paginated(
        &self,
        did: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> StorageResult<Vec<Blueprint>> {
        let all = self.list_blueprints(did).await?;
        Ok(all.into_iter().skip(offset).take(limit).collect())
    }

    async fn count_blueprints(&self, did: Option<&str>) -> StorageResult<u32> {
        let all = self.list_blueprints(did).await?;
        Ok(all.len() as u32)
    }
}

#[tokio::test]
async fn test_blueprint_error_tracking() {
    let storage = TestBlueprintStorage::new();

    // Create a test blueprint
    let blueprint = Blueprint {
        aturi: "test_blueprint".to_string(),
        did: "did:plc:test123".to_string(),
        node_order: vec!["node1".to_string()],
        enabled: true,
        error: None,
        created_at: Utc::now(),
    };

    storage.add_blueprint(blueprint.clone()).await;

    // Verify initial state
    let retrieved = storage
        .get_blueprint("test_blueprint")
        .await
        .unwrap()
        .unwrap();
    assert!(retrieved.enabled);
    assert!(retrieved.error.is_none());

    // Simulate an error occurring
    storage
        .update_blueprint_error(
            "test_blueprint",
            false,
            Some("Node 0 (jetstream_entry) failed: Invalid configuration".to_string()),
        )
        .await
        .unwrap();

    // Verify error was recorded
    let retrieved = storage
        .get_blueprint("test_blueprint")
        .await
        .unwrap()
        .unwrap();
    assert!(!retrieved.enabled);
    assert_eq!(
        retrieved.error,
        Some("Node 0 (jetstream_entry) failed: Invalid configuration".to_string())
    );

    // Verify update history
    let updates = storage.get_error_updates().await;
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].0, "test_blueprint");
    assert_eq!(updates[0].1, false);
    assert!(updates[0].2.is_some());

    // Clear the error (simulating a fix)
    storage
        .update_blueprint_error("test_blueprint", true, None)
        .await
        .unwrap();

    // Verify error was cleared
    let retrieved = storage
        .get_blueprint("test_blueprint")
        .await
        .unwrap()
        .unwrap();
    assert!(retrieved.enabled);
    assert!(retrieved.error.is_none());

    // Verify update history
    let updates = storage.get_error_updates().await;
    assert_eq!(updates.len(), 2);
    assert_eq!(updates[1].0, "test_blueprint");
    assert_eq!(updates[1].1, true);
    assert!(updates[1].2.is_none());
}

#[tokio::test]
async fn test_disabled_blueprint_skipped() {
    let storage = TestBlueprintStorage::new();

    // Create an enabled blueprint
    let enabled_blueprint = Blueprint {
        aturi: "enabled_blueprint".to_string(),
        did: "did:plc:test123".to_string(),
        node_order: vec!["node1".to_string()],
        enabled: true,
        error: None,
        created_at: Utc::now(),
    };

    // Create a disabled blueprint with error
    let disabled_blueprint = Blueprint {
        aturi: "disabled_blueprint".to_string(),
        did: "did:plc:test123".to_string(),
        node_order: vec!["node2".to_string()],
        enabled: false,
        error: Some("Previous failure".to_string()),
        created_at: Utc::now(),
    };

    storage.add_blueprint(enabled_blueprint).await;
    storage.add_blueprint(disabled_blueprint).await;

    // List all blueprints
    let all_blueprints = storage.list_blueprints(None).await.unwrap();
    assert_eq!(all_blueprints.len(), 2);

    // Filter enabled blueprints (this would be done by the processor)
    let enabled_blueprints: Vec<_> = all_blueprints.into_iter().filter(|b| b.enabled).collect();
    assert_eq!(enabled_blueprints.len(), 1);
    assert_eq!(enabled_blueprints[0].aturi, "enabled_blueprint");
}
