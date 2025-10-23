//! Blueprint evaluation result storage trait and implementations

use crate::errors::StorageError;
use anyhow::Result;
use async_trait::async_trait;
use atproto_record::aturi::ATURI;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    str::FromStr,
};
use ulid::Ulid;

/// Represents the result of a blueprint evaluation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvaluationResult {
    /// The globally unique identifier for the evaluation
    pub id: String,
    /// The AT-URI of the blueprint that was evaluated
    pub aturi: String,
    /// The time that the blueprint was first queued for evaluation
    pub queued_at: DateTime<Utc>,
    /// The time that blueprint evaluation concluded
    pub completed_at: DateTime<Utc>,
    /// "OK" or a "[node-aturi] [error]" string
    pub result: String,
}

impl EvaluationResult {
    /// Create a new successful evaluation result
    pub fn success(aturi: String, queued_at: DateTime<Utc>) -> Self {
        Self {
            id: Ulid::new().to_string(),
            aturi,
            queued_at,
            completed_at: Utc::now(),
            result: "OK".to_string(),
        }
    }

    /// Create a new failed evaluation result
    pub fn failure(aturi: String, queued_at: DateTime<Utc>, node_aturi: &str, error: &str) -> Self {
        Self {
            id: Ulid::new().to_string(),
            aturi,
            queued_at,
            completed_at: Utc::now(),
            result: format!("[{}] {}", node_aturi, error),
        }
    }
}

/// Trait for storing blueprint evaluation results
#[async_trait]
pub trait EvaluationResultStorage: Send + Sync {
    /// Record a new evaluation result
    async fn record(&self, result: EvaluationResult) -> Result<()>;

    /// Get an evaluation result by ID (optional, for debugging)
    async fn get(&self, id: &str) -> Result<Option<EvaluationResult>>;

    /// List recent evaluation results for a blueprint (optional, for debugging)
    async fn list_by_blueprint(&self, aturi: &str, limit: usize) -> Result<Vec<EvaluationResult>>;
}

/// No-op implementation for development and testing
pub struct NoopEvaluationResultStorage;

impl NoopEvaluationResultStorage {
    pub fn new() -> Self {
        Self
    }
}

impl Default for NoopEvaluationResultStorage {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl EvaluationResultStorage for NoopEvaluationResultStorage {
    async fn record(&self, _result: EvaluationResult) -> Result<()> {
        // Do nothing
        Ok(())
    }

    async fn get(&self, _id: &str) -> Result<Option<EvaluationResult>> {
        // Always return None
        Ok(None)
    }

    async fn list_by_blueprint(
        &self,
        _aturi: &str,
        _limit: usize,
    ) -> Result<Vec<EvaluationResult>> {
        // Always return empty list
        Ok(Vec::new())
    }
}

/// Tracing implementation that logs evaluation results
pub struct TracingEvaluationResultStorage;

impl TracingEvaluationResultStorage {
    pub fn new() -> Self {
        Self
    }
}

impl Default for TracingEvaluationResultStorage {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl EvaluationResultStorage for TracingEvaluationResultStorage {
    async fn record(&self, result: EvaluationResult) -> Result<()> {
        let duration_ms = (result.completed_at - result.queued_at).num_milliseconds();

        if result.result == "OK" {
            tracing::info!(
                evaluation.id = %result.id,
                blueprint.aturi = %result.aturi,
                evaluation.duration_ms = duration_ms,
                evaluation.queued_at = %result.queued_at.to_rfc3339(),
                evaluation.completed_at = %result.completed_at.to_rfc3339(),
                evaluation.result = "success",
                "Blueprint evaluation completed successfully"
            );
        } else {
            // Parse the error to extract node AT-URI if present
            let (node_aturi, error_msg) = if result.result.starts_with('[') {
                if let Some(end_bracket) = result.result.find(']') {
                    let node = &result.result[1..end_bracket];
                    let error = result.result[end_bracket + 1..].trim();
                    (Some(node), error)
                } else {
                    (None, result.result.as_str())
                }
            } else {
                (None, result.result.as_str())
            };

            if let Some(node) = node_aturi {
                tracing::info!(
                    evaluation.id = %result.id,
                    blueprint.aturi = %result.aturi,
                    node.aturi = %node,
                    evaluation.duration_ms = duration_ms,
                    evaluation.queued_at = %result.queued_at.to_rfc3339(),
                    evaluation.completed_at = %result.completed_at.to_rfc3339(),
                    evaluation.result = "failure",
                    evaluation.error = %error_msg,
                    "Blueprint evaluation failed at node"
                );
            } else {
                tracing::info!(
                    evaluation.id = %result.id,
                    blueprint.aturi = %result.aturi,
                    evaluation.duration_ms = duration_ms,
                    evaluation.queued_at = %result.queued_at.to_rfc3339(),
                    evaluation.completed_at = %result.completed_at.to_rfc3339(),
                    evaluation.result = "failure",
                    evaluation.error = %error_msg,
                    "Blueprint evaluation failed"
                );
            }
        }

        Ok(())
    }

    async fn get(&self, id: &str) -> Result<Option<EvaluationResult>> {
        tracing::debug!(
            evaluation.id = %id,
            "Get evaluation result requested (not implemented in tracing storage)"
        );
        Ok(None)
    }

    async fn list_by_blueprint(&self, aturi: &str, limit: usize) -> Result<Vec<EvaluationResult>> {
        tracing::debug!(
            blueprint.aturi = %aturi,
            list.limit = limit,
            "List evaluation results requested (not implemented in tracing storage)"
        );
        Ok(Vec::new())
    }
}

/// Filesystem-based implementation that stores evaluation results as JSON files
pub struct FilesystemEvaluationResultStorage {
    base_directory: PathBuf,
}

impl FilesystemEvaluationResultStorage {
    /// Create a new filesystem storage with the given base directory
    pub fn new<P: AsRef<Path>>(base_directory: P) -> Self {
        Self {
            base_directory: base_directory.as_ref().to_path_buf(),
        }
    }

    /// Parse AT-URI to extract DID and blueprint ID
    /// Example: at://did:plc:cbkjy5n7bk3ax2wplmtjofq2/tools.graze.ifthisthenat.blueprint/01K4ZD4FRHPDWPJD9TVVSNMYQ6
    /// Returns: (did:plc:cbkjy5n7bk3ax2wplmtjofq2, 01K4ZD4FRHPDWPJD9TVVSNMYQ6)
    fn parse_aturi(aturi: &str) -> Result<(String, String)> {
        let parsed = ATURI::from_str(aturi).map_err(|e| StorageError::InvalidInput {
            details: format!("Invalid AT-URI: {}", e),
        })?;

        let did = parsed.authority;
        let blueprint_id = parsed.record_key;

        Ok((did, blueprint_id))
    }

    /// Get the directory path for a blueprint
    fn get_blueprint_directory(&self, did: &str, blueprint_id: &str) -> PathBuf {
        self.base_directory.join(did).join(blueprint_id)
    }

    /// Get the file path for an evaluation result
    fn get_evaluation_file_path(
        &self,
        did: &str,
        blueprint_id: &str,
        evaluation_id: &str,
    ) -> PathBuf {
        self.get_blueprint_directory(did, blueprint_id)
            .join(format!("{}.json", evaluation_id))
    }

    /// Find the evaluation file by searching all blueprint directories
    async fn find_evaluation_file(&self, evaluation_id: &str) -> Result<Option<PathBuf>> {
        let base_dir = &self.base_directory;

        if !base_dir.exists() {
            return Ok(None);
        }

        // Iterate through DID directories
        let mut did_entries = tokio::fs::read_dir(base_dir).await?;
        while let Some(did_entry) = did_entries.next_entry().await? {
            if !did_entry.file_type().await?.is_dir() {
                continue;
            }

            let did_path = did_entry.path();

            // Iterate through blueprint directories
            let mut blueprint_entries = tokio::fs::read_dir(&did_path).await?;
            while let Some(blueprint_entry) = blueprint_entries.next_entry().await? {
                if !blueprint_entry.file_type().await?.is_dir() {
                    continue;
                }

                let blueprint_path = blueprint_entry.path();
                let evaluation_file = blueprint_path.join(format!("{}.json", evaluation_id));

                if evaluation_file.exists() {
                    return Ok(Some(evaluation_file));
                }
            }
        }

        Ok(None)
    }
}

#[async_trait]
impl EvaluationResultStorage for FilesystemEvaluationResultStorage {
    async fn record(&self, result: EvaluationResult) -> Result<()> {
        // Parse the AT-URI to get DID and blueprint ID
        let (did, blueprint_id) = Self::parse_aturi(&result.aturi)?;

        // Get the directory path
        let dir_path = self.get_blueprint_directory(&did, &blueprint_id);

        // Create directories if they don't exist
        tokio::fs::create_dir_all(&dir_path).await?;

        // Get the file path
        let file_path = self.get_evaluation_file_path(&did, &blueprint_id, &result.id);

        // Serialize the result to JSON
        let json_content = serde_json::to_string_pretty(&result)?;

        // Write to file
        tokio::fs::write(&file_path, json_content).await?;

        tracing::debug!(
            evaluation.id = %result.id,
            blueprint.aturi = %result.aturi,
            file.path = %file_path.display(),
            "Saved evaluation result to filesystem"
        );

        Ok(())
    }

    async fn get(&self, id: &str) -> Result<Option<EvaluationResult>> {
        // Find the evaluation file by searching through directories
        let file_path = match self.find_evaluation_file(id).await? {
            Some(path) => path,
            None => return Ok(None),
        };

        // Read the file
        let json_content = tokio::fs::read_to_string(&file_path).await?;

        // Deserialize from JSON
        let result: EvaluationResult = serde_json::from_str(&json_content)?;

        Ok(Some(result))
    }

    async fn list_by_blueprint(&self, aturi: &str, limit: usize) -> Result<Vec<EvaluationResult>> {
        // Parse the AT-URI to get DID and blueprint ID
        let (did, blueprint_id) = Self::parse_aturi(aturi)?;

        // Get the directory path
        let dir_path = self.get_blueprint_directory(&did, &blueprint_id);

        // Check if directory exists
        if !dir_path.exists() {
            return Ok(Vec::new());
        }

        let mut results = Vec::new();

        // Read all JSON files in the directory
        let mut entries = tokio::fs::read_dir(&dir_path).await?;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();

            // Skip if not a JSON file
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }

            // Read and deserialize the file
            match tokio::fs::read_to_string(&path).await {
                Ok(json_content) => match serde_json::from_str::<EvaluationResult>(&json_content) {
                    Ok(result) => results.push(result),
                    Err(e) => {
                        tracing::warn!(
                            file.path = %path.display(),
                            error = %e,
                            "Failed to deserialize evaluation result"
                        );
                    }
                },
                Err(e) => {
                    tracing::warn!(
                        file.path = %path.display(),
                        error = %e,
                        "Failed to read evaluation result file"
                    );
                }
            }
        }

        // Sort by completed_at descending (most recent first)
        results.sort_by(|a, b| b.completed_at.cmp(&a.completed_at));

        // Limit the results
        results.truncate(limit);

        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_evaluation_result_success() {
        let queued_at = Utc::now();
        let result = EvaluationResult::success(
            "at://did:plc:cbkjy5n7bk3ax2wplmtjofq2/tools.graze.ifthisthenat.blueprint/123"
                .to_string(),
            queued_at,
        );

        assert!(!result.id.is_empty());
        assert_eq!(result.result, "OK");
        assert!(result.completed_at >= queued_at);
    }

    #[tokio::test]
    async fn test_evaluation_result_failure() {
        let queued_at = Utc::now();
        let result = EvaluationResult::failure(
            "at://did:plc:cbkjy5n7bk3ax2wplmtjofq2/tools.graze.ifthisthenat.blueprint/123"
                .to_string(),
            queued_at,
            "at://did:plc:cbkjy5n7bk3ax2wplmtjofq2/tools.graze.ifthisthenat.node/456",
            "Node evaluation failed: invalid configuration",
        );

        assert!(!result.id.is_empty());
        assert_eq!(
            result.result,
            "[at://did:plc:cbkjy5n7bk3ax2wplmtjofq2/tools.graze.ifthisthenat.node/456] Node evaluation failed: invalid configuration"
        );
        assert!(result.completed_at >= queued_at);
    }

    #[tokio::test]
    async fn test_noop_storage() {
        let storage = NoopEvaluationResultStorage::new();
        let queued_at = Utc::now();
        let result = EvaluationResult::success(
            "at://did:plc:cbkjy5n7bk3ax2wplmtjofq2/tools.graze.ifthisthenat.blueprint/123"
                .to_string(),
            queued_at,
        );

        // Should not error
        storage.record(result.clone()).await.unwrap();

        // Should return None
        assert!(storage.get(&result.id).await.unwrap().is_none());

        // Should return empty list
        assert!(
            storage
                .list_by_blueprint(&result.aturi, 10)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn test_tracing_storage() {
        let storage = TracingEvaluationResultStorage::new();
        let queued_at = Utc::now();

        // Test success case
        let success_result = EvaluationResult::success(
            "at://did:plc:cbkjy5n7bk3ax2wplmtjofq2/tools.graze.ifthisthenat.blueprint/123"
                .to_string(),
            queued_at,
        );
        storage.record(success_result).await.unwrap();

        // Test failure case
        let failure_result = EvaluationResult::failure(
            "at://did:plc:cbkjy5n7bk3ax2wplmtjofq2/tools.graze.ifthisthenat.blueprint/123"
                .to_string(),
            queued_at,
            "at://did:plc:cbkjy5n7bk3ax2wplmtjofq2/tools.graze.ifthisthenat.node/456",
            "Invalid configuration",
        );
        storage.record(failure_result).await.unwrap();
    }

    #[tokio::test]
    async fn test_filesystem_storage_parse_aturi() {
        let aturi = "at://did:plc:cbkjy5n7bk3ax2wplmtjofq2/tools.graze.ifthisthenat.blueprint/01K4ZD4FRHPDWPJD9TVVSNMYQ6";
        let (did, blueprint_id) = FilesystemEvaluationResultStorage::parse_aturi(aturi).unwrap();

        assert_eq!(did, "did:plc:cbkjy5n7bk3ax2wplmtjofq2");
        assert_eq!(blueprint_id, "01K4ZD4FRHPDWPJD9TVVSNMYQ6");
    }

    #[tokio::test]
    async fn test_filesystem_storage_paths() {
        let temp_dir = std::env::temp_dir().join(format!("iftta_test_{}", Ulid::new()));
        let storage = FilesystemEvaluationResultStorage::new(&temp_dir);

        let did = "did:plc:cbkjy5n7bk3ax2wplmtjofq2";
        let blueprint_id = "01K4ZD4FRHPDWPJD9TVVSNMYQ6";
        let evaluation_id = "01K51CKSQ82X3GVD576N0KB5Y0";

        let blueprint_dir = storage.get_blueprint_directory(did, blueprint_id);
        assert_eq!(blueprint_dir, temp_dir.join(did).join(blueprint_id));

        let evaluation_file = storage.get_evaluation_file_path(did, blueprint_id, evaluation_id);
        assert_eq!(
            evaluation_file,
            temp_dir
                .join(did)
                .join(blueprint_id)
                .join(format!("{}.json", evaluation_id))
        );

        // Cleanup
        let _ = tokio::fs::remove_dir_all(&temp_dir).await;
    }

    #[tokio::test]
    async fn test_filesystem_storage_record_and_get() {
        let temp_dir = std::env::temp_dir().join(format!("iftta_test_{}", Ulid::new()));
        let storage = FilesystemEvaluationResultStorage::new(&temp_dir);

        let queued_at = Utc::now();
        let mut result = EvaluationResult::success(
            "at://did:plc:cbkjy5n7bk3ax2wplmtjofq2/tools.graze.ifthisthenat.blueprint/blueprint123"
                .to_string(),
            queued_at,
        );
        // Use a fixed ID for testing
        result.id = "evaluation123".to_string();

        // Record the result
        storage.record(result.clone()).await.unwrap();

        // Verify the file was created
        let expected_path = temp_dir
            .join("did:plc:cbkjy5n7bk3ax2wplmtjofq2")
            .join("blueprint123")
            .join("evaluation123.json");
        assert!(expected_path.exists());

        // Retrieve the result
        let retrieved = storage.get("evaluation123").await.unwrap();
        assert!(retrieved.is_some());

        let retrieved = retrieved.unwrap();
        assert_eq!(retrieved.id, result.id);
        assert_eq!(retrieved.aturi, result.aturi);
        assert_eq!(retrieved.result, result.result);

        // Cleanup
        let _ = tokio::fs::remove_dir_all(&temp_dir).await;
    }

    #[tokio::test]
    async fn test_filesystem_storage_list_by_blueprint() {
        let temp_dir = std::env::temp_dir().join(format!("iftta_test_{}", Ulid::new()));
        let storage = FilesystemEvaluationResultStorage::new(&temp_dir);

        let aturi =
            "at://did:plc:abcdefghijklmnopqrstuvwx/tools.graze.ifthisthenat.blueprint/blueprint456";
        let queued_at = Utc::now();

        // Create multiple evaluation results
        for i in 0..5 {
            let mut result = if i % 2 == 0 {
                EvaluationResult::success(aturi.to_string(), queued_at)
            } else {
                EvaluationResult::failure(
                    aturi.to_string(),
                    queued_at,
                    &format!(
                        "at://did:plc:abcdefghijklmnopqrstuvwx/tools.graze.ifthisthenat.node/{}",
                        i
                    ),
                    "Test error",
                )
            };
            result.id = format!("eval{:03}", i);

            // Add a small delay to ensure different completion times
            tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

            storage.record(result).await.unwrap();
        }

        // List results with limit
        let results = storage.list_by_blueprint(aturi, 3).await.unwrap();
        assert_eq!(results.len(), 3);

        // Verify they are sorted by completed_at descending (most recent first)
        assert_eq!(results[0].id, "eval004");
        assert_eq!(results[1].id, "eval003");
        assert_eq!(results[2].id, "eval002");

        // List all results
        let all_results = storage.list_by_blueprint(aturi, 10).await.unwrap();
        assert_eq!(all_results.len(), 5);

        // Cleanup
        let _ = tokio::fs::remove_dir_all(&temp_dir).await;
    }

    #[tokio::test]
    async fn test_filesystem_storage_nonexistent() {
        let temp_dir = std::env::temp_dir().join(format!("iftta_test_{}", Ulid::new()));
        let storage = FilesystemEvaluationResultStorage::new(&temp_dir);

        // Try to get non-existent evaluation
        let result = storage.get("nonexistent").await.unwrap();
        assert!(result.is_none());

        // Try to list for non-existent blueprint
        let results = storage
            .list_by_blueprint(
                "at://did:plc:zyxwvutsrqponmlkjihgfedc/tools.graze.ifthisthenat.blueprint/nonexistent",
                10,
            )
            .await
            .unwrap();
        assert!(results.is_empty());

        // Cleanup (directory might not exist, but that's ok)
        let _ = tokio::fs::remove_dir_all(&temp_dir).await;
    }

    #[tokio::test]
    async fn test_filesystem_storage_invalid_aturi() {
        let temp_dir = std::env::temp_dir().join(format!("iftta_test_{}", Ulid::new()));
        let storage = FilesystemEvaluationResultStorage::new(&temp_dir);

        // Test with invalid AT-URI (missing prefix)
        let mut result = EvaluationResult::success("invalid-uri".to_string(), Utc::now());
        result.id = "test1".to_string();

        let err = storage.record(result).await;
        assert!(err.is_err());
        assert!(err.unwrap_err().to_string().contains("Invalid AT-URI"));

        // Test with invalid AT-URI (wrong number of parts)
        let mut result2 = EvaluationResult::success("at://invalid/test".to_string(), Utc::now());
        result2.id = "test2".to_string();

        let err2 = storage.record(result2).await;
        assert!(err2.is_err());
        assert!(err2.unwrap_err().to_string().contains("Invalid AT-URI"));

        // Cleanup (directory might not exist, but that's ok)
        let _ = tokio::fs::remove_dir_all(&temp_dir).await;
    }
}
