//! Example demonstrating the benefits of using croner with serde feature
//!
//! Run with: cargo run --example cron_serde_example

use chrono::Utc;
use croner::Cron;
use serde::{Deserialize, Serialize};
use std::str::FromStr;

/// Configuration that can be serialized/deserialized
#[derive(Serialize, Deserialize, Debug)]
struct ScheduledTask {
    name: String,
    cron_expression: String,
    #[serde(skip)]
    cached_cron: Option<Cron>,
}

impl ScheduledTask {
    /// Initialize the cached Cron from the expression
    fn init_cron(&mut self) -> Result<(), String> {
        self.cached_cron = Some(
            Cron::from_str(&self.cron_expression).map_err(|e| format!("Invalid cron: {}", e))?,
        );
        Ok(())
    }

    /// Get the next scheduled time using the cached Cron
    fn next_run(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        self.cached_cron
            .as_ref()?
            .find_next_occurrence(&Utc::now(), false)
            .ok()
    }
}

fn main() {
    println!("Croner with serde feature example\n");

    // Create a scheduled task
    let mut task = ScheduledTask {
        name: "Daily Report".to_string(),
        cron_expression: "0 9 * * *".to_string(), // Daily at 9 AM
        cached_cron: None,
    };

    // Initialize the cached Cron object
    task.init_cron().expect("Failed to parse cron");

    // Serialize to JSON (note: cached_cron is skipped)
    let json = serde_json::to_string_pretty(&task).unwrap();
    println!("Serialized task:\n{}\n", json);

    // Deserialize from JSON
    let mut restored: ScheduledTask = serde_json::from_str(&json).unwrap();
    println!("Deserialized task: {:?}\n", restored);

    // Re-initialize the cached Cron after deserialization
    restored.init_cron().expect("Failed to parse cron");

    // Use the cached Cron for efficient evaluation
    if let Some(next) = restored.next_run() {
        println!("Next scheduled run: {}", next);
    }

    // Benefits of this approach:
    println!("\nBenefits of using serde with croner:");
    println!("1. Cron expressions can be stored as strings in config/database");
    println!("2. Parsed Cron objects can be cached for performance");
    println!("3. No repeated parsing overhead during runtime");
    println!("4. Clean separation between storage format and runtime representation");
}
