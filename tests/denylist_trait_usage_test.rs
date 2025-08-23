//! Test that demonstrates interchangeable usage of denylist implementations.

use anyhow::Result;
use ifthisthenat::denylist::{
    DenyListCursor, DenyListManager, DenyListStorage,
    noop::{NoopDenyListManager, NoopDenyListStorage},
};

/// Example function that works with any DenyListManager implementation
async fn check_user_allowed(manager: &dyn DenyListManager, user_did: &str) -> Result<bool> {
    let is_denied = manager.exists(user_did).await?;
    Ok(!is_denied)
}

/// Example function that works with any DenyListStorage implementation
async fn manage_denylist(storage: &dyn DenyListStorage, action: &str, item: &str) -> Result<()> {
    match action {
        "add" => storage.insert(item, Some("Added for testing")).await?,
        "remove" => {
            storage.delete(item).await?;
        }
        "check" => {
            let item_data = storage.get(item).await?;
            println!("Item data: {:?}", item_data);
        }
        _ => return Err(anyhow::anyhow!("Unknown action: {}", action)),
    }
    Ok(())
}

#[tokio::test]
async fn test_trait_object_usage() -> Result<()> {
    // Test with noop manager
    let noop_manager = NoopDenyListManager::new();

    // Should always allow users (never deny)
    assert!(check_user_allowed(&noop_manager, "did:plc:test123").await?);
    assert!(check_user_allowed(&noop_manager, "did:plc:malicious").await?);

    Ok(())
}

#[tokio::test]
async fn test_storage_trait_object_usage() -> Result<()> {
    let noop_storage = NoopDenyListStorage::new();

    // Test various operations through trait object
    manage_denylist(&noop_storage, "add", "test-item").await?;
    manage_denylist(&noop_storage, "check", "test-item").await?;
    manage_denylist(&noop_storage, "remove", "test-item").await?;

    Ok(())
}

#[tokio::test]
async fn test_dynamic_implementation_selection() -> Result<()> {
    // Simulate choosing implementation based on configuration
    let use_noop = true; // In real code, this would come from config

    let manager: Box<dyn DenyListManager> = if use_noop {
        Box::new(NoopDenyListManager::new())
    } else {
        // In real code, you would create PostgresDenyListStorage here
        Box::new(NoopDenyListManager::new())
    };

    // Use the manager regardless of implementation
    let is_allowed = check_user_allowed(manager.as_ref(), "did:plc:user123").await?;
    assert!(is_allowed); // Noop always allows

    Ok(())
}

#[tokio::test]
async fn test_storage_as_manager() -> Result<()> {
    // Test that NoopDenyListStorage can be used as DenyListManager
    let storage = NoopDenyListStorage::new();

    // Use storage as manager
    let is_allowed = check_user_allowed(&storage, "did:plc:test").await?;
    assert!(is_allowed);

    Ok(())
}

#[tokio::test]
async fn test_cursor_pagination_noop() -> Result<()> {
    let storage = NoopDenyListStorage::new();

    // Test pagination behavior
    let cursor = DenyListCursor {
        after: None,
        limit: 10,
    };

    let page = storage.list(cursor).await?;
    assert!(page.items.is_empty());
    assert!(page.next_cursor.is_none());

    // Test with different cursor
    let cursor = DenyListCursor {
        after: Some("some-item".to_string()),
        limit: 50,
    };

    let page = storage.list(cursor).await?;
    assert!(page.items.is_empty());
    assert!(page.next_cursor.is_none());

    Ok(())
}
