# Denylist Management System

This document describes the denylist management system for filtering and blocking unwanted content in the If This Then AT:// application.

## Overview

The denylist system provides efficient membership checking using PostgreSQL storage with bloom filter optimization. It consists of two main traits:

- `DenyListManager`: For checking if items exist in the denylist
- `DenyListStorage`: For CRUD operations on denylist items

## Implementations

The system provides multiple implementations:

- **PostgreSQL** (`postgres::PostgresDenyListStorage`): Full-featured implementation with bloom filter optimization
- **No-op** (`noop::NoopDenyListManager`, `noop::NoopDenyListStorage`): Disabled implementations for testing or when denylist functionality is not needed

## Architecture

```
┌─────────────────┐    ┌──────────────────┐    ┌─────────────────┐
│  Application    │───▶│  DenyListManager │───▶│  BloomFilter    │
│  Code           │    │                  │    │  (Fast Check)   │
└─────────────────┘    └──────────────────┘    └─────────────────┘
                                │                        │
                                ▼                        ▼
                       ┌──────────────────┐    ┌─────────────────┐
                       │  DenyListStorage │───▶│  PostgreSQL     │
                       │                  │    │  (Persistent)   │
                       └──────────────────┘    └─────────────────┘
```

## Usage

### 1. Setup

First, ensure the database migration has been applied:

```bash
sqlx migrate run
```

### 2. Create Storage Instance

```rust
use ifthisthenat::denylist::postgres::PostgresDenyListStorage;
use sqlx::PgPool;

// Create storage with default bloom filter settings
let storage = PostgresDenyListStorage::new(
    pool,           // PostgreSQL connection pool
    Some(100_000),  // Expected number of items (optional, default: 100,000)
    Some(0.01),     // False positive rate (optional, default: 0.01)
);

// Initialize bloom filter with existing data
storage.init().await?;
```

### 3. Basic Operations

```rust
use ifthisthenat::denylist::{DenyListManager, DenyListStorage, DenyListCursor};

// Add items to denylist
storage.insert("did:plc:malicious123", Some("Known spam account")).await?;
storage.insert("handle.bad.actor", Some("Abusive behavior")).await?;
storage.insert("https://phishing.site", None).await?;

// Check if items are denied (very fast due to bloom filter)
let is_denied = storage.exists("did:plc:malicious123").await?;
if is_denied {
    println!("User is on denylist, blocking action");
}

// Get detailed information about a denied item
if let Some(item) = storage.get("did:plc:malicious123").await? {
    println!("Denied: {} - Reason: {:?}", item.item, item.note);
    println!("Added: {}", item.created_at);
}

// List denied items with pagination
let cursor = DenyListCursor {
    after: None,    // Start from beginning
    limit: 50,      // Get 50 items per page
};

let page = storage.list(cursor).await?;
for item in page.items {
    println!("Denied: {} ({})", item.item, item.created_at);
}

// Remove item from denylist
let deleted = storage.delete("handle.bad.actor").await?;
if deleted {
    println!("Item removed from denylist");
}
```

## Performance Characteristics

### Bloom Filter Benefits

- **Fast negative responses**: If bloom filter says "no", item is definitely not in denylist
- **Memory efficient**: Uses minimal memory regardless of denylist size
- **Thread-safe**: Concurrent reads are very fast

### Important Notes

1. **False positives possible**: Bloom filter may say "yes" even if item isn't in denylist
   - When bloom filter says "yes", a database query is performed to verify
   - False positive rate is configurable (default: 1%)

2. **Delete behavior**: Deleted items remain in bloom filter
   - This may cause false positives for deleted items
   - Database queries will still return correct results
   - Consider rebuilding bloom filter periodically if many deletions occur

3. **Initialization**: Call `init()` after creating storage to populate bloom filter

## Database Schema

The denylist table has the following structure:

```sql
CREATE TABLE denylist (
    item TEXT PRIMARY KEY,           -- The denied item (DID, handle, URL, etc.)
    created_at TIMESTAMPTZ NOT NULL, -- When item was added
    note TEXT                        -- Optional reason for denial
);

CREATE INDEX idx_denylist_created_at ON denylist(created_at DESC);
```

## Use Cases

### 1. User Blocking

```rust
// Block a malicious user
storage.insert("did:plc:spammer123", Some("Multiple spam reports")).await?;

// Check before processing user actions
if storage.exists(user_did).await? {
    return Err("User is blocked");
}
```

### 2. Content Filtering

```rust
// Block malicious domains
storage.insert("phishing.example.com", Some("Known phishing site")).await?;

// Check URLs in posts
for url in extract_urls(&post_text) {
    if let Some(domain) = get_domain(&url) {
        if storage.exists(&domain).await? {
            return Err("Post contains blocked domain");
        }
    }
}
```

### 3. Handle Monitoring

```rust
// Block problematic handles
storage.insert("offensive.handle.bsky.social", Some("ToS violation")).await?;

// Check during handle resolution
if storage.exists(&handle).await? {
    return Err("Handle is blocked");
}
```

## Configuration

### Bloom Filter Tuning

Choose bloom filter parameters based on your use case:

```rust
// High accuracy (lower false positive rate)
let storage = PostgresDenyListStorage::new(pool, Some(100_000), Some(0.001));

// Memory efficient (higher false positive rate)
let storage = PostgresDenyListStorage::new(pool, Some(50_000), Some(0.05));

// Default balanced settings
let storage = PostgresDenyListStorage::new(pool, None, None);
```

### Performance Monitoring

Monitor bloom filter effectiveness:

```rust
// Track metrics in your application
let exists_result = storage.exists(item).await?;
if exists_result {
    // This could be a true positive or false positive
    metrics::counter!("denylist.bloom_positive").increment(1);

    let db_result = storage.get(item).await?;
    if db_result.is_some() {
        metrics::counter!("denylist.true_positive").increment(1);
    } else {
        metrics::counter!("denylist.false_positive").increment(1);
    }
}
```

## Thread Safety

All operations are thread-safe and can be called concurrently:

- Multiple threads can check existence simultaneously
- Insertions are atomic and thread-safe
- Bloom filter updates are protected by RwLock

## Error Handling

All operations return `Result<T, anyhow::Error>`. Common error cases:

- Database connection failures
- Invalid SQL queries
- Bloom filter initialization failures

```rust
match storage.exists("test-item").await {
    Ok(exists) => println!("Item exists: {}", exists),
    Err(e) => eprintln!("Denylist check failed: {}", e),
}
```

## No-op Implementation

For testing or when denylist functionality needs to be disabled, use the no-op implementations:

### NoopDenyListManager

```rust
use ifthisthenat::denylist::{DenyListManager, noop::NoopDenyListManager};

let manager = NoopDenyListManager::new();

// Always returns false - nothing is ever denied
assert!(!manager.exists("any-item").await?);
assert!(!manager.exists("malicious-user").await?);
```

### NoopDenyListStorage

```rust
use ifthisthenat::denylist::{DenyListStorage, DenyListCursor, noop::NoopDenyListStorage};

let storage = NoopDenyListStorage::new();

// Operations succeed but have no effect
storage.insert("test-item", Some("note")).await?;
assert!(storage.get("test-item").await?.is_none());

// List always returns empty
let cursor = DenyListCursor { after: None, limit: 10 };
let page = storage.list(cursor).await?;
assert!(page.items.is_empty());
```

### Use Cases for No-op Implementation

1. **Testing**: When you need predictable behavior that never denies anything
2. **Feature flags**: Dynamically disable denylist functionality
3. **Development**: Simplify local development by removing external dependencies
4. **Graceful degradation**: Fall back when database is unavailable

```rust
use ifthisthenat::denylist::{DenyListManager, postgres::PostgresDenyListStorage, noop::NoopDenyListManager};

// Choose implementation based on configuration
let manager: Box<dyn DenyListManager> = if config.denylist_enabled {
    Box::new(PostgresDenyListStorage::new(pool, None, None))
} else {
    Box::new(NoopDenyListManager::new())
};
```