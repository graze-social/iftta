//! Common utilities and shared functionality for the blueprint engine.
//!
//! This module provides shared utilities used across all node evaluators,
//! ensuring consistent behavior and configuration throughout the system.

use crate::engine::datalogic_utils::create_datalogic;
use datalogic_rs::DataLogic;
use serde_json::Value;
use std::cell::RefCell;
use std::sync::Arc;

/// Maximum number of evaluations before rotating the cached DataLogic instance.
///
/// This provides a balance between performance (reusing instances) and freshness
/// (avoiding very long-lived instances). With datalogic-rs 4.0.4, the arena
/// allocator memory leak is fixed, so we can use a much higher threshold.
///
/// Note: Rotation is still beneficial to avoid any potential resource accumulation
/// and to periodically refresh instances, but it's no longer critical for memory.
const DATALOGIC_ROTATION_THRESHOLD: usize = 10000;

thread_local! {
    /// Thread-local cache for DataLogic instances.
    ///
    /// Stores a tuple of (DataLogic, usage_counter). When the counter reaches
    /// DATALOGIC_ROTATION_THRESHOLD, the instance is dropped and recreated.
    static DATALOGIC_CACHE_DEFAULT: RefCell<Option<(DataLogic, usize)>> = RefCell::new(None);
    static DATALOGIC_CACHE_PRESERVING: RefCell<Option<(DataLogic, usize)>> = RefCell::new(None);
}

/// Gets a cached DataLogic instance for the current thread.
///
/// This function implements thread-local caching with periodic instance rotation
/// for optimal performance. Each thread maintains its own DataLogic instance,
/// which is reused across multiple evaluations.
///
/// # Performance Benefits
///
/// This approach provides significant performance benefits:
/// - Avoids creating thousands of DataLogic instances per second
/// - Avoids repeated custom operator registration overhead
/// - Reduces memory allocator pressure
/// - Eliminates compilation overhead for the same logic patterns
///
/// # Configuration
///
/// The DataLogic instance is configured with:
/// - Preserve structure: Enabled to maintain JSON structure integrity
/// - Custom operators: metrohash, facet_text, parse_aturi
///
/// # Memory (v4.0.4)
///
/// With datalogic-rs 4.0.4, the arena allocator memory leak is fixed!
/// Instances can now be cached for much longer without memory concerns.
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
/// let expression = json!({"==": [{"var": "status"}, "active"]});
/// let data = json!({"status": "active"});
///
/// let result = with_cached_datalogic(|datalogic| {
///     let compiled = datalogic.compile(&expression)?;
///     datalogic.evaluate_owned(&compiled, data)
/// })?;
/// ```
pub fn with_cached_datalogic<F, R>(preserve: bool, f: F) -> R
where
    F: FnOnce(&DataLogic) -> R,
{
    if preserve {
        DATALOGIC_CACHE_PRESERVING.with(|cache| {
            let mut cache_ref = cache.borrow_mut();

            // Check if we need to create or rotate the instance
            let should_rotate = cache_ref
                .as_ref()
                .map(|(_, count)| *count >= DATALOGIC_ROTATION_THRESHOLD)
                .unwrap_or(true);

            if should_rotate {
                // Drop old instance (if any) and create a new one
                *cache_ref = Some((create_datalogic(true), 0));
            }

            // Get the instance and increment counter
            let (datalogic, count) = cache_ref.as_mut().unwrap();
            *count += 1;

            // Execute the closure with the cached instance
            f(datalogic)
        })
    } else {
        DATALOGIC_CACHE_DEFAULT.with(|cache| {
            let mut cache_ref = cache.borrow_mut();

            // Check if we need to create or rotate the instance
            let should_rotate = cache_ref
                .as_ref()
                .map(|(_, count)| *count >= DATALOGIC_ROTATION_THRESHOLD)
                .unwrap_or(true);

            if should_rotate {
                // Drop old instance (if any) and create a new one
                *cache_ref = Some((create_datalogic(false), 0));
            }

            // Get the instance and increment counter
            let (datalogic, count) = cache_ref.as_mut().unwrap();
            *count += 1;

            // Execute the closure with the cached instance
            f(datalogic)
        })
    }
}

/// Helper function to evaluate JSON expressions using the cached DataLogic instance.
///
/// This is a convenience wrapper that handles the compile+evaluate pattern
/// required by datalogic-rs 4.0.4. It compiles the logic expression and evaluates
/// it against the provided data.
///
/// # Arguments
///
/// * `logic` - The JSON logic expression to evaluate
/// * `data` - The data to evaluate against
///
/// # Returns
///
/// The evaluation result as a JSON Value, or an error
///
/// # Example
///
/// ```rust,ignore
/// let expression = json!({"==": [{"var": "status"}, "active"]});
/// let data = json!({"status": "active"});
/// let result = evaluate_json_logic(&expression, &data)?;
/// ```
pub fn evaluate_json_logic(preserve: bool, logic: &Value, data: &Value) -> anyhow::Result<Value> {
    with_cached_datalogic(preserve, |datalogic| {
        let compiled = datalogic
            .compile(logic)
            .map_err(|e| anyhow::anyhow!("Compilation failed: {}", e))?;
        datalogic
            .evaluate(&compiled, Arc::new(data.clone()))
            .map_err(|e| anyhow::anyhow!("Evaluation failed: {}", e))
    })
}

/// Process a template object that may contain DataLogic expressions.
///
/// This function implements template processing for datalogic-rs 4.0.4, which allows
/// custom operators to work while still supporting template-style objects. It recursively
/// processes values, evaluating DataLogic expressions and preserving literals.
///
/// # Template Processing Rules
///
/// - **Objects with single key**: Attempted to compile as DataLogic expression
///   - If compilation succeeds: Evaluated as expression
///   - If compilation fails: Treated as template object
/// - **Objects with multiple keys**: Treated as template, each field processed recursively
/// - **Arrays**: Each element processed recursively
/// - **Literals** (string, number, bool, null): Returned unchanged
///
/// # Arguments
///
/// * `template` - The template value to process
/// * `data` - The data context for evaluation
///
/// # Returns
///
/// The processed value with expressions evaluated, or an error
///
/// # Example
///
/// ```rust,ignore
/// let template = json!({
///     "name": "result",
///     "value": {"val": ["field"]},
///     "doubled": {"*": [{"val": ["num"]}, 2]}
/// });
/// let data = json!({"field": "test", "num": 5});
/// let result = evaluate_template(&template, &data)?;
/// // result = {"name": "result", "value": "test", "doubled": 10}
/// ```
pub fn evaluate_template(preserve: bool, template: &Value, data: &Value) -> anyhow::Result<Value> {
    with_cached_datalogic(preserve, |datalogic| process_template_recursive(template, data, datalogic))
}

/// Check if a key looks like a DataLogic operator.
fn is_likely_operator(key: &str) -> bool {
    // Standard DataLogic operators
    const STANDARD_OPERATORS: &[&str] = &[
        "val",
        "var", // Variable access
        "==",
        "!=",
        "<",
        ">",
        "<=",
        ">=",
        "===",
        "!==", // Comparisons
        "+",
        "-",
        "*",
        "/",
        "%", // Arithmetic
        "and",
        "or",
        "not",
        "!!",  // Boolean logic
        "if",  // Conditionals
        "cat", // String operations
        "in",
        "contains", // Containment
        "map",
        "filter",
        "reduce",
        "all",
        "some",
        "none",  // Array operations
        "merge", // Object operations
        "missing",
        "missing_some", // Validation
        "now",
        "datetime", // Time operations
    ];

    // Custom operators we've registered
    const CUSTOM_OPERATORS: &[&str] = &["metrohash", "facet_text", "parse_aturi"];

    STANDARD_OPERATORS.contains(&key) || CUSTOM_OPERATORS.contains(&key)
}

/// Internal recursive function for template processing.
fn process_template_recursive(
    template: &Value,
    data: &Value,
    datalogic: &DataLogic,
) -> anyhow::Result<Value> {
    use serde_json::Map;

    match template {
        Value::Object(obj) => {
            // If it's an object with a single key that looks like an operator, try as expression
            if obj.len() == 1 {
                let key = obj.keys().next().unwrap();

                // Check if this looks like a DataLogic operator
                if is_likely_operator(key) {
                    // Try to compile and evaluate as expression
                    // We use evaluate_json() which is the high-level API that handles everything
                    let logic_str = serde_json::to_string(template)
                        .map_err(|e| anyhow::anyhow!("Failed to serialize template: {}", e))?;
                    let data_str = serde_json::to_string(data)
                        .map_err(|e| anyhow::anyhow!("Failed to serialize data: {}", e))?;

                    match datalogic.evaluate_json(&logic_str, &data_str) {
                        Ok(result) => return Ok(result),
                        Err(_e) => {
                            // Evaluation failed - this might be a literal object that happens
                            // to have an operator-like key. Fall through to template processing.
                        }
                    }
                }
                // Either not an operator, or evaluation failed - treat as template
            }

            // Process as template object - recursively process each field
            let mut result = Map::new();
            for (key, value) in obj {
                // IMPORTANT: Only recursively process if the value isn't itself an operator expression
                // Otherwise we'll pre-evaluate arguments before the parent operator runs
                let processed = process_template_recursive(value, data, datalogic)?;
                result.insert(key.clone(), processed);
            }
            Ok(Value::Object(result))
        }
        Value::Array(arr) => {
            // Process each array element
            let processed: Result<Vec<Value>, _> = arr
                .iter()
                .map(|item| process_template_recursive(item, data, datalogic))
                .collect();
            Ok(Value::Array(processed?))
        }
        // Literals pass through unchanged
        Value::String(_) | Value::Number(_) | Value::Bool(_) | Value::Null => Ok(template.clone()),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    #[test]
    fn test_if_operator_works_with_datalogic_new() {
        let datalogic = crate::engine::datalogic_utils::create_datalogic(true);

        let logic = json!({"if": [true, "yes", "no"]});
        let data = json!({});

        let logic_str = serde_json::to_string(&logic).unwrap();
        let data_str = serde_json::to_string(&data).unwrap();

        let result = datalogic.evaluate_json(&logic_str, &data_str);
        eprintln!("test_if_operator result: {:?}", result);
        assert!(result.is_ok(), "If operator should work");
        assert_eq!(result.unwrap(), json!("yes"));
    }

    #[test]
    fn test_if_operator_with_objects() {
        let datalogic = crate::engine::datalogic_utils::create_datalogic(true);

        let logic = json!({
            "if": [
                {"==": [{"val": ["active"]}, true]},
                {"code": "ACTIVE", "message": "User is active"},
                {"code": "INACTIVE", "message": "User is inactive"}
            ]
        });
        let data = json!({"active": true});

        let logic_str = serde_json::to_string(&logic).unwrap();
        let data_str = serde_json::to_string(&data).unwrap();

        let result = datalogic.evaluate_json(&logic_str, &data_str);
        eprintln!("test_if_operator_with_objects result: {:?}", result);
        assert!(
            result.is_ok(),
            "If operator with objects should work: {:?}",
            result
        );
    }
}
