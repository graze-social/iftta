use async_trait::async_trait;
use cadence::{
    BufferedUdpMetricSink, Counted, CountedExt, Gauged, Metric, QueuingMetricSink, StatsdClient,
    Timed,
};
use std::net::UdpSocket;
use std::sync::Arc;
use thiserror::Error;
use tracing::{debug, error};

/// Trait for publishing metrics with counter, gauge, and timing support
/// Designed for minimal compatibility with cadence-style metrics
#[async_trait]
pub trait MetricsPublisher: Send + Sync {
    /// Increment a counter by 1
    async fn incr(&self, key: &str);

    /// Increment a counter by a specific value
    async fn count(&self, key: &str, value: u64);

    /// Increment a counter with tags
    async fn incr_with_tags(&self, key: &str, tags: &[(&str, &str)]);

    /// Increment a counter by a specific value with tags
    async fn count_with_tags(&self, key: &str, value: u64, tags: &[(&str, &str)]);

    /// Record a gauge value
    async fn gauge(&self, key: &str, value: u64);

    /// Record a gauge value with tags
    async fn gauge_with_tags(&self, key: &str, value: u64, tags: &[(&str, &str)]);

    /// Record a timing in milliseconds
    async fn time(&self, key: &str, millis: u64);

    /// Record a timing with tags
    async fn time_with_tags(&self, key: &str, millis: u64, tags: &[(&str, &str)]);

    /// Record a histogram value
    async fn histogram(&self, key: &str, value: u64);

    /// Record a histogram value with tags
    async fn histogram_with_tags(&self, key: &str, value: u64, tags: &[(&str, &str)]);
}

/// No-op implementation for development and testing
#[derive(Debug, Clone, Default)]
pub struct NoOpMetricsPublisher;

impl NoOpMetricsPublisher {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl MetricsPublisher for NoOpMetricsPublisher {
    async fn incr(&self, _key: &str) {}
    async fn count(&self, _key: &str, _value: u64) {}
    async fn incr_with_tags(&self, _key: &str, _tags: &[(&str, &str)]) {}
    async fn count_with_tags(&self, _key: &str, _value: u64, _tags: &[(&str, &str)]) {}
    async fn gauge(&self, _key: &str, _value: u64) {}
    async fn gauge_with_tags(&self, _key: &str, _value: u64, _tags: &[(&str, &str)]) {}
    async fn time(&self, _key: &str, _millis: u64) {}
    async fn time_with_tags(&self, _key: &str, _millis: u64, _tags: &[(&str, &str)]) {}
    async fn histogram(&self, _key: &str, _value: u64) {}
    async fn histogram_with_tags(&self, _key: &str, _value: u64, _tags: &[(&str, &str)]) {}
}

/// Statsd-backed metrics publisher using cadence
pub struct StatsdMetricsPublisher {
    client: StatsdClient,
    default_tags: Vec<(String, String)>,
}

impl StatsdMetricsPublisher {
    /// Create a new StatsdMetricsPublisher with default configuration
    pub fn new(host: &str, prefix: &str) -> Result<Self, Box<dyn std::error::Error>> {
        Self::new_with_bind(host, prefix, "[::]:0")
    }

    /// Create a new StatsdMetricsPublisher with custom bind address
    pub fn new_with_bind(
        host: &str,
        prefix: &str,
        bind_addr: &str,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        Self::new_with_bind_and_tags(host, prefix, bind_addr, vec![])
    }

    /// Create a new StatsdMetricsPublisher with default tags
    pub fn new_with_tags(
        host: &str,
        prefix: &str,
        default_tags: Vec<(String, String)>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        Self::new_with_bind_and_tags(host, prefix, "[::]:0", default_tags)
    }

    /// Create a new StatsdMetricsPublisher with custom bind address and tags
    pub fn new_with_bind_and_tags(
        host: &str,
        prefix: &str,
        bind_addr: &str,
        default_tags: Vec<(String, String)>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        tracing::info!(
            "Creating StatsdMetricsPublisher: host={}, prefix={}, bind={}, tags={:?}",
            host,
            prefix,
            bind_addr,
            default_tags
        );

        let socket = UdpSocket::bind(bind_addr)?;
        socket.set_nonblocking(true)?;

        let buffered_sink = BufferedUdpMetricSink::from(host, socket)?;
        let queuing_sink = QueuingMetricSink::builder()
            .with_error_handler(move |error| {
                error!("Failed to send metric via sink: {}", error);
            })
            .build(buffered_sink);
        let client = StatsdClient::from_sink(prefix, queuing_sink);

        tracing::info!(
            "StatsdMetricsPublisher created successfully with bind address: {}",
            bind_addr
        );
        Ok(Self {
            client,
            default_tags,
        })
    }

    /// Apply default tags to a builder
    fn apply_default_tags<'a, M>(
        &'a self,
        mut builder: cadence::MetricBuilder<'a, 'a, M>,
    ) -> cadence::MetricBuilder<'a, 'a, M>
    where
        M: Metric + From<String>,
    {
        for (k, v) in &self.default_tags {
            builder = builder.with_tag(k.as_str(), v.as_str());
        }
        builder
    }
}

#[async_trait]
impl MetricsPublisher for StatsdMetricsPublisher {
    async fn incr(&self, key: &str) {
        debug!("Sending metric incr: {}", key);
        if self.default_tags.is_empty() {
            match self.client.incr(key) {
                Ok(_) => debug!("Successfully sent metric: {}", key),
                Err(e) => error!("Failed to send metric {}: {}", key, e),
            }
        } else {
            let builder = self.client.incr_with_tags(key);
            let builder = self.apply_default_tags(builder);
            let _ = builder.send();
            debug!("Sent metric with tags: {}", key);
        }
    }

    async fn count(&self, key: &str, value: u64) {
        if self.default_tags.is_empty() {
            let _ = self.client.count(key, value);
        } else {
            let builder = self.client.count_with_tags(key, value);
            let builder = self.apply_default_tags(builder);
            let _ = builder.send();
        }
    }

    async fn incr_with_tags(&self, key: &str, tags: &[(&str, &str)]) {
        let mut builder = self.client.incr_with_tags(key);
        builder = self.apply_default_tags(builder);
        for (k, v) in tags {
            builder = builder.with_tag(k, v);
        }
        let _ = builder.send();
    }

    async fn count_with_tags(&self, key: &str, value: u64, tags: &[(&str, &str)]) {
        let mut builder = self.client.count_with_tags(key, value);
        builder = self.apply_default_tags(builder);
        for (k, v) in tags {
            builder = builder.with_tag(k, v);
        }
        let _ = builder.send();
    }

    async fn gauge(&self, key: &str, value: u64) {
        debug!("Sending metric gauge: {} = {}", key, value);
        if self.default_tags.is_empty() {
            match self.client.gauge(key, value) {
                Ok(_) => debug!("Successfully sent gauge: {} = {}", key, value),
                Err(e) => error!("Failed to send gauge {} = {}: {}", key, value, e),
            }
        } else {
            let builder = self.client.gauge_with_tags(key, value);
            let builder = self.apply_default_tags(builder);
            let _ = builder.send();
            debug!("Sent gauge with tags: {} = {}", key, value);
        }
    }

    async fn gauge_with_tags(&self, key: &str, value: u64, tags: &[(&str, &str)]) {
        let mut builder = self.client.gauge_with_tags(key, value);
        builder = self.apply_default_tags(builder);
        for (k, v) in tags {
            builder = builder.with_tag(k, v);
        }
        let _ = builder.send();
    }

    async fn time(&self, key: &str, millis: u64) {
        if self.default_tags.is_empty() {
            let _ = self.client.time(key, millis);
        } else {
            let builder = self.client.time_with_tags(key, millis);
            let builder = self.apply_default_tags(builder);
            let _ = builder.send();
        }
    }

    async fn time_with_tags(&self, key: &str, millis: u64, tags: &[(&str, &str)]) {
        let mut builder = self.client.time_with_tags(key, millis);
        builder = self.apply_default_tags(builder);
        for (k, v) in tags {
            builder = builder.with_tag(k, v);
        }
        let _ = builder.send();
    }

    async fn histogram(&self, key: &str, value: u64) {
        // StatsD doesn't have native histogram support, use timing as fallback
        self.time(key, value).await;
    }

    async fn histogram_with_tags(&self, key: &str, value: u64, tags: &[(&str, &str)]) {
        // StatsD doesn't have native histogram support, use timing as fallback
        self.time_with_tags(key, value, tags).await;
    }
}

/// Type alias for shared metrics publisher
pub type SharedMetricsPublisher = Arc<dyn MetricsPublisher>;

/// Metrics-specific errors
#[derive(Debug, Error)]
pub enum MetricsError {
    /// Failed to create metrics publisher
    #[error("error-iftta-metrics-1 Failed to create metrics publisher: {0}")]
    CreationFailed(String),

    /// Invalid configuration for metrics
    #[error("error-iftta-metrics-2 Invalid metrics configuration: {0}")]
    InvalidConfig(String),
}

/// Create a metrics publisher based on configuration
///
/// Returns either a no-op publisher or a StatsD publisher based on the
/// `METRICS_ADAPTER` configuration value.
pub fn create_metrics_publisher(
    metrics_adapter: &str,
    metrics_statsd_host: Option<&str>,
    metrics_prefix: &str,
    metrics_statsd_bind: &str,
    metrics_tags: Option<&str>,
) -> Result<SharedMetricsPublisher, MetricsError> {
    match metrics_adapter {
        "noop" | "" => Ok(Arc::new(NoOpMetricsPublisher::new())),
        "statsd" => {
            let host = metrics_statsd_host.ok_or_else(|| {
                MetricsError::InvalidConfig(
                    "METRICS_STATSD_HOST is required when using statsd adapter".to_string(),
                )
            })?;

            // Parse tags from comma-separated key:value pairs
            let default_tags = if let Some(tags_str) = metrics_tags {
                tags_str
                    .split(',')
                    .filter_map(|tag| {
                        let parts: Vec<&str> = tag.trim().split(':').collect();
                        if parts.len() == 2 {
                            Some((parts[0].to_string(), parts[1].to_string()))
                        } else {
                            error!("Invalid tag format: {}", tag);
                            None
                        }
                    })
                    .collect()
            } else {
                vec![]
            };

            let publisher = StatsdMetricsPublisher::new_with_bind_and_tags(
                host,
                metrics_prefix,
                metrics_statsd_bind,
                default_tags,
            )
            .map_err(|e| MetricsError::CreationFailed(e.to_string()))?;

            Ok(Arc::new(publisher))
        }
        _ => Err(MetricsError::InvalidConfig(format!(
            "Unknown metrics adapter: {}",
            metrics_adapter
        ))),
    }
}

/// Helper struct for timing operations
pub struct MetricTimer {
    start: std::time::Instant,
    metric: String,
    publisher: SharedMetricsPublisher,
    tags: Vec<(String, String)>,
}

impl MetricTimer {
    /// Start a new timer
    pub fn new(metric: impl Into<String>, publisher: SharedMetricsPublisher) -> Self {
        Self {
            start: std::time::Instant::now(),
            metric: metric.into(),
            publisher,
            tags: vec![],
        }
    }

    /// Add a tag to the timer
    pub fn with_tag(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.tags.push((key.into(), value.into()));
        self
    }

    /// Record the elapsed time
    pub async fn record(self) {
        let elapsed = self.start.elapsed().as_millis() as u64;
        if self.tags.is_empty() {
            self.publisher.time(&self.metric, elapsed).await;
        } else {
            let tags: Vec<(&str, &str)> = self
                .tags
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str()))
                .collect();
            self.publisher
                .time_with_tags(&self.metric, elapsed, &tags)
                .await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_noop_metrics() {
        let metrics = NoOpMetricsPublisher::new();

        // These should all be no-ops and not panic
        metrics.incr("test.counter").await;
        metrics.count("test.counter", 5).await;
        metrics
            .incr_with_tags("test.counter", &[("env", "test")])
            .await;
        metrics.gauge("test.gauge", 100).await;
        metrics.time("test.timing", 42).await;
        metrics.histogram("test.histogram", 100).await;
    }

    #[tokio::test]
    async fn test_metric_timer() {
        let metrics: SharedMetricsPublisher = Arc::new(NoOpMetricsPublisher::new());
        let timer = MetricTimer::new("operation.duration", metrics.clone())
            .with_tag("endpoint", "/api/test");

        // Simulate some work
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

        timer.record().await;
    }

    #[test]
    fn test_create_noop_publisher() {
        let result = create_metrics_publisher("noop", None, "ifthisthenat", "[::]:0", None);
        assert!(result.is_ok());
    }

    #[test]
    fn test_missing_statsd_host() {
        let result = create_metrics_publisher("statsd", None, "ifthisthenat", "[::]:0", None);
        assert!(result.is_err());
        if let Err(e) = result {
            assert!(matches!(e, MetricsError::InvalidConfig(_)));
        }
    }
}
