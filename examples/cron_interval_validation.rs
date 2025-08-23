//! Example demonstrating cron interval validation for periodic_entry nodes
//!
//! Run with: cargo run --example cron_interval_validation

use chrono::Utc;
use croner::Cron;
use std::str::FromStr;

const MIN_INTERVAL_SECONDS: i64 = 30;
const MAX_INTERVAL_SECONDS: i64 = 90 * 24 * 60 * 60; // 90 days

fn validate_and_show_interval(cron_expr: &str) {
    println!("\nTesting: \"{}\"", cron_expr);

    match Cron::from_str(cron_expr) {
        Ok(cron) => {
            // Use a fixed reference time for consistent results
            let reference = chrono::DateTime::parse_from_rfc3339("2025-01-15T12:00:00Z")
                .unwrap()
                .with_timezone(&Utc);

            match cron.find_next_occurrence(&reference, false) {
                Ok(first) => match cron.find_next_occurrence(&first, false) {
                    Ok(second) => {
                        let interval = second.signed_duration_since(first);
                        let seconds = interval.num_seconds();
                        let days = seconds / (24 * 60 * 60);
                        let hours = (seconds % (24 * 60 * 60)) / 3600;
                        let minutes = (seconds % 3600) / 60;

                        println!("  ✓ Parsed successfully");
                        println!(
                            "  Interval: {} days, {} hours, {} minutes ({} seconds)",
                            days, hours, minutes, seconds
                        );

                        if seconds < MIN_INTERVAL_SECONDS {
                            println!(
                                "  ❌ TOO FREQUENT: Minimum is {} seconds",
                                MIN_INTERVAL_SECONDS
                            );
                        } else if seconds > MAX_INTERVAL_SECONDS {
                            println!(
                                "  ❌ TOO INFREQUENT: Maximum is {} days",
                                MAX_INTERVAL_SECONDS / (24 * 60 * 60)
                            );
                        } else {
                            println!("  ✅ VALID: Within allowed bounds (30s - 90 days)");
                        }
                    }
                    Err(e) => println!("  ✗ Cannot calculate second occurrence: {}", e),
                },
                Err(e) => println!("  ✗ Cannot calculate first occurrence: {}", e),
            }
        }
        Err(e) => println!("  ✗ Invalid cron expression: {}", e),
    }
}

fn main() {
    println!("Cron Interval Validation Example");
    println!("=================================");
    println!("Minimum interval: {} seconds", MIN_INTERVAL_SECONDS);
    println!(
        "Maximum interval: {} days",
        MAX_INTERVAL_SECONDS / (24 * 60 * 60)
    );

    // Valid patterns
    println!("\n--- VALID PATTERNS ---");
    validate_and_show_interval("* * * * *"); // Every minute
    validate_and_show_interval("*/5 * * * *"); // Every 5 minutes
    validate_and_show_interval("0 * * * *"); // Every hour
    validate_and_show_interval("0 0 * * *"); // Daily
    validate_and_show_interval("0 0 * * 0"); // Weekly (Sunday)
    validate_and_show_interval("0 0 1,15 * *"); // Twice monthly
    validate_and_show_interval("@hourly"); // Special string: hourly
    validate_and_show_interval("@daily"); // Special string: daily
    validate_and_show_interval("@weekly"); // Special string: weekly
    validate_and_show_interval("@monthly"); // Special string: monthly

    // Invalid patterns (too infrequent)
    println!("\n--- INVALID PATTERNS (TOO INFREQUENT) ---");
    validate_and_show_interval("0 0 1 */4 *"); // Every 4 months
    validate_and_show_interval("0 0 1 1 *"); // Yearly (Jan 1)
    validate_and_show_interval("@yearly"); // Special string: yearly

    // Edge cases
    println!("\n--- EDGE CASES ---");
    validate_and_show_interval("0 0 1 */2 *"); // Every 2 months (~60 days)
    validate_and_show_interval("0 0 1 */3 *"); // Every 3 months (~90 days)

    println!("\n✓ Validation complete!");
}
