//! Tests for Redis-based Jetstream cursor persistence

#[cfg(test)]
mod redis_cursor_tests {
    use atproto_jetstream::{
        EventHandler, JetstreamEvent, JetstreamEventCommit, JetstreamEventDelete,
    };
    use deadpool_redis::redis::{self, AsyncCommands};
    use ifthisthenat::consumer::{Consumer, RedisCursorHandler};
    use ifthisthenat::storage::cache::create_cache_pool;
    use std::time::Duration;

    /// Helper to check if Redis is available for testing
    async fn redis_available() -> bool {
        match std::env::var("TEST_REDIS_URL") {
            Ok(url) => match redis::Client::open(url.as_str()) {
                Ok(client) => client.get_multiplexed_async_connection().await.is_ok(),
                Err(_) => false,
            },
            Err(_) => false,
        }
    }

    #[tokio::test]
    async fn test_redis_cursor_write_and_read() {
        if !redis_available().await {
            eprintln!("Skipping test: Redis not available. Set TEST_REDIS_URL to enable.");
            return;
        }

        let redis_url = std::env::var("TEST_REDIS_URL").unwrap();
        let redis_pool = create_cache_pool(&redis_url).expect("Failed to create Redis pool");
        let redis_key = format!("test:jetstream:cursor:{}", uuid::Uuid::new_v4());

        // Create handler
        let handler = RedisCursorHandler::new(
            "test-handler".to_string(),
            redis_pool.clone(),
            redis_key.clone(),
            60, // 60 seconds TTL
        )
        .await
        .expect("Failed to create Redis cursor handler");

        // Simulate a Jetstream event
        // Create a mock commit structure with the fields we need
        let commit = JetstreamEventCommit {
            cid: "test-cid".to_string(),
            rev: "test-rev".to_string(),
            operation: "create".to_string(),
            collection: "app.bsky.feed.post".to_string(),
            rkey: "test-rkey".to_string(),
            record: serde_json::json!({"text": "test post"}),
        };

        let event = JetstreamEvent::Commit {
            did: "did:plc:test123".to_string(),
            commit,
            time_us: 1234567890,
            kind: "commit".to_string(),
        };

        // Process the event
        handler
            .handle_event(event)
            .await
            .expect("Failed to handle event");

        // Wait for write to complete (handler writes every 5 seconds)
        tokio::time::sleep(Duration::from_secs(6)).await;

        // Read back the cursor
        let cursor = RedisCursorHandler::read_cursor(&redis_pool, &redis_key).await;
        assert_eq!(
            cursor,
            Some(1234567890),
            "Cursor should be saved and readable"
        );

        // Clean up - delete the test key
        if let Ok(mut conn) = redis_pool.get().await {
            let _: Result<(), _> = conn.del(&redis_key).await;
        }
    }

    #[tokio::test]
    async fn test_redis_cursor_ttl() {
        if !redis_available().await {
            eprintln!("Skipping test: Redis not available. Set TEST_REDIS_URL to enable.");
            return;
        }

        let redis_url = std::env::var("TEST_REDIS_URL").unwrap();
        let redis_pool = create_cache_pool(&redis_url).expect("Failed to create Redis pool");
        let redis_key = format!("test:jetstream:cursor:ttl:{}", uuid::Uuid::new_v4());

        // Create handler with short TTL
        let handler = RedisCursorHandler::new(
            "test-handler-ttl".to_string(),
            redis_pool.clone(),
            redis_key.clone(),
            2, // 2 seconds TTL
        )
        .await
        .expect("Failed to create Redis cursor handler");

        // Simulate event
        let commit = JetstreamEventDelete {
            rev: "test-rev-2".to_string(),
            operation: "delete".to_string(),
            collection: "app.bsky.feed.post".to_string(),
            rkey: "test-rkey-2".to_string(),
        };

        let event = JetstreamEvent::Delete {
            did: "did:plc:test456".to_string(),
            commit,
            time_us: 9876543210,
            kind: "commit".to_string(),
        };

        // Process event
        handler
            .handle_event(event)
            .await
            .expect("Failed to handle event");

        // Wait for write
        tokio::time::sleep(Duration::from_secs(6)).await;

        // Read immediately
        let cursor1 = RedisCursorHandler::read_cursor(&redis_pool, &redis_key).await;
        assert_eq!(
            cursor1,
            Some(9876543210),
            "Cursor should be present initially"
        );

        // Wait for TTL to expire (adding buffer time)
        tokio::time::sleep(Duration::from_secs(4)).await;

        // Should be gone now
        let cursor2 = RedisCursorHandler::read_cursor(&redis_pool, &redis_key).await;
        assert_eq!(cursor2, None, "Cursor should have expired");
    }

    #[tokio::test]
    async fn test_consumer_redis_integration() {
        if !redis_available().await {
            eprintln!("Skipping test: Redis not available. Set TEST_REDIS_URL to enable.");
            return;
        }

        let redis_url = std::env::var("TEST_REDIS_URL").unwrap();
        let redis_pool = create_cache_pool(&redis_url).expect("Failed to create Redis pool");
        let redis_key = format!("test:consumer:cursor:{}", uuid::Uuid::new_v4());

        let consumer = Consumer {};

        // Create Redis cursor handler through consumer
        let handler = consumer
            .create_redis_cursor_handler(
                redis_pool.clone(),
                redis_key.clone(),
                3600, // 1 hour TTL
            )
            .await
            .expect("Failed to create Redis cursor handler via consumer");

        // Verify it's created correctly
        assert_eq!(handler.handler_id(), "redis-cursor-writer");

        // Test reading cursor through consumer helper
        let cursor = Consumer::read_cursor_from_redis(&redis_pool, &redis_key).await;
        assert_eq!(cursor, None, "Should return None for non-existent cursor");

        // Clean up
        if let Ok(mut conn) = redis_pool.get().await {
            let _: Result<(), _> = conn.del(&redis_key).await;
        }
    }
}
