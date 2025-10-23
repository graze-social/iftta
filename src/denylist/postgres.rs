//! PostgreSQL-backed implementation of denylist storage with bloom filter optimization.

use anyhow::Result;
use async_trait::async_trait;
use chrono::Utc;
use fastbloom::BloomFilter;
use sqlx::{PgPool, Row};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use super::{DenyListCursor, DenyListItem, DenyListManager, DenyListPage, DenyListStorage};

/// PostgreSQL-backed denylist storage with bloom filter optimization.
///
/// This implementation uses a PostgreSQL table for persistent storage
/// and an in-memory bloom filter for fast membership checks. The bloom
/// filter is populated on initialization and updated when new items are
/// added.
pub struct PostgresDenyListStorage {
    pool: PgPool,
    /// Bloom filter for fast membership checks.
    /// Protected by RwLock for thread-safe access.
    bloom_filter: Arc<RwLock<BloomFilter>>,
    /// Expected number of items for bloom filter sizing
    expected_items: usize,
    /// False positive rate for bloom filter
    false_positive_rate: f64,
}

impl PostgresDenyListStorage {
    /// Create a new PostgresDenyListStorage instance.
    ///
    /// # Arguments
    ///
    /// * `pool` - PostgreSQL connection pool
    /// * `expected_items` - Expected number of items for bloom filter sizing (default: 100_000)
    /// * `false_positive_rate` - Target false positive rate for bloom filter (default: 0.01)
    pub fn new(
        pool: PgPool,
        expected_items: Option<usize>,
        false_positive_rate: Option<f64>,
    ) -> Self {
        let expected_items = expected_items.unwrap_or(100_000);
        let false_positive_rate = false_positive_rate.unwrap_or(0.01);

        let bloom_filter =
            BloomFilter::with_false_pos(false_positive_rate).expected_items(expected_items);

        Self {
            pool,
            bloom_filter: Arc::new(RwLock::new(bloom_filter)),
            expected_items,
            false_positive_rate,
        }
    }

    /// Initialize the bloom filter by loading all items from the database.
    ///
    /// This method should be called after creating the storage instance
    /// to populate the bloom filter with existing denylist items.
    pub async fn init(&self) -> Result<()> {
        info!("Initializing denylist bloom filter");

        let mut total_items = 0;
        let mut cursor = DenyListCursor {
            after: None,
            limit: 1000, // Load in batches of 1000
        };

        // Clear and recreate bloom filter to ensure clean state
        {
            let mut bloom = self.bloom_filter.write().await;
            *bloom = BloomFilter::with_false_pos(self.false_positive_rate)
                .expected_items(self.expected_items);
        }

        loop {
            let page = self.list(cursor.clone()).await?;

            if page.items.is_empty() {
                break;
            }

            // Add items to bloom filter
            {
                let mut bloom = self.bloom_filter.write().await;
                for item in &page.items {
                    bloom.insert(&item.item);
                    total_items += 1;
                }
            }

            if let Some(next) = page.next_cursor {
                cursor.after = Some(next);
            } else {
                break;
            }
        }

        info!(
            "Initialized denylist bloom filter with {} items",
            total_items
        );

        Ok(())
    }
}

#[async_trait]
impl DenyListManager for PostgresDenyListStorage {
    async fn exists(&self, member: &str) -> Result<bool> {
        // First check bloom filter for fast negative responses
        {
            let bloom = self.bloom_filter.read().await;
            if !bloom.contains(member) {
                // Bloom filter guarantees item is not in the set
                return Ok(false);
            }
        }

        // Bloom filter indicates possible membership, check database
        let exists =
            sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM denylist WHERE item = $1)")
                .bind(member)
                .fetch_one(&self.pool)
                .await?;

        Ok(exists)
    }
}

#[async_trait]
impl DenyListStorage for PostgresDenyListStorage {
    async fn insert(&self, item: &str, note: Option<&str>) -> Result<()> {
        let now = Utc::now();

        // Insert into database
        sqlx::query(
            r#"
            INSERT INTO denylist (item, created_at, note)
            VALUES ($1, $2, $3)
            ON CONFLICT (item) DO UPDATE SET
                note = COALESCE(EXCLUDED.note, denylist.note)
            "#,
        )
        .bind(item)
        .bind(now)
        .bind(note)
        .execute(&self.pool)
        .await?;

        // Add to bloom filter
        {
            let mut bloom = self.bloom_filter.write().await;
            bloom.insert(item);
        }

        debug!("Added item to denylist: {}", item);
        Ok(())
    }

    async fn delete(&self, item: &str) -> Result<bool> {
        let result = sqlx::query("DELETE FROM denylist WHERE item = $1")
            .bind(item)
            .execute(&self.pool)
            .await?;

        let deleted = result.rows_affected() > 0;

        if deleted {
            debug!("Deleted item from denylist: {}", item);
            // Note: We intentionally do not remove from bloom filter
            // as per requirements - this may cause false positives
            // but maintains bloom filter integrity
            warn!(
                "Item '{}' deleted from database but remains in bloom filter (may cause false positives)",
                item
            );
        }

        Ok(deleted)
    }

    async fn get(&self, item: &str) -> Result<Option<DenyListItem>> {
        let row = sqlx::query(
            r#"
            SELECT item, created_at, note
            FROM denylist
            WHERE item = $1
            "#,
        )
        .bind(item)
        .fetch_optional(&self.pool)
        .await?;

        if let Some(row) = row {
            Ok(Some(DenyListItem {
                item: row.get("item"),
                created_at: row.get("created_at"),
                note: row.get("note"),
            }))
        } else {
            Ok(None)
        }
    }

    async fn list(&self, cursor: DenyListCursor) -> Result<DenyListPage> {
        let limit = cursor.limit.min(1000); // Cap at 1000 items per page

        let rows = if let Some(after) = cursor.after {
            sqlx::query(
                r#"
                SELECT item, created_at, note
                FROM denylist
                WHERE item > $1
                ORDER BY item ASC
                LIMIT $2
                "#,
            )
            .bind(after)
            .bind(limit as i64 + 1) // Fetch one extra to determine if there's a next page
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query(
                r#"
                SELECT item, created_at, note
                FROM denylist
                ORDER BY item ASC
                LIMIT $1
                "#,
            )
            .bind(limit as i64 + 1)
            .fetch_all(&self.pool)
            .await?
        };

        let mut items = Vec::with_capacity(limit);
        let mut next_cursor = None;

        for (i, row) in rows.into_iter().enumerate() {
            if i >= limit {
                // This is the extra item, use it for the next cursor
                next_cursor = Some(row.get::<String, _>("item"));
                break;
            }

            items.push(DenyListItem {
                item: row.get("item"),
                created_at: row.get("created_at"),
                note: row.get("note"),
            });
        }

        Ok(DenyListPage { items, next_cursor })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::postgres::PgPoolOptions;

    /// Helper to check if test database is available
    async fn test_db_available() -> bool {
        let database_url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            "postgres://postgres:postgres@localhost:5432/ifthisthenat_test".to_string()
        });

        PgPoolOptions::new()
            .max_connections(1)
            .connect(&database_url)
            .await
            .is_ok()
    }

    async fn setup_test_db() -> PgPool {
        let database_url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            "postgres://postgres:postgres@localhost:5432/ifthisthenat_test".to_string()
        });

        PgPoolOptions::new()
            .max_connections(5)
            .connect(&database_url)
            .await
            .expect("Failed to connect to test database")
    }

    async fn prepare_test_db(pool: &PgPool) {
        // Create test table
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS denylist (
                item TEXT PRIMARY KEY,
                created_at TIMESTAMPTZ NOT NULL,
                note TEXT
            )
            "#,
        )
        .execute(pool)
        .await
        .expect("Failed to create test table");

        // Clear any existing data
        sqlx::query("TRUNCATE denylist")
            .execute(pool)
            .await
            .expect("Failed to truncate test table");
    }

    async fn cleanup_test_db(pool: &PgPool) {
        sqlx::query("DROP TABLE IF EXISTS denylist")
            .execute(pool)
            .await
            .expect("Failed to drop test table");
    }

    #[tokio::test]
    async fn test_insert_and_exists() {
        if !test_db_available().await {
            eprintln!("Skipping test: Database not available. Set TEST_DATABASE_URL to enable.");
            return;
        }

        let pool = setup_test_db().await;
        prepare_test_db(&pool).await;

        let storage = PostgresDenyListStorage::new(pool.clone(), Some(100), Some(0.01));
        storage.init().await.unwrap();

        // Test insertion
        storage
            .insert("test-item-1", Some("Test note"))
            .await
            .unwrap();

        // Test existence check
        assert!(storage.exists("test-item-1").await.unwrap());
        assert!(!storage.exists("non-existent").await.unwrap());

        cleanup_test_db(&pool).await;
    }

    #[tokio::test]
    async fn test_get_item() {
        if !test_db_available().await {
            eprintln!("Skipping test: Database not available. Set TEST_DATABASE_URL to enable.");
            return;
        }

        let pool = setup_test_db().await;
        prepare_test_db(&pool).await;
        let storage = PostgresDenyListStorage::new(pool.clone(), Some(100), Some(0.01));
        storage.init().await.unwrap();

        // Insert item
        storage
            .insert("test-item-2", Some("Note for item 2"))
            .await
            .unwrap();

        // Get item
        let item = storage.get("test-item-2").await.unwrap();
        assert!(item.is_some());
        let item = item.unwrap();
        assert_eq!(item.item, "test-item-2");
        assert_eq!(item.note.as_deref(), Some("Note for item 2"));

        // Get non-existent item
        let item = storage.get("non-existent").await.unwrap();
        assert!(item.is_none());

        cleanup_test_db(&pool).await;
    }

    #[tokio::test]
    async fn test_delete_item() {
        if !test_db_available().await {
            eprintln!("Skipping test: Database not available. Set TEST_DATABASE_URL to enable.");
            return;
        }

        let pool = setup_test_db().await;
        prepare_test_db(&pool).await;
        let storage = PostgresDenyListStorage::new(pool.clone(), Some(100), Some(0.01));
        storage.init().await.unwrap();

        // Insert and verify
        storage.insert("test-item-3", None).await.unwrap();
        assert!(storage.exists("test-item-3").await.unwrap());

        // Delete
        let deleted = storage.delete("test-item-3").await.unwrap();
        assert!(deleted);

        // Verify deletion from database
        let item = storage.get("test-item-3").await.unwrap();
        assert!(item.is_none());

        // Note: Item may still return true from exists() due to bloom filter
        // This is expected behavior as we don't rebuild bloom filter on delete

        // Try to delete non-existent item
        let deleted = storage.delete("non-existent").await.unwrap();
        assert!(!deleted);

        cleanup_test_db(&pool).await;
    }

    #[tokio::test]
    async fn test_list_pagination() {
        if !test_db_available().await {
            eprintln!("Skipping test: Database not available. Set TEST_DATABASE_URL to enable.");
            return;
        }

        let pool = setup_test_db().await;
        prepare_test_db(&pool).await;
        let storage = PostgresDenyListStorage::new(pool.clone(), Some(100), Some(0.01));

        // Insert multiple items
        for i in 0..5 {
            storage
                .insert(&format!("item-{:02}", i), Some(&format!("Note {}", i)))
                .await
                .unwrap();
        }

        // Test listing with pagination
        let cursor = DenyListCursor {
            after: None,
            limit: 2,
        };
        let page1 = storage.list(cursor).await.unwrap();
        assert_eq!(page1.items.len(), 2);
        assert!(page1.next_cursor.is_some());
        assert_eq!(page1.items[0].item, "item-00");
        assert_eq!(page1.items[1].item, "item-01");

        // Get next page
        let cursor = DenyListCursor {
            after: page1.next_cursor,
            limit: 2,
        };
        let page2 = storage.list(cursor).await.unwrap();
        assert_eq!(page2.items.len(), 2);
        assert!(page2.next_cursor.is_some());
        assert_eq!(page2.items[0].item, "item-02");
        assert_eq!(page2.items[1].item, "item-03");

        // Get last page
        let cursor = DenyListCursor {
            after: page2.next_cursor,
            limit: 2,
        };
        let page3 = storage.list(cursor).await.unwrap();
        assert_eq!(page3.items.len(), 1);
        assert!(page3.next_cursor.is_none());
        assert_eq!(page3.items[0].item, "item-04");

        cleanup_test_db(&pool).await;
    }

    #[tokio::test]
    async fn test_bloom_filter_init() {
        if !test_db_available().await {
            eprintln!("Skipping test: Database not available. Set TEST_DATABASE_URL to enable.");
            return;
        }

        let pool = setup_test_db().await;
        prepare_test_db(&pool).await;
        let storage = PostgresDenyListStorage::new(pool.clone(), Some(100), Some(0.01));

        // Insert items before init
        for i in 0..10 {
            sqlx::query(
                r#"
                INSERT INTO denylist (item, created_at, note)
                VALUES ($1, $2, $3)
                "#,
            )
            .bind(format!("pre-init-{}", i))
            .bind(Utc::now())
            .bind(format!("Pre-init note {}", i))
            .execute(&pool)
            .await
            .unwrap();
        }

        // Initialize bloom filter
        storage.init().await.unwrap();

        // Check that pre-existing items are in bloom filter
        for i in 0..10 {
            assert!(storage.exists(&format!("pre-init-{}", i)).await.unwrap());
        }

        cleanup_test_db(&pool).await;
    }

    #[tokio::test]
    async fn test_insert_duplicate() {
        if !test_db_available().await {
            eprintln!("Skipping test: Database not available. Set TEST_DATABASE_URL to enable.");
            return;
        }

        let pool = setup_test_db().await;
        prepare_test_db(&pool).await;
        let storage = PostgresDenyListStorage::new(pool.clone(), Some(100), Some(0.01));
        storage.init().await.unwrap();

        // Insert item
        storage
            .insert("duplicate-item", Some("First note"))
            .await
            .unwrap();

        // Insert duplicate with different note
        storage
            .insert("duplicate-item", Some("Second note"))
            .await
            .unwrap();

        // Verify only one item exists
        let item = storage.get("duplicate-item").await.unwrap().unwrap();
        assert_eq!(item.item, "duplicate-item");
        // Note should be updated to the new value
        assert_eq!(item.note.as_deref(), Some("Second note"));

        cleanup_test_db(&pool).await;
    }
}
