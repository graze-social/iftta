//! Denylist management module for filtering and blocking unwanted content.
//!
//! This module provides traits and implementations for managing denylists
//! with efficient lookups using bloom filters and persistent storage.
//!
//! # Example Usage
//!
//! ```rust
//! use ifthisthenat::denylist::{DenyListManager, noop::NoopDenyListManager};
//!
//! # #[tokio::main]
//! # async fn main() -> anyhow::Result<()> {
//! // Create a no-op manager that never denies anything
//! let manager = NoopDenyListManager::new();
//!
//! // Check if a user should be blocked
//! let should_block = manager.exists("did:plc:user123").await?;
//! if should_block {
//!     println!("User is on denylist, blocking action");
//! } else {
//!     println!("User is allowed");
//! }
//! # Ok(())
//! # }
//! ```

use anyhow::Result;
use async_trait::async_trait;

pub mod noop;
pub mod postgres;

/// Trait for checking membership in a denylist.
///
/// This trait provides a simple interface for checking whether an item
/// exists in a denylist. Implementations should be optimized for fast
/// lookups, potentially using techniques like bloom filters.
#[async_trait]
pub trait DenyListManager: Send + Sync {
    /// Check if a member exists in the denylist.
    ///
    /// # Arguments
    ///
    /// * `member` - The item to check for membership in the denylist
    ///
    /// # Returns
    ///
    /// * `Ok(true)` if the item exists in the denylist
    /// * `Ok(false)` if the item does not exist in the denylist
    /// * `Err` if there was an error checking membership
    async fn exists(&self, member: &str) -> Result<bool>;
}

/// Cursor for paginating through denylist items.
#[derive(Debug, Clone, Default)]
pub struct DenyListCursor {
    /// The item to start listing from (exclusive)
    pub after: Option<String>,
    /// Maximum number of items to return
    pub limit: usize,
}

/// A single item in the denylist.
#[derive(Debug, Clone)]
pub struct DenyListItem {
    /// The denied item (e.g., DID, handle, URL)
    pub item: String,
    /// When this item was added to the denylist
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// Optional note about why this item was denied
    pub note: Option<String>,
}

/// Result of a paginated list operation.
#[derive(Debug)]
pub struct DenyListPage {
    /// The items in this page
    pub items: Vec<DenyListItem>,
    /// Cursor for fetching the next page, if any
    pub next_cursor: Option<String>,
}

/// Trait for persistent storage of denylist items.
///
/// This trait provides CRUD operations for managing denylist items
/// in persistent storage. Implementations should be thread-safe and
/// suitable for use in async contexts.
#[async_trait]
pub trait DenyListStorage: Send + Sync {
    /// Insert a new item into the denylist.
    ///
    /// # Arguments
    ///
    /// * `item` - The item to add to the denylist
    /// * `note` - Optional note about why this item is being denied
    ///
    /// # Returns
    ///
    /// * `Ok(())` if the item was successfully inserted
    /// * `Err` if there was an error inserting the item
    async fn insert(&self, item: &str, note: Option<&str>) -> Result<()>;

    /// Delete an item from the denylist.
    ///
    /// # Arguments
    ///
    /// * `item` - The item to remove from the denylist
    ///
    /// # Returns
    ///
    /// * `Ok(true)` if the item was deleted
    /// * `Ok(false)` if the item was not found
    /// * `Err` if there was an error deleting the item
    async fn delete(&self, item: &str) -> Result<bool>;

    /// Get a specific item from the denylist.
    ///
    /// # Arguments
    ///
    /// * `item` - The item to retrieve
    ///
    /// # Returns
    ///
    /// * `Ok(Some(item))` if the item was found
    /// * `Ok(None)` if the item was not found
    /// * `Err` if there was an error retrieving the item
    async fn get(&self, item: &str) -> Result<Option<DenyListItem>>;

    /// List items in the denylist with pagination.
    ///
    /// # Arguments
    ///
    /// * `cursor` - Pagination cursor specifying where to start and how many items to return
    ///
    /// # Returns
    ///
    /// * `Ok(page)` containing the items and next cursor if there are more items
    /// * `Err` if there was an error listing items
    async fn list(&self, cursor: DenyListCursor) -> Result<DenyListPage>;
}
