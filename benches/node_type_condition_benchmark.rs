// Benchmark for the condition node type.
// This benchmark tests various condition evaluation scenarios including
// boolean payloads, DataLogic expressions, and complex nested conditions.

use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};
use ifthisthenat::engine::evaluator::NodeEvaluator;
use ifthisthenat::engine::node_type_condition::ConditionEvaluator;
use ifthisthenat::storage::node::Node;
use serde_json::{Value, json};
use std::fs;
use std::path::Path;
use tokio::runtime::Runtime;

// Load test case from data directory
fn load_test_case(filename: &str) -> (Value, Value, Value, Value) {
    let path = Path::new("benches/data").join(filename);
    let contents =
        fs::read_to_string(&path).expect(&format!("Failed to read test case: {}", filename));
    let data: Value =
        serde_json::from_str(&contents).expect(&format!("Failed to parse test case: {}", filename));

    let input = data.get("input").expect("Test case missing input").clone();
    let output = data
        .get("output")
        .expect("Test case missing output")
        .clone();
    let payload = data
        .get("payload")
        .expect("Test case missing payload")
        .clone();
    let configuration = data.get("configuration").unwrap_or(&json!({})).clone();

    (input, output, payload, configuration)
}

// Benchmark simple boolean conditions
fn bench_boolean_conditions(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let evaluator = ConditionEvaluator::new();

    let mut group = c.benchmark_group("condition_boolean");
    group.sample_size(100);

    // Test with boolean true
    let node_true = Node {
        aturi: "test_node".to_string(),
        blueprint: "test_blueprint".to_string(),
        node_type: "condition".to_string(),
        payload: json!(true),
        configuration: json!({}),
        created_at: chrono::Utc::now(),
    };

    // Test with boolean false
    let node_false = Node {
        aturi: "test_node".to_string(),
        blueprint: "test_blueprint".to_string(),
        node_type: "condition".to_string(),
        payload: json!(false),
        configuration: json!({}),
        created_at: chrono::Utc::now(),
    };

    let input = json!({
        "status": "active",
        "count": 42,
        "user": "test"
    });

    group.bench_function(BenchmarkId::from_parameter("true"), |b| {
        b.iter(|| {
            rt.block_on(async {
                let _ = evaluator
                    .evaluate(black_box(&node_true), black_box(&input))
                    .await;
            });
        });
    });

    group.bench_function(BenchmarkId::from_parameter("false"), |b| {
        b.iter(|| {
            rt.block_on(async {
                let _ = evaluator
                    .evaluate(black_box(&node_false), black_box(&input))
                    .await;
            });
        });
    });

    group.finish();
}

// Benchmark simple DataLogic expressions
fn bench_simple_expressions(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let evaluator = ConditionEvaluator::new();

    let mut group = c.benchmark_group("condition_simple_expressions");
    group.sample_size(100);

    let test_cases = vec![
        ("equality", json!({"==": [{"val": ["status"]}, "active"]})),
        ("greater_than", json!({">": [{"val": ["count"]}, 10]})),
        ("less_than", json!({"<": [{"val": ["count"]}, 100]})),
        (
            "contains",
            json!({"contains": [{"val": ["message"]}, "test"]}),
        ),
        ("exists", json!({"exists": [{"val": ["status"]}]})),
    ];

    let input = json!({
        "status": "active",
        "count": 42,
        "message": "This is a test message"
    });

    for (name, payload) in test_cases {
        let node = Node {
            aturi: "test_node".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "condition".to_string(),
            payload,
            configuration: json!({}),
            created_at: chrono::Utc::now(),
        };

        group.bench_function(BenchmarkId::from_parameter(name), |b| {
            b.iter(|| {
                rt.block_on(async {
                    let _ = evaluator
                        .evaluate(black_box(&node), black_box(&input))
                        .await;
                });
            });
        });
    }

    group.finish();
}

// Benchmark complex nested conditions
fn bench_complex_conditions(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let evaluator = ConditionEvaluator::new();

    let mut group = c.benchmark_group("condition_complex");
    group.sample_size(100);

    // Simple AND condition
    let and_condition = json!({
        "and": [
            {"==": [{"val": ["status"]}, "active"]},
            {">": [{"val": ["count"]}, 10]}
        ]
    });

    // Simple OR condition
    let or_condition = json!({
        "or": [
            {"==": [{"val": ["status"]}, "active"]},
            {"==": [{"val": ["status"]}, "pending"]}
        ]
    });

    // Nested AND/OR condition
    let nested_condition = json!({
        "and": [
            {"==": [{"val": ["type"]}, "post"]},
            {">": [{"val": ["score"]}, 10]},
            {"or": [
                {"==": [{"val": ["author"]}, "alice"]},
                {"==": [{"val": ["author"]}, "bob"]}
            ]}
        ]
    });

    // Very complex nested condition
    let complex_condition = json!({
        "or": [
            {"and": [
                {"==": [{"val": ["priority"]}, "high"]},
                {">": [{"val": ["score"]}, 90]}
            ]},
            {"and": [
                {"==": [{"val": ["priority"]}, "medium"]},
                {">": [{"val": ["score"]}, 75]},
                {"<": [{"val": ["score"]}, 90]}
            ]},
            {"and": [
                {"==": [{"val": ["priority"]}, "low"]},
                {"contains": [{"val": ["tags"]}, "urgent"]}
            ]}
        ]
    });

    let input = json!({
        "type": "post",
        "status": "active",
        "priority": "high",
        "count": 42,
        "score": 95,
        "author": "alice",
        "tags": ["review", "urgent"]
    });

    for (name, payload) in &[
        ("and", and_condition),
        ("or", or_condition),
        ("nested", nested_condition),
        ("complex", complex_condition),
    ] {
        let node = Node {
            aturi: "test_node".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "condition".to_string(),
            payload: payload.clone(),
            configuration: json!({}),
            created_at: chrono::Utc::now(),
        };

        group.bench_function(BenchmarkId::from_parameter(name), |b| {
            b.iter(|| {
                rt.block_on(async {
                    let _ = evaluator
                        .evaluate(black_box(&node), black_box(&input))
                        .await;
                });
            });
        });
    }

    group.finish();
}

// Benchmark with different input sizes
fn bench_input_sizes(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let evaluator = ConditionEvaluator::new();

    let mut group = c.benchmark_group("condition_input_sizes");
    group.sample_size(100);

    // Create condition that checks nested fields
    let node = Node {
        aturi: "test_node".to_string(),
        blueprint: "test_blueprint".to_string(),
        node_type: "condition".to_string(),
        payload: json!({
            "and": [
                {"==": [{"val": ["user", "status"]}, "active"]},
                {">": [{"val": ["metrics", "score"]}, 50]}
            ]
        }),
        configuration: json!({}),
        created_at: chrono::Utc::now(),
    };

    // Small input
    let small_input = json!({
        "user": {"status": "active"},
        "metrics": {"score": 75}
    });

    // Medium input with more fields
    let medium_input = json!({
        "user": {
            "id": 123,
            "status": "active",
            "name": "Test User",
            "email": "test@example.com"
        },
        "metrics": {
            "score": 75,
            "views": 1000,
            "likes": 50,
            "shares": 20
        },
        "metadata": {
            "created": "2024-01-01",
            "updated": "2024-01-02",
            "tags": ["test", "benchmark"]
        }
    });

    // Large input with many fields
    let mut large_obj = serde_json::Map::new();
    large_obj.insert("user".to_string(), json!({"status": "active", "id": 123}));
    large_obj.insert("metrics".to_string(), json!({"score": 75}));
    for i in 0..100 {
        large_obj.insert(
            format!("field_{}", i),
            json!({
                "value": i,
                "description": format!("Field number {}", i),
                "nested": {
                    "data": format!("Data for field {}", i)
                }
            }),
        );
    }
    let large_input = json!(large_obj);

    for (name, input) in &[
        ("small", small_input),
        ("medium", medium_input),
        ("large", large_input),
    ] {
        group.bench_function(BenchmarkId::from_parameter(name), |b| {
            b.iter(|| {
                rt.block_on(async {
                    let _ = evaluator
                        .evaluate(black_box(&node), black_box(&input))
                        .await;
                });
            });
        });
    }

    group.finish();
}

// Benchmark with data files
fn bench_data_files(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let evaluator = ConditionEvaluator::new();

    let mut group = c.benchmark_group("condition_data_files");
    group.sample_size(100);

    let test_files = vec![
        "condition_simple.json",
        "condition_complex.json",
        "condition_nested_fields.json",
        "condition_array_operations.json",
    ];

    for filename in test_files {
        // Skip if file doesn't exist yet
        let path = Path::new("benches/data").join(&filename);
        if !path.exists() {
            continue;
        }

        let (input, _output, payload, configuration) = load_test_case(&filename);

        let node = Node {
            aturi: "test_node".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "condition".to_string(),
            payload,
            configuration,
            created_at: chrono::Utc::now(),
        };

        let test_name = filename.trim_end_matches(".json");
        group.bench_function(BenchmarkId::from_parameter(test_name), |b| {
            b.iter(|| {
                rt.block_on(async {
                    let _ = evaluator
                        .evaluate(black_box(&node), black_box(&input))
                        .await;
                });
            });
        });
    }

    group.finish();
}

// Benchmark range operations
fn bench_range_operations(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let evaluator = ConditionEvaluator::new();

    let mut group = c.benchmark_group("condition_range_operations");
    group.sample_size(100);

    let test_cases = vec![
        (
            "single_range",
            json!({
                "and": [
                    {">=": [{"val": ["value"]}, 0]},
                    {"<=": [{"val": ["value"]}, 100]}
                ]
            }),
        ),
        (
            "multiple_ranges",
            json!({
                "and": [
                    {">=": [{"val": ["score"]}, 0]},
                    {"<=": [{"val": ["score"]}, 100]},
                    {">": [{"val": ["count"]}, 10]},
                    {"<": [{"val": ["count"]}, 1000]}
                ]
            }),
        ),
        (
            "nested_ranges",
            json!({
                "or": [
                    {"and": [
                        {">": [{"val": ["metrics", "score"]}, 90]},
                        {"<=": [{"val": ["metrics", "score"]}, 100]}
                    ]},
                    {"and": [
                        {">": [{"val": ["metrics", "score"]}, 70]},
                        {"<=": [{"val": ["metrics", "score"]}, 90]}
                    ]}
                ]
            }),
        ),
    ];

    let input = json!({
        "value": 75,
        "score": 85,
        "count": 250,
        "metrics": {
            "score": 92,
            "percentile": 88
        }
    });

    for (name, payload) in test_cases {
        let node = Node {
            aturi: "test_node".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "condition".to_string(),
            payload,
            configuration: json!({}),
            created_at: chrono::Utc::now(),
        };

        group.bench_function(BenchmarkId::from_parameter(name), |b| {
            b.iter(|| {
                rt.block_on(async {
                    let _ = evaluator
                        .evaluate(black_box(&node), black_box(&input))
                        .await;
                });
            });
        });
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_boolean_conditions,
    bench_simple_expressions,
    bench_complex_conditions,
    bench_input_sizes,
    bench_data_files,
    bench_range_operations
);
criterion_main!(benches);
