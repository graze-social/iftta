//! No-operation implementation of denylist traits.
//!
//! This module provides implementations that do nothing, useful for testing
//! or when denylist functionality needs to be disabled.

use anyhow::Result;
use async_trait::async_trait;

use super::{DenyListCursor, DenyListItem, DenyListManager, DenyListPage, DenyListStorage};

/// No-operation implementation of `DenyListManager`.
///
/// This implementation always returns `false` for existence checks,
/// effectively disabling all denylist filtering.
///
/// # Example
///
/// ```rust
/// use ifthisthenat::denylist::{DenyListManager, noop::NoopDenyListManager};
///
/// # #[tokio::main]
/// # async fn main() -> anyhow::Result<()> {
/// let manager = NoopDenyListManager::new();
///
/// // Always returns false - nothing is denied
/// assert!(!manager.exists("any-item").await?);
/// assert!(!manager.exists("malicious-user").await?);
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, Default)]
pub struct NoopDenyListManager;

impl NoopDenyListManager {
    /// Create a new no-operation denylist manager.
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl DenyListManager for NoopDenyListManager {
    async fn exists(&self, _member: &str) -> Result<bool> {
        // Never deny anything
        Ok(false)
    }
}

/// No-operation implementation of `DenyListStorage`.
///
/// This implementation accepts all operations but doesn't persist anything.
/// All storage operations succeed but have no effect.
///
/// # Example
///
/// ```rust
/// use ifthisthenat::denylist::{DenyListStorage, DenyListCursor, noop::NoopDenyListStorage};
///
/// # #[tokio::main]
/// # async fn main() -> anyhow::Result<()> {
/// let storage = NoopDenyListStorage::new();
///
/// // Insert succeeds but doesn't store anything
/// storage.insert("test-item", Some("test note")).await?;
///
/// // Get always returns None
/// assert!(storage.get("test-item").await?.is_none());
///
/// // List always returns empty
/// let cursor = DenyListCursor { after: None, limit: 10 };
/// let page = storage.list(cursor).await?;
/// assert!(page.items.is_empty());
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, Default)]
pub struct NoopDenyListStorage;

impl NoopDenyListStorage {
    /// Create a new no-operation denylist storage.
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl DenyListStorage for NoopDenyListStorage {
    async fn insert(&self, _item: &str, _note: Option<&str>) -> Result<()> {
        // Pretend to insert successfully
        Ok(())
    }

    async fn delete(&self, _item: &str) -> Result<bool> {
        // Pretend item was not found (and thus not deleted)
        Ok(false)
    }

    async fn get(&self, _item: &str) -> Result<Option<DenyListItem>> {
        // Never find anything
        Ok(None)
    }

    async fn list(&self, _cursor: DenyListCursor) -> Result<DenyListPage> {
        // Always return empty list
        Ok(DenyListPage {
            items: Vec::new(),
            next_cursor: None,
        })
    }
}

#[async_trait]
impl DenyListManager for NoopDenyListStorage {
    async fn exists(&self, _member: &str) -> Result<bool> {
        // Never deny anything
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_noop_manager() {
        let manager = NoopDenyListManager::new();

        // Should never deny anything
        assert!(!manager.exists("test-item").await.unwrap());
        assert!(!manager.exists("malicious-user").await.unwrap());
        assert!(!manager.exists("").await.unwrap());
    }

    #[tokio::test]
    async fn test_noop_storage_insert_and_get() {
        let storage = NoopDenyListStorage::new();

        // Insert should succeed
        storage
            .insert("test-item", Some("test note"))
            .await
            .unwrap();

        // But get should return None
        let item = storage.get("test-item").await.unwrap();
        assert!(item.is_none());
    }

    #[tokio::test]
    async fn test_noop_storage_delete() {
        let storage = NoopDenyListStorage::new();

        // Delete should indicate item was not found
        let deleted = storage.delete("non-existent").await.unwrap();
        assert!(!deleted);
    }

    #[tokio::test]
    async fn test_noop_storage_list() {
        let storage = NoopDenyListStorage::new();

        let cursor = DenyListCursor {
            after: None,
            limit: 10,
        };

        let page = storage.list(cursor).await.unwrap();
        assert!(page.items.is_empty());
        assert!(page.next_cursor.is_none());
    }

    #[tokio::test]
    async fn test_noop_storage_as_manager() {
        let storage = NoopDenyListStorage::new();

        // When used as DenyListManager, should never deny
        assert!(!storage.exists("any-item").await.unwrap());
    }

    #[tokio::test]
    async fn test_noop_storage_list_with_pagination() {
        let storage = NoopDenyListStorage::new();

        // Test with different cursor configurations
        let cursors = [
            DenyListCursor {
                after: None,
                limit: 1,
            },
            DenyListCursor {
                after: Some("some-item".to_string()),
                limit: 100,
            },
            DenyListCursor {
                after: Some("another-item".to_string()),
                limit: 0,
            },
        ];

        for cursor in cursors {
            let page = storage.list(cursor).await.unwrap();
            assert!(page.items.is_empty());
            assert!(page.next_cursor.is_none());
        }
    }

    #[tokio::test]
    async fn test_default_implementations() {
        // Test that Default trait works
        let manager: NoopDenyListManager = Default::default();
        assert!(!manager.exists("test").await.unwrap());

        let storage: NoopDenyListStorage = Default::default();
        assert!(storage.get("test").await.unwrap().is_none());
    }
}
