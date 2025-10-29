//! Common utilities and shared functionality for the blueprint engine.
//!
//! This module provides shared utilities used across all node evaluators,
//! ensuring consistent behavior and configuration throughout the system.

use crate::engine::datalogic_utils::create_datalogic_with_custom_ops;
use datalogic_rs::DataLogic;
use std::cell::RefCell;

/// Maximum number of evaluations before rotating the cached DataLogic instance.
///
/// This prevents unbounded memory growth in the DataLogic arena allocator.
/// After this many uses, the instance is dropped and a new one is created,
/// clearing accumulated arena memory.
const DATALOGIC_ROTATION_THRESHOLD: usize = 1000;

thread_local! {
    /// Thread-local cache for DataLogic instances.
    ///
    /// Stores a tuple of (DataLogic, usage_counter). When the counter reaches
    /// DATALOGIC_ROTATION_THRESHOLD, the instance is dropped and recreated.
    static DATALOGIC_CACHE: RefCell<Option<(DataLogic, usize)>> = RefCell::new(None);
}

/// Gets a cached DataLogic instance for the current thread.
///
/// This function implements thread-local caching with periodic instance rotation
/// to prevent memory leaks from the DataLogic bump allocator. Each thread maintains
/// its own DataLogic instance, which is reused across multiple evaluations for
/// performance.
///
/// # Memory Management
///
/// The DataLogic arena allocator accumulates memory that may not be properly freed
/// on drop (upstream issue in datalogic-rs). To mitigate this:
///
/// - One DataLogic instance per thread (instead of one per evaluation)
/// - Instances are rotated after DATALOGIC_ROTATION_THRESHOLD uses
/// - Old instances are dropped, clearing their accumulated arena memory
///
/// # Performance
///
/// This approach provides significant performance benefits:
/// - Avoids creating thousands of DataLogic instances per second
/// - Avoids repeated custom operator registration overhead
/// - Reduces memory allocator pressure
///
/// # Configuration
///
/// The DataLogic instance is configured with:
/// - Chunk size: 1MB (1024 * 1024 bytes)
/// - Preserve structure: Enabled to maintain JSON structure integrity
/// - Custom operators: metrohash, facet_text, parse_aturi
///
/// # Returns
///
/// A reference to the thread-local DataLogic instance, valid for the duration
/// of the closure execution.
///
/// # Example
///
/// ```rust,ignore
/// use ifthisthenat::engine::common::with_cached_datalogic;
/// use serde_json::json;
///
/// let expression = json!({"==": [{"val": ["status"]}, "active"]});
/// let data = json!({"status": "active"});
///
/// let result = with_cached_datalogic(|datalogic| {
///     datalogic.evaluate_json(&expression, &data, None)
/// })?;
/// ```
pub fn with_cached_datalogic<F, R>(f: F) -> R
where
    F: FnOnce(&DataLogic) -> R,
{
    DATALOGIC_CACHE.with(|cache| {
        let mut cache_ref = cache.borrow_mut();

        // Check if we need to create or rotate the instance
        let should_rotate = cache_ref
            .as_ref()
            .map(|(_, count)| *count >= DATALOGIC_ROTATION_THRESHOLD)
            .unwrap_or(true);

        if should_rotate {
            // Drop old instance (if any) and create a new one
            *cache_ref = Some((create_datalogic_with_custom_ops(), 0));
        }

        // Get the instance and increment counter
        let (datalogic, count) = cache_ref.as_mut().unwrap();
        *count += 1;

        // Execute the closure with the cached instance
        f(datalogic)
    })
}

/// Creates a standard DataLogic instance with consistent configuration and custom operators.
///
/// **DEPRECATED**: This function creates a new DataLogic instance on every call,
/// which causes memory leaks due to the bump allocator not properly freeing memory.
/// Use `with_cached_datalogic` instead for better performance and reduced memory usage.
///
/// This function is kept for backwards compatibility and testing purposes only.
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
/// - `parse_aturi`: Parses AT-URI strings into components
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
#[deprecated(
    since = "1.0.0-rc.1",
    note = "Use with_cached_datalogic instead to avoid memory leaks"
)]
pub fn create_datalogic() -> DataLogic {
    // Use the enhanced version with custom operators
    create_datalogic_with_custom_ops()
}
