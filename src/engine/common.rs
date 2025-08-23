//! Common utilities and shared functionality for the blueprint engine.
//!
//! This module provides shared utilities used across all node evaluators,
//! ensuring consistent behavior and configuration throughout the system.

use crate::engine::datalogic_utils::create_datalogic_with_custom_ops;
use datalogic_rs::DataLogic;

/// Creates a standard DataLogic instance with consistent configuration and custom operators.
///
/// This function provides a centralized way to create DataLogic instances
/// used across all evaluators for uniform behavior. DataLogic is used for
/// evaluating logical expressions and data transformations in node payloads.
///
/// # Configuration
///
/// The DataLogic instance is configured with:
/// - Chunk size: 1MB (1024 * 1024 bytes)
/// - Preserve structure: Enabled to maintain JSON structure integrity
/// - Custom operators: Includes `metrohash` for string concatenation and hashing
///
/// # Custom Operators
///
/// - `metrohash`: Concatenates strings and applies MetroHash64 algorithm
/// - `facet_text`: Parses text for mentions and URLs to create AT Protocol facets
///
/// # Returns
///
/// A configured `DataLogic` instance ready for expression evaluation with custom operators.
///
/// # Example
///
/// ```rust,ignore
/// use ifthisthenat::engine::common::create_datalogic;
/// use serde_json::json;
///
/// let datalogic = create_datalogic();
///
/// // Standard operator
/// let expression = json!({
///     "==": [
///         {"val": ["status"]},
///         "active"
///     ]
/// });
/// let data = json!({"status": "active"});
///
/// match datalogic.evaluate(&expression, &data) {
///     Ok(result) => println!("Result: {}", result),
///     Err(e) => println!("Evaluation error: {}", e),
/// }
///
/// // Custom metrohash operator
/// let hash_expr = json!({
///     "metrohash": ["user", ":", {"val": ["id"]}]
/// });
/// let hash_data = json!({"id": "12345"});
///
/// match datalogic.evaluate(&hash_expr, &hash_data) {
///     Ok(result) => println!("Hash: {}", result),
///     Err(e) => println!("Error: {}", e),
/// }
/// ```
pub fn create_datalogic() -> DataLogic {
    // Use the enhanced version with custom operators
    create_datalogic_with_custom_ops()
}
