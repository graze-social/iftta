//! Integration tests for the denylist functionality.

use anyhow::Result;
use ifthisthenat::denylist::{
    DenyListCursor, DenyListManager, DenyListStorage, postgres::PostgresDenyListStorage,
};
use sqlx::postgres::PgPoolOptions;

async fn setup_test_pool() -> sqlx::PgPool {
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5438/iftta".to_string());

    PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .expect("Failed to connect to database")
}

async fn cleanup_denylist(pool: &sqlx::PgPool) {
    sqlx::query("DELETE FROM denylist WHERE item LIKE 'test-%'")
        .execute(pool)
        .await
        .expect("Failed to clean up test data");
}

#[tokio::test]
async fn test_denylist_workflow() -> Result<()> {
    let pool = setup_test_pool().await;
    cleanup_denylist(&pool).await;

    // Create denylist storage
    let storage = PostgresDenyListStorage::new(pool.clone(), Some(1000), Some(0.01));
    storage.init().await?;

    // Test data
    let test_items = vec![
        ("test-did:plc:malicious123", Some("Known spam account")),
        ("test-handle.bad.actor", Some("Abusive behavior reported")),
        ("test-url://phishing.site", None),
    ];

    // Insert items
    for (item, note) in &test_items {
        storage.insert(item, *note).await?;
    }

    // Check existence
    for (item, _) in &test_items {
        assert!(
            storage.exists(item).await?,
            "Item {} should exist in denylist",
            item
        );
    }

    // Check non-existent item
    assert!(
        !storage.exists("test-good-actor").await?,
        "Non-denied item should not exist in denylist"
    );

    // Get specific item
    let item = storage.get("test-did:plc:malicious123").await?;
    assert!(item.is_some());
    let item = item.unwrap();
    assert_eq!(item.item, "test-did:plc:malicious123");
    assert_eq!(item.note.as_deref(), Some("Known spam account"));

    // List items with pagination
    let cursor = DenyListCursor {
        after: None,
        limit: 2,
    };
    let page = storage.list(cursor).await?;
    assert_eq!(page.items.len(), 2);
    assert!(page.next_cursor.is_some());

    // Delete an item
    let deleted = storage.delete("test-url://phishing.site").await?;
    assert!(deleted);

    // Verify deletion from database
    let item = storage.get("test-url://phishing.site").await?;
    assert!(item.is_none());

    // Note: The item may still return true from exists() due to bloom filter
    // This is expected behavior as we don't rebuild the bloom filter on delete

    cleanup_denylist(&pool).await;
    Ok(())
}

#[tokio::test]
async fn test_bloom_filter_false_positives() -> Result<()> {
    let pool = setup_test_pool().await;
    cleanup_denylist(&pool).await;

    // Create storage with very small bloom filter to increase false positives
    let storage = PostgresDenyListStorage::new(pool.clone(), Some(10), Some(0.5));
    storage.init().await?;

    // Insert a few items
    for i in 0..5 {
        storage.insert(&format!("test-bloom-{}", i), None).await?;
    }

    // Check known items (should always be true)
    for i in 0..5 {
        assert!(
            storage.exists(&format!("test-bloom-{}", i)).await?,
            "Known item should exist"
        );
    }

    // Check many non-existent items
    // With a 0.5 false positive rate, about half should return false
    let mut false_positives = 0;
    for i in 100..200 {
        if storage.exists(&format!("test-bloom-{}", i)).await? {
            false_positives += 1;
        }
    }

    println!(
        "False positive rate: {:.2}%",
        (false_positives as f64 / 100.0) * 100.0
    );

    cleanup_denylist(&pool).await;
    Ok(())
}

#[tokio::test]
async fn test_concurrent_operations() -> Result<()> {
    use tokio::task;

    let pool = setup_test_pool().await;
    cleanup_denylist(&pool).await;

    // Add a small delay to ensure cleanup is complete
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let storage = std::sync::Arc::new(PostgresDenyListStorage::new(
        pool.clone(),
        Some(1000),
        Some(0.01),
    ));
    storage.init().await?;

    // Use a unique timestamp to avoid conflicts with other test runs
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();

    // Spawn multiple tasks that insert items concurrently
    let mut handles = vec![];

    for i in 0..10 {
        let storage = storage.clone();
        let handle = task::spawn(async move {
            let item = format!("test-concurrent-{}-{}", timestamp, i);
            println!("Inserting: {}", item);
            storage.insert(&item, None).await
        });
        handles.push(handle);
    }

    // Wait for all insertions to complete
    for handle in handles {
        handle.await??;
    }

    println!("All insertions completed, checking items...");

    // Verify all items were inserted
    for i in 0..10 {
        let item_key = format!("test-concurrent-{}-{}", timestamp, i);
        println!("Checking for item: {}", item_key);

        let item = storage.get(&item_key).await?;
        assert!(
            item.is_some(),
            "Concurrent item {} should exist in database",
            i
        );

        // Also check that the bloom filter was updated
        assert!(
            storage.exists(&item_key).await?,
            "Concurrent item {} should exist in bloom filter",
            i
        );
    }

    cleanup_denylist(&pool).await;
    Ok(())
}
