use async_trait::async_trait;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, trace};

use super::{Blueprint, BlueprintCache, BlueprintStorage, StorageResult};

/// A wrapper that adds caching to any BlueprintStorage implementation
pub struct CachedBlueprintStorage {
    storage: Arc<dyn BlueprintStorage>,
    cache: Arc<dyn BlueprintCache>,
    ttl: Duration,
}

impl CachedBlueprintStorage {
    /// Create a new cached storage wrapper
    pub fn new(
        storage: Arc<dyn BlueprintStorage>,
        cache: Arc<dyn BlueprintCache>,
        ttl: Duration,
    ) -> Self {
        Self {
            storage,
            cache,
            ttl,
        }
    }

    /// Create with default 60-second TTL
    pub fn with_default_ttl(
        storage: Arc<dyn BlueprintStorage>,
        cache: Arc<dyn BlueprintCache>,
    ) -> Self {
        Self::new(storage, cache, Duration::from_secs(60))
    }
}

#[async_trait]
impl BlueprintStorage for CachedBlueprintStorage {
    async fn get_blueprint(&self, aturi: &str) -> StorageResult<Option<Blueprint>> {
        // Try cache first
        if let Ok(Some(blueprint)) = self.cache.get(aturi).await {
            trace!(aturi = %aturi, "Blueprint found in cache");
            return Ok(Some(blueprint));
        }

        // Cache miss - fetch from storage
        debug!(aturi = %aturi, "Cache miss, fetching from storage");
        let blueprint = self.storage.get_blueprint(aturi).await?;

        // Cache the result if found
        if let Some(ref bp) = blueprint
            && let Err(e) = self.cache.set(aturi, bp.clone(), self.ttl).await
        {
            debug!(aturi = %aturi, error = ?e, "Failed to cache blueprint");
        }

        Ok(blueprint)
    }

    async fn list_blueprints(&self, did: Option<&str>) -> StorageResult<Vec<Blueprint>> {
        // List operations go directly to storage (no caching)
        self.storage.list_blueprints(did).await
    }

    async fn list_blueprints_paginated(
        &self,
        did: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> StorageResult<Vec<Blueprint>> {
        // Paginated list operations go directly to storage (no caching)
        self.storage
            .list_blueprints_paginated(did, limit, offset)
            .await
    }

    async fn count_blueprints(&self, did: Option<&str>) -> StorageResult<u32> {
        // Count operations go directly to storage (no caching)
        self.storage.count_blueprints(did).await
    }

    async fn create_blueprint(&self, blueprint: &Blueprint) -> StorageResult<()> {
        // Create in storage first
        self.storage.create_blueprint(blueprint).await?;

        // Then cache it
        if let Err(e) = self
            .cache
            .set(&blueprint.aturi, blueprint.clone(), self.ttl)
            .await
        {
            debug!(aturi = %blueprint.aturi, error = ?e, "Failed to cache new blueprint");
        }

        Ok(())
    }

    async fn update_blueprint(&self, blueprint: &Blueprint) -> StorageResult<()> {
        // Update in storage first
        self.storage.update_blueprint(blueprint).await?;

        // Invalidate old cache entry and set new one
        if let Err(e) = self.cache.invalidate(&blueprint.aturi).await {
            debug!(aturi = %blueprint.aturi, error = ?e, "Failed to invalidate cached blueprint");
        }

        // Cache the updated version
        if let Err(e) = self
            .cache
            .set(&blueprint.aturi, blueprint.clone(), self.ttl)
            .await
        {
            debug!(aturi = %blueprint.aturi, error = ?e, "Failed to cache updated blueprint");
        }

        Ok(())
    }

    async fn update_blueprint_error(
        &self,
        aturi: &str,
        enabled: bool,
        error: Option<String>,
    ) -> StorageResult<()> {
        // Update in storage
        self.storage
            .update_blueprint_error(aturi, enabled, error)
            .await?;

        // Invalidate cache since the blueprint has changed
        if let Err(e) = self.cache.invalidate(aturi).await {
            debug!(aturi = %aturi, error = ?e, "Failed to invalidate cached blueprint after error update");
        }

        Ok(())
    }

    async fn delete_blueprint(&self, aturi: &str) -> StorageResult<()> {
        // Delete from storage first
        self.storage.delete_blueprint(aturi).await?;

        // Then invalidate cache
        if let Err(e) = self.cache.invalidate(aturi).await {
            debug!(aturi = %aturi, error = ?e, "Failed to invalidate deleted blueprint from cache");
        }

        Ok(())
    }
}

// Integration tests for CachedBlueprintStorage are located in tests/
// directory and require DATABASE_URL to be set
