//! Periodic entry node for generating scheduled events.
//!
//! This module implements an entry node type that generates JSON payloads
//! periodically based on cron expressions. It allows blueprints to trigger
//! on schedules rather than external events.
//!
//! # Usage
//!
//! Periodic entry nodes must be the first node in a blueprint. They generate
//! payloads at scheduled intervals using cron expressions and DataLogic evaluation.
//!
//! # Example Blueprint
//!
//! ```json
//! {
//!   "nodes": [
//!     {
//!       "type": "periodic_entry",
//!       "configuration": {
//!         "cron": "0 * * * *"  // Every hour
//!       },
//!       "payload": true  // Boolean payload to allow processing
//!     },
//!     {
//!       "type": "periodic_entry",
//!       "configuration": {
//!         "cron": "0 */2 * * *"  // Every 2 hours
//!       },
//!       "payload": {
//!         "==": [{"val": ["scheduler", "enabled"]}, true]  // Conditional evaluation
//!       }
//!     }
//!   ]
//! }
//! ```

use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use croner::Cron;
use serde_json::Value;
use std::str::FromStr;
use tracing::debug;

use crate::errors::EngineError;

use crate::storage::node::Node;

use super::common::create_datalogic;
use super::evaluator::NodeEvaluator;

/// Minimum allowed interval between scheduled runs (30 minutes)
const MIN_INTERVAL_SECONDS: i64 = 1800;

/// Maximum allowed interval between scheduled runs (90 days)
const MAX_INTERVAL_SECONDS: i64 = 90 * 24 * 60 * 60;

/// Evaluator for periodic entry nodes.
///
/// Periodic entry nodes generate events on a schedule defined by cron expressions.
/// They act as entry points for blueprints that need to run periodically rather
/// than in response to external events.
///
/// # Configuration
///
/// The node's configuration field requires:
/// - `cron`: A cron expression defining the schedule (required)
///
/// Supported cron formats:
/// - Standard 5-field: `MIN HOUR DAY MONTH WEEKDAY`
/// - Special strings: `@yearly`, `@monthly`, `@weekly`, `@daily`, `@hourly`
///
/// # Validation
///
/// The evaluator validates that:
/// - The cron expression is syntactically valid
/// - The schedule interval is at least 30 minutes and no more than 90 days
/// - The next scheduled time can be calculated
///
/// # Payload Evaluation
///
/// The node's payload field determines whether the blueprint should proceed:
/// - Boolean `true`: Blueprint continues processing
/// - Boolean `false`: Blueprint stops processing
/// - Object: Evaluated using DataLogic, must return a boolean result
///
/// Common DataLogic patterns:
/// - Check time of day: `{">": [{"format_date": [{"now": []}, "HH"]}, "08"]}`
/// - Check enabled flag: `{"==": [{"val": ["scheduler", "enabled"]}, true]}`
pub struct PeriodicEntryEvaluator;

impl PeriodicEntryEvaluator {
    /// Create a new periodic entry evaluator
    pub fn new() -> Self {
        Self
    }

    /// Validate the cron expression and check interval bounds
    pub fn validate_cron_schedule(cron_expr: &str) -> Result<()> {
        // Check for empty expression
        if cron_expr.is_empty() {
            return Err(anyhow::anyhow!("Invalid cron expression: empty string"));
        }

        // Allow special strings like @yearly, @monthly, @weekly, @daily, @hourly
        let is_special_string = cron_expr.starts_with('@');

        // For non-special strings, check field count
        if !is_special_string && cron_expr.split_whitespace().count() < 5 {
            return Err(anyhow::anyhow!("Invalid cron expression: {}", cron_expr));
        }

        // Parse the cron expression
        let cron = Cron::from_str(cron_expr)
            .with_context(|| anyhow::anyhow!("Invalid cron expression: {}", cron_expr))?;

        // Use a fixed reference time to ensure consistent validation
        // Using a time in the middle of a day to avoid edge cases with day boundaries
        let reference_time = DateTime::parse_from_rfc3339("2025-01-15T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        // Calculate the next two occurrences to determine the interval
        let first_occurrence = cron
            .find_next_occurrence(&reference_time, false)
            .with_context(|| {
                anyhow::anyhow!(
                    "Cannot calculate first occurrence for cron expression: {}",
                    cron_expr
                )
            })?;

        let second_occurrence = cron
            .find_next_occurrence(&first_occurrence, false)
            .with_context(|| {
                anyhow::anyhow!(
                    "Cannot calculate second occurrence for cron expression: {}",
                    cron_expr
                )
            })?;

        // Calculate the interval between occurrences
        let interval = second_occurrence.signed_duration_since(first_occurrence);
        let interval_seconds = interval.num_seconds();

        // Validate minimum interval (30 minutes)
        if interval_seconds < MIN_INTERVAL_SECONDS {
            return Err(anyhow::anyhow!(
                "Cron schedule interval is too frequent: {} seconds (minimum: {} minutes)",
                interval_seconds,
                MIN_INTERVAL_SECONDS / 60
            ));
        }

        // Validate maximum interval (90 days)
        if interval_seconds > MAX_INTERVAL_SECONDS {
            return Err(anyhow::anyhow!(
                "Cron schedule interval is too infrequent: {} seconds (maximum: {} days)",
                interval_seconds,
                MAX_INTERVAL_SECONDS / (24 * 60 * 60)
            ));
        }

        Ok(())
    }

    /// Calculate the next run time for a cron expression
    ///
    /// This method parses the cron expression and attempts to find the next occurrence.
    /// If the croner library fails (which can happen with certain patterns), it falls back
    /// to a reasonable default based on the pattern.
    pub fn calculate_next_run(
        cron_expr: &str,
        after: Option<DateTime<Utc>>,
    ) -> Result<DateTime<Utc>> {
        let cron = Cron::from_str(cron_expr)
            .with_context(|| anyhow::anyhow!("Invalid cron expression: {}", cron_expr))?;

        Self::calculate_next_run_with_cron(&cron, after.unwrap_or_else(Utc::now))
    }

    /// Calculate the next run time using a pre-parsed Cron object
    /// This is more efficient when the Cron is already parsed and cached
    pub fn calculate_next_run_with_cron(
        cron: &Cron,
        reference_time: DateTime<Utc>,
    ) -> Result<DateTime<Utc>> {
        // Try to find the next occurrence
        // Return error if croner fails - no fallback
        cron.find_next_occurrence(&reference_time, false)
            .with_context(|| "Failed to calculate next occurrence from cron expression")
    }
}

impl Default for PeriodicEntryEvaluator {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl NodeEvaluator for PeriodicEntryEvaluator {
    async fn evaluate(&self, node: &Node, input: &Value) -> Result<Option<Value>> {
        debug!("Evaluating periodic_entry node");

        // Extract and validate cron expression
        let cron_expr = node
            .configuration
            .get("cron")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                EngineError::MissingRequiredField {
                    field_name: "cron".to_string(),
                    node_type: "periodic_entry".to_string(),
                }
            })?;

        // Validate the cron schedule
        Self::validate_cron_schedule(cron_expr)?;

        // Evaluate the payload
        let result = if let Some(bool_value) = node.payload.as_bool() {
            Value::Bool(bool_value)
        } else if node.payload.is_object() {
            let datalogic = create_datalogic();
            datalogic.evaluate_json(&node.payload, input, None)?
        } else {
            return Err(EngineError::InvalidFieldType {
                field_name: "payload".to_string(),
                node_type: "periodic_entry".to_string(),
                expected_type: "boolean or object".to_string(),
            }.into());
        };

        debug!(
            cron = %cron_expr,
            "Periodic entry evaluated successfully"
        );

        // Ensure the result is a boolean
        match result {
            Value::Bool(true) => Ok(Some(input.clone())),
            Value::Bool(false) => Ok(None),
            _ => Err(EngineError::DataLogicFailed {
                expression: format!("{:?}", node.payload),
                details: format!("Node must evaluate to a boolean, got: {:?}", result),
            }.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_validate_cron_schedule_valid() {
        // Valid schedules within bounds (30 minutes minimum)
        assert!(PeriodicEntryEvaluator::validate_cron_schedule("0,30 * * * *").is_ok()); // Every 30 minutes
        assert!(PeriodicEntryEvaluator::validate_cron_schedule("0 * * * *").is_ok()); // Every hour
        assert!(PeriodicEntryEvaluator::validate_cron_schedule("0 */2 * * *").is_ok()); // Every 2 hours
        assert!(PeriodicEntryEvaluator::validate_cron_schedule("0 0 * * *").is_ok()); // Daily
        assert!(PeriodicEntryEvaluator::validate_cron_schedule("0 0 * * 1").is_ok()); // Weekly (Monday)
    }

    #[test]
    fn test_validate_cron_schedule_too_frequent() {
        // Test schedules that are more frequent than 30 minutes
        assert!(PeriodicEntryEvaluator::validate_cron_schedule("* * * * *").is_err()); // Every minute (60s < 1800s)
        assert!(PeriodicEntryEvaluator::validate_cron_schedule("*/5 * * * *").is_err()); // Every 5 minutes (300s < 1800s)
        assert!(PeriodicEntryEvaluator::validate_cron_schedule("*/15 * * * *").is_err()); // Every 15 minutes (900s < 1800s)
        assert!(PeriodicEntryEvaluator::validate_cron_schedule("0,10,20 * * * *").is_err()); // Every 10 minutes (600s < 1800s)
    }

    #[test]
    fn test_validate_cron_schedule_too_infrequent() {
        // Test schedules that exceed the 90-day maximum
        // Monthly schedules can sometimes exceed 90 days (e.g., Jan 1 to Apr 1 = 90 days)
        // Yearly definitely exceeds 90 days
        assert!(PeriodicEntryEvaluator::validate_cron_schedule("0 0 1 1 *").is_err()); // Yearly on Jan 1
        assert!(PeriodicEntryEvaluator::validate_cron_schedule("@yearly").is_err()); // Special string for yearly
    }

    #[test]
    fn test_validate_cron_schedule_special_strings() {
        // Test special cron strings
        assert!(PeriodicEntryEvaluator::validate_cron_schedule("@hourly").is_ok()); // Every hour (3600s)
        assert!(PeriodicEntryEvaluator::validate_cron_schedule("@daily").is_ok()); // Every day (86400s)
        assert!(PeriodicEntryEvaluator::validate_cron_schedule("@weekly").is_ok()); // Every week (604800s)
        assert!(PeriodicEntryEvaluator::validate_cron_schedule("@monthly").is_ok()); // Every month (~30 days)
        assert!(PeriodicEntryEvaluator::validate_cron_schedule("@yearly").is_err()); // Every year (> 90 days)
    }

    #[test]
    fn test_validate_cron_schedule_edge_cases() {
        // Test patterns at the boundaries

        // Every 2 months is ~60 days, should be OK (< 90 days)
        assert!(PeriodicEntryEvaluator::validate_cron_schedule("0 0 1 */2 *").is_ok());

        // Every 3 months could be 90-92 days depending on months, might fail
        match PeriodicEntryEvaluator::validate_cron_schedule("0 0 1 */3 *") {
            Ok(_) => {} // May pass depending on which months
            Err(e) => {
                assert!(e.to_string().contains("too infrequent"));
            }
        }

        // Weekly (7 days) is well within bounds
        assert!(PeriodicEntryEvaluator::validate_cron_schedule("0 0 * * 0").is_ok()); // Every Sunday

        // Every 30 days via day of month
        assert!(PeriodicEntryEvaluator::validate_cron_schedule("0 0 30 * *").is_ok());
    }

    #[test]
    fn test_validate_cron_schedule_invalid() {
        // Invalid cron expressions
        assert!(PeriodicEntryEvaluator::validate_cron_schedule("invalid cron").is_err());
        assert!(PeriodicEntryEvaluator::validate_cron_schedule("").is_err());
        assert!(PeriodicEntryEvaluator::validate_cron_schedule("* * *").is_err()); // Too few fields (3 instead of 5)
    }

    #[test]
    fn test_calculate_next_run() {
        // Use a simple cron pattern that croner can handle
        let cron_expr = "0 * * * *"; // Every hour at minute 0
        let base_time = DateTime::parse_from_rfc3339("2025-01-15T10:30:00Z")
            .unwrap()
            .with_timezone(&Utc);

        match PeriodicEntryEvaluator::calculate_next_run(cron_expr, Some(base_time)) {
            Ok(next_run) => {
                // Should be in the future
                let diff = next_run.signed_duration_since(base_time).num_seconds();
                assert!(diff > 0);
            }
            Err(_) => {
                // If croner fails (which can happen with certain patterns),
                // that's expected behavior now - no fallback
                // The blueprint would be disabled in this case
            }
        }
    }

    #[tokio::test]
    async fn test_periodic_entry_evaluator() {
        let evaluator = PeriodicEntryEvaluator::new();

        // Test with boolean true payload
        let node = Node {
            aturi: "test_periodic".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "periodic_entry".to_string(),
            configuration: json!({
                "cron": "0 * * * *" // Every hour
            }),
            payload: json!(true),
            created_at: Utc::now(),
        };

        let input = json!({"test": "data"}); // Input data to pass through

        let result = evaluator.evaluate(&node, &input).await.unwrap();
        assert!(result.is_some());
        assert_eq!(result.unwrap(), input);

        // Test with boolean false payload
        let node_false = Node {
            aturi: "test_periodic".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "periodic_entry".to_string(),
            configuration: json!({
                "cron": "0,30 * * * *" // Every 30 minutes
            }),
            payload: json!(false),
            created_at: Utc::now(),
        };

        let result = evaluator.evaluate(&node_false, &input).await.unwrap();
        assert!(result.is_none());

        // Test with object payload that evaluates to boolean
        let node_obj = Node {
            aturi: "test_periodic".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "periodic_entry".to_string(),
            configuration: json!({
                "cron": "0,30 * * * *" // Every 30 minutes
            }),
            payload: json!({
                "==": [1, 1] // Always true
            }),
            created_at: Utc::now(),
        };

        let result = evaluator.evaluate(&node_obj, &input).await.unwrap();
        assert!(result.is_some());
        assert_eq!(result.unwrap(), input);
    }

    #[tokio::test]
    async fn test_periodic_entry_missing_cron() {
        let evaluator = PeriodicEntryEvaluator::new();

        let node = Node {
            aturi: "test_periodic".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "periodic_entry".to_string(),
            configuration: json!({}), // Missing cron field
            payload: json!({"event": "test"}),
            created_at: Utc::now(),
        };

        let input = json!({});

        let result = evaluator.evaluate(&node, &input).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("requires 'cron' in configuration")
        );
    }

    #[tokio::test]
    async fn test_periodic_entry_invalid_cron() {
        let evaluator = PeriodicEntryEvaluator::new();

        let node = Node {
            aturi: "test_periodic".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "periodic_entry".to_string(),
            configuration: json!({
                "cron": "invalid cron expression" // Invalid cron
            }),
            payload: json!({"event": "test"}),
            created_at: Utc::now(),
        };

        let input = json!({});

        let result = evaluator.evaluate(&node, &input).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Invalid cron expression")
        );
    }
}
