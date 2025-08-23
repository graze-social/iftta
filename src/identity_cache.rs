use anyhow::Result;
use async_trait::async_trait;
use atproto_identity::{model::Document, resolve::IdentityResolver, storage::DidDocumentStorage};
use chrono::{Duration, Utc};
use lru::LruCache;
use std::num::NonZeroUsize;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::warn;

#[derive(Clone, Debug)]
pub struct CacheConfig {
    pub memory_cache_size: usize,
    pub memory_ttl_seconds: i64,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            memory_cache_size: 1000,
            memory_ttl_seconds: 300,
        }
    }
}

#[derive(Clone)]
struct CachedDocument {
    document: Document,
    cached_at: chrono::DateTime<Utc>,
}

impl CachedDocument {
    fn is_expired(&self, ttl_seconds: i64) -> bool {
        let age = Utc::now() - self.cached_at;
        age > Duration::seconds(ttl_seconds)
    }
}

pub struct CachingIdentityResolver<R, S>
where
    R: IdentityResolver + 'static,
    S: DidDocumentStorage + 'static,
{
    base_resolver: Arc<R>,
    storage: Arc<S>,
    memory_cache: Arc<RwLock<LruCache<String, CachedDocument>>>,
    config: CacheConfig,
}

impl<R, S> CachingIdentityResolver<R, S>
where
    R: IdentityResolver + 'static,
    S: DidDocumentStorage + 'static,
{
    pub fn new(base_resolver: Arc<R>, storage: Arc<S>) -> Self {
        Self::with_config(base_resolver, storage, CacheConfig::default())
    }

    pub fn with_config(base_resolver: Arc<R>, storage: Arc<S>, config: CacheConfig) -> Self {
        let cache_size = NonZeroUsize::new(config.memory_cache_size.max(1))
            .expect("Cache size must be at least 1");

        Self {
            base_resolver,
            storage,
            memory_cache: Arc::new(RwLock::new(LruCache::new(cache_size))),
            config,
        }
    }

    fn normalize_subject(subject: &str) -> String {
        if subject.starts_with("did:") {
            subject.to_string()
        } else {
            subject.to_lowercase()
        }
    }
}

#[async_trait]
impl<R, S> IdentityResolver for CachingIdentityResolver<R, S>
where
    R: IdentityResolver + Send + Sync + 'static,
    S: DidDocumentStorage + Send + Sync + 'static,
{
    async fn resolve(&self, subject: &str) -> Result<Document> {
        let normalized = Self::normalize_subject(subject);

        {
            let mut cache = self.memory_cache.write().await;
            if let Some(cached) = cache.get(&normalized) {
                if !cached.is_expired(self.config.memory_ttl_seconds) {
                    return Ok(cached.document.clone());
                }
                cache.pop(&normalized);
            }
        }

        if let Ok(Some(stored)) = self.storage.get_document_by_did(&normalized).await {
            let cached_doc = CachedDocument {
                document: stored.clone(),
                cached_at: Utc::now(),
            };
            let mut cache = self.memory_cache.write().await;
            cache.put(normalized.clone(), cached_doc);
            return Ok(stored);
        }

        let document = self.base_resolver.resolve(subject).await?;

        let cached_doc = CachedDocument {
            document: document.clone(),
            cached_at: Utc::now(),
        };
        let mut cache = self.memory_cache.write().await;
        cache.put(normalized.clone(), cached_doc);

        if let Err(e) = self.storage.store_document(document.clone()).await {
            warn!(error = ?e, did = %normalized, "Failed to cache DID document in storage");
        }

        Ok(document)
    }
}
