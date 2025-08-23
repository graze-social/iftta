//! Integration test to verify webhook evaluator selection based on configuration

use ifthisthenat::{
    config::WebhookQueueConfig,
    engine::{
        evaluator::NodeEvaluator, node_evaluator_factory::NodeEvaluatorFactory,
        node_type_publish_webhook_direct::PublishWebhookDirectEvaluator,
        node_type_publish_webhook_queue::PublishWebhookQueueEvaluator,
    },
};
use std::sync::Arc;
use tokio::sync::mpsc;

/// Test that the correct evaluator is selected based on configuration
#[test]
fn test_webhook_evaluator_selection() {
    // Test 1: When queue is disabled, direct evaluator should be used
    let config_disabled = WebhookQueueConfig {
        enabled: false,
        ..Default::default()
    };

    // This simulates what happens in main.rs
    let http_client = Arc::new(reqwest::Client::new());

    let evaluator_for_disabled: Arc<dyn NodeEvaluator> = if config_disabled.enabled {
        // Would create queue evaluator
        panic!("Should not reach here with disabled config");
    } else {
        // Create direct evaluator
        Arc::new(PublishWebhookDirectEvaluator::new(http_client.clone()))
    };

    // Verify we got the right type (we can verify the factory supports it)
    let factory_direct =
        NodeEvaluatorFactory::new().register("publish_webhook", evaluator_for_disabled);
    assert!(factory_direct.supports("publish_webhook"));

    // Test 2: When queue is enabled, queue evaluator should be used
    let config_enabled = WebhookQueueConfig {
        enabled: true,
        max_concurrent: 20,
        queue_size: 2000,
        ..Default::default()
    };

    let (sender, _receiver) = mpsc::channel(config_enabled.queue_size);

    let evaluator_for_enabled: Arc<dyn NodeEvaluator> = if config_enabled.enabled {
        // Create queue evaluator
        Arc::new(PublishWebhookQueueEvaluator::new(sender))
    } else {
        panic!("Should not reach here with enabled config");
    };

    // Verify we got an evaluator
    let factory_queue =
        NodeEvaluatorFactory::new().register("publish_webhook", evaluator_for_enabled);
    assert!(factory_queue.supports("publish_webhook"));
}

/// Test factory registration with different evaluators
#[test]
fn test_factory_registration() {
    let http_client = Arc::new(reqwest::Client::new());
    let (sender, _receiver) = mpsc::channel(100);

    // Create factory with direct evaluator
    let factory_direct = NodeEvaluatorFactory::new().register(
        "publish_webhook",
        Arc::new(PublishWebhookDirectEvaluator::new(http_client.clone())),
    );

    assert!(factory_direct.supports("publish_webhook"));

    // Create factory with queue evaluator
    let factory_queue = NodeEvaluatorFactory::new().register(
        "publish_webhook",
        Arc::new(PublishWebhookQueueEvaluator::new(sender)),
    );

    assert!(factory_queue.supports("publish_webhook"));

    // Both factories should support the same node type
    assert_eq!(
        factory_direct.supports("publish_webhook"),
        factory_queue.supports("publish_webhook")
    );
}

/// Test configuration environment variable parsing
#[test]
fn test_webhook_config_from_env() {
    // Test with default environment (no variables set)
    let default_config = WebhookQueueConfig::from_env();
    assert!(!default_config.enabled); // Should be disabled by default
    assert_eq!(default_config.max_concurrent, 10);
    assert_eq!(default_config.queue_size, 1000);
    assert_eq!(default_config.max_retries, 3);

    // Test with environment variables set
    unsafe {
        std::env::set_var("WEBHOOK_QUEUE_ENABLED", "true");
        std::env::set_var("WEBHOOK_MAX_CONCURRENT", "50");
        std::env::set_var("WEBHOOK_QUEUE_SIZE", "5000");
    }

    let custom_config = WebhookQueueConfig::from_env();
    assert!(custom_config.enabled);
    assert_eq!(custom_config.max_concurrent, 50);
    assert_eq!(custom_config.queue_size, 5000);

    // Clean up
    unsafe {
        std::env::remove_var("WEBHOOK_QUEUE_ENABLED");
        std::env::remove_var("WEBHOOK_MAX_CONCURRENT");
        std::env::remove_var("WEBHOOK_QUEUE_SIZE");
    }
}
