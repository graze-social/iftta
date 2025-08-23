// Benchmark for the sentiment analysis node type.
// Note: This benchmark requires the sentiment analysis model to be downloaded
// from Hugging Face on first run, which may take some time.
// The model will be cached after the first download.

use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};
use ifthisthenat::engine::evaluator::NodeEvaluator;
use ifthisthenat::engine::node_type_sentiment_analysis::{SentimentAnalysisEvaluator, get_model};
use ifthisthenat::storage::node::Node;
use serde_json::json;
use std::sync::Once;
use tokio::runtime::Runtime;

// Ensure model is loaded only once before benchmarks
static INIT: Once = Once::new();

fn init_model() {
    INIT.call_once(|| {
        let rt = Runtime::new().unwrap();
        rt.block_on(async {
            println!("Pre-loading sentiment analysis model...");
            match get_model().await {
                Ok(_) => println!("Model loaded successfully."),
                Err(e) => eprintln!("Warning: Failed to pre-load model: {}. Benchmarks may include model loading time.", e),
            }
        });
    });
}

// Create test data with different text sizes
fn create_short_text() -> String {
    "I love this amazing product! It makes me so happy!".to_string()
}

fn create_medium_text() -> String {
    "This is a longer piece of text that contains multiple sentences. \
     I really enjoy using this service because it provides great value. \
     The customer support team is fantastic and always helpful. \
     Overall, I'm extremely satisfied with my experience and would \
     highly recommend it to others. The features are intuitive and \
     the performance is outstanding."
        .to_string()
}

fn create_long_text() -> String {
    // Create a text that will be truncated (>2000 chars)
    let base = "This is a very detailed review of the product. ";
    base.repeat(100) // ~4700 chars
}

// Benchmark the sentiment analysis with string payload (field extraction)
fn bench_string_payload(c: &mut Criterion) {
    init_model();
    let rt = Runtime::new().unwrap();
    let evaluator = SentimentAnalysisEvaluator::new();

    let mut group = c.benchmark_group("sentiment_analysis_string_payload");
    // Configure for ML workloads - fewer samples with more time
    group.sample_size(50);
    group.warm_up_time(std::time::Duration::from_secs(10));
    group.measurement_time(std::time::Duration::from_secs(60));

    for (name, text) in &[
        ("short", create_short_text()),
        ("medium", create_medium_text()),
        ("long", create_long_text()),
    ] {
        let node = Node {
            aturi: "test_node".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "sentiment_analysis".to_string(),
            payload: json!("message"),
            configuration: json!({}),
            created_at: chrono::Utc::now(),
        };

        let input = json!({
            "message": text,
            "user": "test_user",
            "timestamp": "2024-01-01T00:00:00Z"
        });

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

// Benchmark the sentiment analysis with object payload (DataLogic evaluation)
fn bench_object_payload(c: &mut Criterion) {
    init_model();
    let rt = Runtime::new().unwrap();
    let evaluator = SentimentAnalysisEvaluator::new();

    let mut group = c.benchmark_group("sentiment_analysis_object_payload");
    // Configure for ML workloads - fewer samples with more time
    group.sample_size(50);
    group.warm_up_time(std::time::Duration::from_secs(10));
    group.measurement_time(std::time::Duration::from_secs(60));

    for (name, text) in &[
        ("short", create_short_text()),
        ("medium", create_medium_text()),
        ("long", create_long_text()),
    ] {
        let node = Node {
            aturi: "test_node".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "sentiment_analysis".to_string(),
            payload: json!({
                "val": ["post", "content", "text"]
            }),
            configuration: json!({"destination": "emotions"}),
            created_at: chrono::Utc::now(),
        };

        let input = json!({
            "post": {
                "content": {
                    "text": text,
                    "author": "user123"
                },
                "metadata": {
                    "likes": 42
                }
            }
        });

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

// Benchmark different input structures
fn bench_input_complexity(c: &mut Criterion) {
    init_model();
    let rt = Runtime::new().unwrap();
    let evaluator = SentimentAnalysisEvaluator::new();

    let mut group = c.benchmark_group("sentiment_analysis_input_complexity");
    // Configure for ML workloads - fewer samples with more time
    group.sample_size(50);
    group.warm_up_time(std::time::Duration::from_secs(10));
    group.measurement_time(std::time::Duration::from_secs(60));

    let text = create_medium_text();

    // Simple flat input
    let simple_input = json!({
        "text": text.clone()
    });

    // Nested input with more fields
    let nested_input = json!({
        "post": {
            "text": text.clone(),
            "author": "user123",
            "metadata": {
                "timestamp": "2024-01-01T00:00:00Z",
                "tags": ["review", "positive"],
                "location": "US"
            }
        },
        "context": {
            "thread_id": "abc123",
            "parent_id": "def456"
        }
    });

    // Very large input object
    let large_input = {
        let mut obj = serde_json::Map::new();
        obj.insert("text".to_string(), json!(text.clone()));
        for i in 0..100 {
            obj.insert(format!("field_{}", i), json!(format!("value_{}", i)));
        }
        json!(obj)
    };

    for (name, input, payload) in &[
        ("simple", simple_input.clone(), json!("text")),
        (
            "nested",
            nested_input.clone(),
            json!({"val": ["post", "text"]}),
        ),
        ("large", large_input.clone(), json!("text")),
    ] {
        let node = Node {
            aturi: "test_node".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "sentiment_analysis".to_string(),
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

// Benchmark with different configuration options
fn bench_configurations(c: &mut Criterion) {
    init_model();
    let rt = Runtime::new().unwrap();
    let evaluator = SentimentAnalysisEvaluator::new();

    let mut group = c.benchmark_group("sentiment_analysis_configurations");
    // Configure for ML workloads - fewer samples with more time
    group.sample_size(30);
    group.measurement_time(std::time::Duration::from_secs(15));
    group.warm_up_time(std::time::Duration::from_secs(5));

    let text = create_medium_text();
    let input = json!({
        "content": text
    });

    for (name, config) in &[
        ("default", json!({})),
        (
            "custom_destination",
            json!({"destination": "mood_analysis"}),
        ),
        (
            "multiple_config",
            json!({
                "destination": "sentiment_result",
                "extra_field": "ignored",
                "another_field": 123
            }),
        ),
    ] {
        let node = Node {
            aturi: "test_node".to_string(),
            blueprint: "test_blueprint".to_string(),
            node_type: "sentiment_analysis".to_string(),
            payload: json!("content"),
            configuration: config.clone(),
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
    bench_string_payload,
    bench_object_payload,
    bench_input_complexity,
    bench_configurations
);
criterion_main!(benches);
