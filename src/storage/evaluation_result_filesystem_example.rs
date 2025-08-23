//! Example of using the FilesystemEvaluationResultStorage
//! 
//! This example shows how to configure and use the filesystem-based storage
//! for blueprint evaluation results.

use anyhow::Result;
use chrono::Utc;
use std::sync::Arc;
use ifthisthenat::storage::evaluation_result::{
    EvaluationResult, EvaluationResultStorage, FilesystemEvaluationResultStorage,
};

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt::init();

    // Create the filesystem storage with a base directory
    let base_directory = "/var/log/iftta/logs";
    let storage: Arc<dyn EvaluationResultStorage> = Arc::new(
        FilesystemEvaluationResultStorage::new(base_directory)
    );

    // Example 1: Record a successful evaluation
    let blueprint_aturi = "at://did:plc:cbkjy5n7bk3ax2wplmtjofq2/tools.graze.ifthisthenat.blueprint/01K4ZD4FRHPDWPJD9TVVSNMYQ6";
    let queued_at = Utc::now();
    
    let success_result = EvaluationResult::success(
        blueprint_aturi.to_string(),
        queued_at,
    );
    
    // This will create the file at:
    // /var/log/iftta/logs/did:plc:cbkjy5n7bk3ax2wplmtjofq2/01K4ZD4FRHPDWPJD9TVVSNMYQ6/{evaluation_id}.json
    storage.record(success_result.clone()).await?;
    println!("Recorded successful evaluation: {}", success_result.id);
    
    // Example 2: Record a failed evaluation
    let failure_result = EvaluationResult::failure(
        blueprint_aturi.to_string(),
        queued_at,
        "at://did:plc:cbkjy5n7bk3ax2wplmtjofq2/node/failing-node",
        "Node configuration invalid: missing required field 'collection'",
    );
    
    storage.record(failure_result.clone()).await?;
    println!("Recorded failed evaluation: {}", failure_result.id);
    
    // Example 3: Retrieve an evaluation by ID
    if let Some(retrieved) = storage.get(&success_result.id).await? {
        println!("Retrieved evaluation: {:?}", retrieved);
    }
    
    // Example 4: List recent evaluations for a blueprint
    let recent_results = storage.list_by_blueprint(blueprint_aturi, 5).await?;
    println!("Found {} recent evaluations for blueprint", recent_results.len());
    
    for result in recent_results {
        let duration = (result.completed_at - result.queued_at).num_milliseconds();
        println!(
            "  - {} | {} | {} ms",
            result.id,
            if result.result == "OK" { "SUCCESS" } else { "FAILURE" },
            duration
        );
    }
    
    Ok(())
}