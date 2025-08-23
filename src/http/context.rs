use atproto_identity::{model::Document, resolve::IdentityResolver};
use axum::extract::FromRef;
use axum_template::engine::Engine;
use cookie::Key;
use sqlx::PgPool;
use std::{ops::Deref, sync::Arc};
use tokio::sync::mpsc;

use crate::{
    atproto::auth::ServiceAccessTokenManager,
    config::Config,
    storage::{
        blueprint::{BlueprintStorage, PostgresBlueprintStorage},
        blueprint_cache::BlueprintCache,
        blueprint_cache_memory::InMemoryBlueprintCache,
        blueprint_cache_redis::RedisBlueprintCache,
        cached_blueprint_storage::CachedBlueprintStorage,
        evaluation_result::EvaluationResultStorage,
        node::{NodeStorage, PostgresNodeStorage},
        oauth_request::{
            OAuthRequestStorage as AppOAuthRequestStorage, PostgresOAuthRequestStorage,
        },
        prototype::{PostgresPrototypeStorage, PrototypeStorage},
        session::{PostgresSessionStorage, SessionStorage},
    },
    tasks::BlueprintWork,
    throttle::BlueprintThrottler,
};
use deadpool_redis::Pool as RedisPool;
use std::time::Duration;

#[cfg(feature = "reload")]
use minijinja_autoreload::AutoReloader;

#[cfg(feature = "reload")]
pub type AppEngine = Engine<AutoReloader>;

#[cfg(feature = "embed")]
use minijinja::Environment;

#[cfg(feature = "embed")]
pub type AppEngine = Engine<Environment<'static>>;

pub struct InnerWebContext {
    pub(crate) config: Config,
    pub(crate) http_client: reqwest::Client,
    pub(crate) identity_resolver: Arc<dyn IdentityResolver>,
    pub(crate) blueprint_storage: Arc<dyn BlueprintStorage>,
    pub(crate) node_storage: Arc<dyn NodeStorage>,
    pub(crate) prototype_storage: Arc<dyn PrototypeStorage>,
    pub(crate) session_storage: Arc<dyn SessionStorage>,
    pub(crate) oauth_request_storage: Arc<dyn AppOAuthRequestStorage>,
    pub(crate) evaluation_result_storage: Arc<dyn EvaluationResultStorage>,
    pub(crate) service_document: Document,
    pub(crate) engine: AppEngine,
    pub(crate) blueprint_sender: Option<mpsc::Sender<BlueprintWork>>,
    pub(crate) cache_reload_sender: Option<mpsc::Sender<()>>,
    pub(crate) throttler: Arc<dyn BlueprintThrottler>,
    pub(crate) service_token_manager: Arc<ServiceAccessTokenManager>,
}

#[derive(Clone, FromRef)]
pub struct WebContext(pub(crate) Arc<InnerWebContext>);

impl Deref for WebContext {
    type Target = InnerWebContext;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl FromRef<WebContext> for Key {
    fn from_ref(context: &WebContext) -> Self {
        context.0.config.http_cookie_key.as_ref().clone()
    }
}

impl WebContext {
    pub fn session_storage(&self) -> &Arc<dyn SessionStorage> {
        &self.0.session_storage
    }

    pub fn blueprint_storage(&self) -> &Arc<dyn BlueprintStorage> {
        &self.0.blueprint_storage
    }

    pub fn node_storage(&self) -> &Arc<dyn NodeStorage> {
        &self.0.node_storage
    }

    pub fn blueprint_sender(&self) -> Option<&mpsc::Sender<BlueprintWork>> {
        self.0.blueprint_sender.as_ref()
    }

    pub fn throttler(&self) -> &Arc<dyn BlueprintThrottler> {
        &self.0.throttler
    }

    pub async fn trigger_blueprint_cache_reload(&self) -> bool {
        if let Some(sender) = &self.0.cache_reload_sender {
            // Send reload request, ignore if receiver is dropped
            let _ = sender.send(()).await;
            true
        } else {
            false
        }
    }

    #[allow(clippy::too_many_arguments)]
    // Note: WebContext now requires a ServiceAccessTokenManager, so it can only be created
    // via with_senders_throttler_and_storage which accepts the token manager as a parameter.
    #[allow(clippy::too_many_arguments)]
    pub fn with_senders_throttler_and_storage(
        config: Config,
        http_client: reqwest::Client,
        engine: AppEngine,
        identity_resolver: Arc<dyn IdentityResolver>,
        pool: PgPool,
        service_document: Document,
        blueprint_sender: Option<mpsc::Sender<BlueprintWork>>,
        cache_reload_sender: Option<mpsc::Sender<()>>,
        redis_pool: Option<RedisPool>,
        throttler: Arc<dyn BlueprintThrottler>,
        evaluation_result_storage: Arc<dyn EvaluationResultStorage>,
        service_token_manager: Arc<ServiceAccessTokenManager>,
    ) -> Self {
        // Create base blueprint storage
        let base_storage: Arc<dyn BlueprintStorage> =
            Arc::new(PostgresBlueprintStorage::new(pool.clone()));

        // Wrap with cache if Redis is available, otherwise use in-memory cache
        let blueprint_storage: Arc<dyn BlueprintStorage> = if let Some(redis_pool) = redis_pool {
            // Use Redis cache
            let cache: Arc<dyn BlueprintCache> = Arc::new(RedisBlueprintCache::new(
                redis_pool,
                "blueprint:cache:".to_string(),
                Duration::from_secs(60),
            ));
            Arc::new(CachedBlueprintStorage::new(
                base_storage,
                cache,
                Duration::from_secs(60),
            ))
        } else {
            // Use in-memory cache
            let cache: Arc<dyn BlueprintCache> =
                Arc::new(InMemoryBlueprintCache::new(1000, Duration::from_secs(60)));
            Arc::new(CachedBlueprintStorage::new(
                base_storage,
                cache,
                Duration::from_secs(60),
            ))
        };
        let node_storage: Arc<dyn NodeStorage> = Arc::new(PostgresNodeStorage::new(pool.clone()));
        let prototype_storage: Arc<dyn PrototypeStorage> =
            Arc::new(PostgresPrototypeStorage::new(Arc::new(pool.clone())));
        let session_storage: Arc<dyn SessionStorage> =
            Arc::new(PostgresSessionStorage::new(pool.clone()));
        let oauth_request_storage: Arc<dyn AppOAuthRequestStorage> =
            Arc::new(PostgresOAuthRequestStorage::new(pool.clone()));

        Self(Arc::new(InnerWebContext {
            config,
            http_client,
            identity_resolver,
            blueprint_storage,
            node_storage,
            prototype_storage,
            session_storage,
            oauth_request_storage,
            evaluation_result_storage,
            service_document,
            engine,
            blueprint_sender,
            cache_reload_sender,
            throttler,
            service_token_manager,
        }))
    }
}

macro_rules! impl_from_ref {
    ($context:ty, $field:ident, $type:ty) => {
        impl FromRef<$context> for $type {
            fn from_ref(context: &$context) -> Self {
                context.0.$field.clone()
            }
        }
    };
}

impl_from_ref!(WebContext, service_document, Document);
impl_from_ref!(WebContext, identity_resolver, Arc<dyn IdentityResolver>);
