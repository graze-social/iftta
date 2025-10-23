//! Test helper utilities for ifthisthenat tests
//!
//! This module provides common test fixtures, mock implementations, and
//! utility functions for testing various components of the system.

use async_trait::async_trait;
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

// Test environment mutex to prevent concurrent environment variable modification
pub static ENV_MUTEX: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

/// Setup test environment with required variables
pub fn setup_test_env() {
    let _guard = ENV_MUTEX.lock();
    unsafe {
        std::env::set_var("EXTERNAL_BASE", "http://test.example.com");
        std::env::set_var(
            "HTTP_COOKIE_KEY",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa=",
        );
        std::env::set_var("ISSUER_DID", "did:plc:test123");
        std::env::set_var("AIP_CLIENT_ID", "test-client");
        std::env::set_var("AIP_CLIENT_SECRET", "test-secret");
        std::env::set_var("DATABASE_URL", "postgres://test:test@localhost/test");
    }
}

/// Clean up test environment
pub fn cleanup_test_env() {
    let _guard = ENV_MUTEX.lock();
    unsafe {
        std::env::remove_var("EXTERNAL_BASE");
        std::env::remove_var("HTTP_COOKIE_KEY");
        std::env::remove_var("ISSUER_DID");
        std::env::remove_var("AIP_CLIENT_ID");
        std::env::remove_var("AIP_CLIENT_SECRET");
        std::env::remove_var("DATABASE_URL");
    }
}

/// Mock blueprint for testing
pub fn create_test_blueprint() -> crate::storage::Blueprint {
    use crate::storage::Blueprint;
    use ulid::Ulid;

    Blueprint {
        aturi: format!(
            "at://did:plc:test123/tools.graze.ifthisthenat.blueprint/{}",
            Ulid::new()
        ),
        did: "did:plc:test123".to_string(),
        node_order: vec!["node1".to_string(), "node2".to_string()],
        enabled: true,
        error: None,
        created_at: chrono::Utc::now(),
    }
}

/// Mock node for testing
pub fn create_test_node(node_type: &str) -> crate::storage::Node {
    use crate::storage::Node;
    use ulid::Ulid;

    let (payload, configuration) = match node_type {
        "jetstream_entry" => (
            serde_json::json!({
                "kind": "commit",
                "collection": ["app.bsky.feed.post"]
            }),
            serde_json::json!({}),
        ),
        "condition" => (
            serde_json::json!({
                "expr": {"==": [{"val": ["text"]}, "test"]}
            }),
            serde_json::json!({}),
        ),
        "transform" => (
            serde_json::json!({
                "output": "transformed",
                "expr": {"cat": ["prefix-", {"val": ["input"]}]}
            }),
            serde_json::json!({}),
        ),
        "publish_record" => (
            serde_json::json!({
                "output": "record",
                "collection": "app.bsky.feed.post",
                "record": {"text": "test post"}
            }),
            serde_json::json!({}),
        ),
        _ => (serde_json::json!({}), serde_json::json!({})),
    };

    Node {
        aturi: format!(
            "at://did:plc:test123/tools.graze.ifthisthenat.node/{}",
            Ulid::new()
        ),
        blueprint: "at://did:plc:test123/tools.graze.ifthisthenat.blueprint/test".to_string(),
        node_type: node_type.to_string(),
        payload,
        configuration,
        created_at: chrono::Utc::now(),
    }
}

/// Mock queue adapter for testing
pub struct MockQueueAdapter<T> {
    items: Arc<Mutex<Vec<T>>>,
    push_count: Arc<Mutex<usize>>,
    pull_count: Arc<Mutex<usize>>,
    fail_on_push: bool,
    fail_on_pull: bool,
}

impl<T> MockQueueAdapter<T> {
    pub fn new() -> Self {
        Self {
            items: Arc::new(Mutex::new(Vec::new())),
            push_count: Arc::new(Mutex::new(0)),
            pull_count: Arc::new(Mutex::new(0)),
            fail_on_push: false,
            fail_on_pull: false,
        }
    }

    pub fn with_failure(mut self, fail_on_push: bool, fail_on_pull: bool) -> Self {
        self.fail_on_push = fail_on_push;
        self.fail_on_pull = fail_on_pull;
        self
    }

    pub fn push_count(&self) -> usize {
        *self.push_count.lock()
    }

    pub fn pull_count(&self) -> usize {
        *self.pull_count.lock()
    }
}

#[async_trait]
impl<T: Send + Sync + 'static> crate::queue_adapter::QueueAdapter<T> for MockQueueAdapter<T> {
    async fn push(&self, item: T) -> anyhow::Result<()> {
        if self.fail_on_push {
            return Err(anyhow::anyhow!("Mock push failure"));
        }
        *self.push_count.lock() += 1;
        self.items.lock().push(item);
        Ok(())
    }

    async fn pull(&self) -> Option<T> {
        if self.fail_on_pull {
            return None;
        }
        *self.pull_count.lock() += 1;
        self.items.lock().pop()
    }

    async fn depth(&self) -> Option<usize> {
        Some(self.items.lock().len())
    }

    async fn is_healthy(&self) -> bool {
        !self.fail_on_push && !self.fail_on_pull
    }
}

/// Mock metrics publisher for testing
pub struct MockMetricsPublisher {
    pub counters: Arc<Mutex<HashMap<String, u64>>>,
    pub gauges: Arc<Mutex<HashMap<String, u64>>>,
    pub timings: Arc<Mutex<HashMap<String, Vec<u64>>>>,
}

impl MockMetricsPublisher {
    pub fn new() -> Self {
        Self {
            counters: Arc::new(Mutex::new(HashMap::new())),
            gauges: Arc::new(Mutex::new(HashMap::new())),
            timings: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn get_counter(&self, key: &str) -> u64 {
        self.counters.lock().get(key).copied().unwrap_or(0)
    }

    pub fn get_gauge(&self, key: &str) -> u64 {
        self.gauges.lock().get(key).copied().unwrap_or(0)
    }

    pub fn get_timings(&self, key: &str) -> Vec<u64> {
        self.timings.lock().get(key).cloned().unwrap_or_default()
    }
}

#[async_trait]
impl crate::metrics::MetricsPublisher for MockMetricsPublisher {
    async fn incr(&self, key: &str) {
        self.count(key, 1).await;
    }

    async fn count(&self, key: &str, value: u64) {
        let mut counters = self.counters.lock();
        *counters.entry(key.to_string()).or_insert(0) += value;
    }

    async fn incr_with_tags(&self, key: &str, _tags: &[(&str, &str)]) {
        self.incr(key).await;
    }

    async fn count_with_tags(&self, key: &str, value: u64, _tags: &[(&str, &str)]) {
        self.count(key, value).await;
    }

    async fn gauge(&self, key: &str, value: u64) {
        self.gauges.lock().insert(key.to_string(), value);
    }

    async fn gauge_with_tags(&self, key: &str, value: u64, _tags: &[(&str, &str)]) {
        self.gauge(key, value).await;
    }

    async fn time(&self, key: &str, millis: u64) {
        self.timings
            .lock()
            .entry(key.to_string())
            .or_default()
            .push(millis);
    }

    async fn time_with_tags(&self, key: &str, millis: u64, _tags: &[(&str, &str)]) {
        self.time(key, millis).await;
    }

    async fn histogram(&self, key: &str, value: u64) {
        self.time(key, value).await;
    }

    async fn histogram_with_tags(&self, key: &str, value: u64, _tags: &[(&str, &str)]) {
        self.histogram(key, value).await;
    }
}

/// Create a test database pool
pub async fn create_test_db_pool() -> sqlx::PgPool {
    let database_url = std::env::var("TEST_DATABASE_URL")
        .unwrap_or_else(|_| "postgres://test:test@localhost/test".to_string());

    sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .expect("Failed to create test database pool")
}

/// Create a test Redis pool
pub fn create_test_redis_pool() -> deadpool_redis::Pool {
    let redis_url =
        std::env::var("TEST_REDIS_URL").unwrap_or_else(|_| "redis://localhost:6379".to_string());

    let config = deadpool_redis::Config::from_url(&redis_url);
    config
        .create_pool(Some(deadpool_redis::Runtime::Tokio1))
        .expect("Failed to create test Redis pool")
}

/// Test fixture for Jetstream event
pub fn create_test_jetstream_event() -> Value {
    serde_json::json!({
        "did": "did:plc:test123",
        "time_us": 1234567890,
        "kind": "commit",
        "commit": {
            "rev": "test-rev",
            "operation": "create",
            "collection": "app.bsky.feed.post",
            "rkey": "test-rkey",
            "record": {
                "text": "Test post",
                "createdAt": "2024-01-01T00:00:00Z"
            }
        }
    })
}

/// Test fixture for webhook payload
pub fn create_test_webhook_payload() -> Value {
    serde_json::json!({
        "event": "test",
        "data": {
            "id": "test-id",
            "value": "test-value"
        },
        "timestamp": "2024-01-01T00:00:00Z"
    })
}

/// Helper to run async tests with timeout
pub async fn with_timeout<T, F>(duration: std::time::Duration, future: F) -> Result<T, String>
where
    F: std::future::Future<Output = T>,
{
    tokio::time::timeout(duration, future)
        .await
        .map_err(|_| "Test timed out".to_string())
}

/// Helper to create a temporary directory for tests
pub fn create_temp_dir(prefix: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("iftta-test-{}-{}", prefix, ulid::Ulid::new()));
    std::fs::create_dir_all(&dir).expect("Failed to create temp dir");
    dir
}

/// Helper to clean up temporary directory
pub fn cleanup_temp_dir(path: &std::path::Path) {
    if path.exists() {
        std::fs::remove_dir_all(path).ok();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::MetricsPublisher;
    use crate::queue_adapter::QueueAdapter;

    #[test]
    fn test_env_setup() {
        setup_test_env();
        assert_eq!(
            std::env::var("EXTERNAL_BASE").unwrap(),
            "http://test.example.com"
        );
        cleanup_test_env();
        assert!(std::env::var("EXTERNAL_BASE").is_err());
    }

    #[tokio::test]
    async fn test_mock_queue_adapter() {
        let adapter = MockQueueAdapter::new();
        adapter.push("test".to_string()).await.unwrap();
        assert_eq!(adapter.push_count(), 1);
        assert_eq!(adapter.depth().await, Some(1));

        let item = adapter.pull().await;
        assert_eq!(item, Some("test".to_string()));
        assert_eq!(adapter.pull_count(), 1);
    }

    #[tokio::test]
    async fn test_mock_metrics_publisher() {
        let metrics = MockMetricsPublisher::new();

        metrics.incr("test.counter").await;
        metrics.count("test.counter", 5).await;
        assert_eq!(metrics.get_counter("test.counter"), 6);

        metrics.gauge("test.gauge", 100).await;
        assert_eq!(metrics.get_gauge("test.gauge"), 100);

        metrics.time("test.timing", 42).await;
        metrics.time("test.timing", 84).await;
        assert_eq!(metrics.get_timings("test.timing"), vec![42, 84]);
    }

    #[tokio::test]
    async fn test_with_timeout() {
        let result = with_timeout(std::time::Duration::from_millis(100), async {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            "success"
        })
        .await;
        assert_eq!(result, Ok("success"));

        let result = with_timeout(std::time::Duration::from_millis(10), async {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            "success"
        })
        .await;
        assert!(result.is_err());
    }
}
