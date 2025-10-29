use anyhow::Result;
use atproto_identity::{
    model::Document,
    resolve::{
        HickoryDnsResolver, IdentityResolver, InnerIdentityResolver, SharedIdentityResolver,
    },
    storage_lru::LruDidDocumentStorage,
};
use atproto_jetstream::{Consumer as JetstreamConsumer, ConsumerTaskConfig, EventHandler};
use deadpool_redis::Pool as RedisPool;
use ifthisthenat::{
    ServiceAccessTokenManager,
    config::Config,
    consumer::{BlueprintEvent, Consumer, CursorWriterHandler},
    denylist::{DenyListManager, noop::NoopDenyListManager, postgres::PostgresDenyListStorage},
    engine::{
        node_evaluator_factory::NodeEvaluatorFactory, node_type_condition::ConditionEvaluator,
        node_type_debug_action::DebugActionEvaluator, node_type_facet_text::FacetTextEvaluator,
        node_type_get_record::GetRecordEvaluator,
        node_type_jetstream_entry::JetstreamEntryEvaluator,
        node_type_parse_aturi::ParseAturiEvaluator,
        node_type_periodic_entry::PeriodicEntryEvaluator,
        node_type_publish_record::PublishRecordEvaluator,
        node_type_publish_webhook_direct::PublishWebhookDirectEvaluator,
        node_type_publish_webhook_queue::PublishWebhookQueueEvaluator,
        node_type_sentiment_analysis::SentimentAnalysisEvaluator,
        node_type_transform::TransformEvaluator, node_type_webhook_entry::WebhookEntryEvaluator,
        node_type_zap_entry::ZapEntryEvaluator,
    },
    http::{context::WebContext, server::build_router},
    identity_cache::{CacheConfig, CachingIdentityResolver},
    leadership::{LeadershipElection, LeadershipManager, RedisLeadershipElection},
    processor::BlueprintProcessor,
    queue_adapter::MpscQueueAdapter,
    storage::cache::create_cache_pool,
    storage::evaluation_result::{
        EvaluationResultStorage, FilesystemEvaluationResultStorage, NoopEvaluationResultStorage,
        TracingEvaluationResultStorage,
    },
    storage::scheduler_state::PostgresSchedulerStateStorage,
    tasks::{
        BlueprintEvaluationTask, SchedulerTask, WebhookTask, create_blueprint_queue_adapter,
        manager::spawn_cancellable_task,
        webhook::{WebhookTaskConfig, WebhookWork},
    },
    throttle::{
        BlueprintThrottler, NoOpBlueprintThrottler, RedisBlueprintThrottler,
        RedisPerIdentityBlueprintThrottler,
    },
};
use sqlx::postgres::PgPoolOptions;
use std::{env, num::NonZeroUsize, sync::Arc, time::Duration};
use tokio::net::TcpListener;
use tokio::signal;
use tokio::sync::mpsc;
use tokio_util::{sync::CancellationToken, task::TaskTracker};
use tracing_subscriber::prelude::*;

#[cfg(feature = "embed")]
use ifthisthenat::http::{context::AppEngine, templates::build_env};

#[cfg(feature = "reload")]
use ifthisthenat::http::{context::AppEngine, templates::build_env};

#[tokio::main]
async fn main() -> Result<()> {
    let version = ifthisthenat::config::version()?;

    env::args().for_each(|arg| {
        if arg == "--version" {
            println!("{version}");
            std::process::exit(0);
        }
    });

    // Load config early to check if Sentry is enabled
    let config = Config::new()?;

    // Initialize Sentry if configured
    // let mut _guard = if let Some(dsn) = &config.sentry_dsn {
        // let guard = sentry::init((
        //     dsn.as_str(),
        //     sentry::ClientOptions {
        //         release: sentry::release_name!(),
        //         environment: config.sentry_environment.clone().map(Into::into),
        //         debug: config.sentry_debug,
        //         traces_sample_rate: config.sentry_traces_sample_rate,
        //         attach_stacktrace: true,
        //         ..Default::default()
        //     }
        //     .add_integration(sentry::integrations::panic::PanicIntegration::default()),
        // ));

        // if guard.is_enabled() {
        //     println!(
        //         "Sentry initialized - environment: {:?}, traces_sample_rate: {}",
        //         config.sentry_environment, config.sentry_traces_sample_rate
        //     );

        //     // Set user context if admin DIDs are configured
        //     if !config.admin_dids.is_empty() {
        //         sentry::configure_scope(|scope| {
        //             scope.set_tag("service", "ifthisthenat");
        //             scope.set_tag("version", &version);
        //         });
        //     }

        //     Some(guard)
        // } else {
        //     eprintln!("Sentry initialization failed");
        //     None
        // }
        // None
    // } else {
    //     println!("Sentry not configured, skipping initialization");
    //     None
    // };

    // Validate OAuth scopes are compatible with publish_record collection constraints
    if let Err(e) = ifthisthenat::validation::Validator::validate_oauth_scopes_for_collections(
        Some(&config.oauth.scope),
        &config.disabled_node_types,
        &config.allowed_publish_collections,
    ) {
        eprintln!("Configuration validation failed: {}", e);
        std::process::exit(1);
    }

    // Create Redis pool if Redis is configured
    let redis_pool: Option<RedisPool> = if let Some(redis_url) = &config.redis_url {
        match create_cache_pool(redis_url) {
            Ok(pool) => {
                tracing::info!("Redis pool created successfully");
                Some(pool)
            }
            Err(e) => {
                tracing::warn!(error = ?e, "Failed to create Redis pool, Redis features will be disabled");
                None
            }
        }
    } else {
        tracing::info!("Redis not configured");
        None
    };

    // Enhanced tracing setup with multiple layers
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    let env_filter = tracing_subscriber::EnvFilter::new(
        std::env::var("RUST_LOG")
            .unwrap_or_else(|_| "ifthisthenat=info,tower_http=info,sqlx=warn".into()),
    );

    // Configure output format based on environment
    let fmt_layer = if std::env::var("JSON_LOGS").is_ok() {
        tracing_subscriber::fmt::layer()
            .json()
            .with_current_span(true)
            .with_span_list(true)
            .boxed()
    } else {
        tracing_subscriber::fmt::layer()
            .pretty()
            .with_thread_ids(true)
            .with_thread_names(true)
            .boxed()
    };

    // Build the subscriber with optional Sentry layer
    let subscriber = tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt_layer);

    // Add Sentry layer if enabled
    if config.sentry_dsn.is_some() {
        // subscriber.with(sentry_tracing::layer()).init();
        subscriber.init();
    } else {
        subscriber.init();
    }

    tracing::info!(version = %version, "Starting ifthisthenat application");
    let external_base = config.external_base.clone();

    let pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(&config.database_url)
        .await?;

    sqlx::migrate!("./migrations").run(&pool).await?;

    // Create denylist manager based on configuration
    let denylist_manager: Arc<dyn DenyListManager> = match config.denylist_type.as_str() {
        "postgres" => {
            tracing::info!("Creating PostgreSQL denylist manager with bloom filter optimization");
            let storage = PostgresDenyListStorage::new(
                pool.clone(),
                None, // Use default expected items (100,000)
                None, // Use default false positive rate (0.01)
            );

            // Initialize the bloom filter with existing data
            if let Err(e) = storage.init().await {
                tracing::warn!(error = ?e, "Failed to initialize denylist bloom filter, continuing without preloading");
            } else {
                tracing::info!("Denylist bloom filter initialized successfully");
            }

            Arc::new(storage)
        }
        _ => {
            tracing::info!("Using no-op denylist manager (denylist disabled)");
            Arc::new(NoopDenyListManager::new())
        }
    };

    let http_client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(config.user_agent.clone())
        .timeout(*config.http_client_timeout.as_ref())
        .build()?;

    let dns_resolver = HickoryDnsResolver::create_resolver(&[]);

    let did_document_storage = Arc::new(LruDidDocumentStorage::new(
        NonZeroUsize::new(config.identity_cache_size.as_ref().to_owned()).unwrap(),
    ));

    let base_resolver = Arc::new(SharedIdentityResolver(Arc::new(InnerIdentityResolver {
        dns_resolver: Arc::new(dns_resolver),
        http_client: http_client.clone(),
        plc_hostname: config.plc_hostname.clone(),
    })));

    let identity_resolver = Arc::new(CachingIdentityResolver::with_config(
        base_resolver.clone(),
        did_document_storage.clone(),
        CacheConfig {
            memory_cache_size: config.identity_cache_size.as_ref().to_owned(),
            memory_ttl_seconds: config.identity_cache_ttl_minutes.to_seconds(),
        },
    ));

    let service_document = Document::builder()
        .id(config.issuer_did.as_str().to_string())
        .build()
        .expect("Failed to build service document");

    // Setup template engine
    let template_env = {
        #[cfg(feature = "embed")]
        {
            AppEngine::from(build_env(
                config.external_base.clone(),
                env!("CARGO_PKG_VERSION").to_string(),
            ))
        }

        #[cfg(feature = "reload")]
        {
            AppEngine::from(build_env())
        }

        #[cfg(not(any(feature = "reload", feature = "embed")))]
        {
            panic("one of 'reload' or 'embed' must be set")
        }
    };

    // Create task tracker and cancellation token first
    let tracker = TaskTracker::new();
    let token = CancellationToken::new();

    // Create Redis client from pool if available (needed for leadership elections)
    let redis_client: Option<deadpool_redis::redis::Client> =
        if let Some(_redis_pool) = redis_pool.as_ref() {
            // Create a Redis client from the pool's configuration
            // This is safe because we know the pool is configured correctly
            Some(
                deadpool_redis::redis::Client::open(config.redis_url.as_ref().unwrap().as_str())
                    .expect("Failed to create Redis client from URL"),
            )
        } else {
            None
        };

    // Setup webhook processor leadership election
    let webhook_leadership: Option<Arc<dyn LeadershipElection>> = if config
        .webhook_leadership
        .enabled
        && redis_client.is_some()
    {
        let redis_client = redis_client.clone().unwrap();
        let leadership_config = config.webhook_leadership.clone();
        let election = Arc::new(RedisLeadershipElection::new(
            redis_client,
            leadership_config.clone(),
        ));

        // Start leadership election manager for webhook processor
        let manager = LeadershipManager::new(election.clone(), leadership_config);

        let election_for_shutdown = election.clone();
        spawn_cancellable_task(&tracker, token.clone(), |cancel_token| async move {
            tokio::select! {
                result = manager.start_election_loop() => {
                    if let Err(e) = result {
                        tracing::error!(error = ?e, scope = "webhook_processor", "Leadership election loop failed");
                        Err(anyhow::anyhow!(e))
                    } else {
                        Ok(())
                    }
                }
                _ = cancel_token.cancelled() => {
                    tracing::info!(scope = "webhook_processor", "Leadership election loop cancelled, releasing leadership");
                    if let Err(e) = election_for_shutdown.release_leadership().await {
                        tracing::error!(error = ?e, scope = "webhook_processor", "Failed to gracefully release leadership");
                    }
                    Ok(())
                }
            }
        });

        tracing::info!(
            instance_id = %config.webhook_leadership.instance_id,
            election_key = %config.webhook_leadership.election_key,
            "Webhook processor leadership election enabled"
        );
        Some(election as Arc<dyn LeadershipElection>)
    } else {
        if !config.webhook_leadership.enabled {
            tracing::info!("Webhook processor leadership election disabled by configuration");
        } else {
            tracing::info!("Webhook processor leadership election disabled - Redis not available");
        }
        None
    };

    // Setup webhook queue if enabled and select appropriate evaluator
    let webhook_evaluator: Arc<dyn ifthisthenat::engine::evaluator::NodeEvaluator> = if config
        .webhook_queue
        .enabled
    {
        let (webhook_sender, webhook_receiver) =
            mpsc::channel::<WebhookWork>(config.webhook_queue.queue_size);

        // Create webhook task configuration
        let webhook_config = WebhookTaskConfig {
            max_retries: config.webhook_queue.max_retries,
            retry_delay_ms: config.webhook_queue.retry_delay_ms,
            max_concurrent: config.webhook_queue.max_concurrent,
            default_timeout_ms: config.webhook_queue.default_timeout_ms,
            log_bodies: config.webhook_queue.log_bodies,
        };

        // Create webhook queue adapter
        let adapter = Arc::new(MpscQueueAdapter::from_channel(
            webhook_sender.clone(),
            webhook_receiver,
        ));

        // Create and start webhook task with optional leadership election
        let webhook_task = WebhookTask::with_leadership(
            adapter,
            Arc::new(http_client.clone()),
            token.clone(),
            webhook_config,
            webhook_leadership.clone(),
        );

        // Spawn the webhook task
        spawn_cancellable_task(&tracker, token.clone(), |cancel_token| async move {
            tokio::select! {
                result = webhook_task.run() => {
                    if let Err(e) = result {
                        tracing::error!(error = ?e, "Webhook processor task failed");
                        Err(anyhow::anyhow!(e))
                    } else {
                        Ok(())
                    }
                }
                _ = cancel_token.cancelled() => {
                    tracing::info!("Webhook processor task cancelled");
                    Ok(())
                }
            }
        });

        tracing::info!(
            "Webhook queue enabled with {} max concurrent requests - using PublishWebhookQueueEvaluator",
            config.webhook_queue.max_concurrent
        );
        Arc::new(PublishWebhookQueueEvaluator::new(webhook_sender))
    } else {
        tracing::info!(
            "Webhook queue disabled - using PublishWebhookDirectEvaluator for synchronous delivery"
        );
        Arc::new(PublishWebhookDirectEvaluator::new(Arc::new(
            http_client.clone(),
        )))
    };

    // Setup blueprint evaluation leadership election
    let blueprint_leadership: Option<Arc<dyn LeadershipElection>> = if config
        .blueprint_leadership
        .enabled
        && redis_client.is_some()
    {
        let redis_client = redis_client.clone().unwrap();
        let leadership_config = config.blueprint_leadership.clone();
        let election = Arc::new(RedisLeadershipElection::new(
            redis_client,
            leadership_config.clone(),
        ));

        // Start leadership election manager for blueprint evaluation
        let manager = LeadershipManager::new(election.clone(), leadership_config);
        let election_for_shutdown = election.clone();

        spawn_cancellable_task(&tracker, token.clone(), |cancel_token| async move {
            tokio::select! {
                result = manager.start_election_loop() => {
                    if let Err(e) = result {
                        tracing::error!(error = ?e, scope = "blueprint_evaluation", "Leadership election loop failed");
                        Err(anyhow::anyhow!(e))
                    } else {
                        Ok(())
                    }
                }
                _ = cancel_token.cancelled() => {
                    tracing::info!(scope = "blueprint_evaluation", "Leadership election loop cancelled, releasing leadership");
                    if let Err(e) = election_for_shutdown.release_leadership().await {
                        tracing::error!(error = ?e, scope = "blueprint_evaluation", "Failed to gracefully release leadership");
                    }
                    Ok(())
                }
            }
        });

        tracing::info!(
            instance_id = %config.blueprint_leadership.instance_id,
            election_key = %config.blueprint_leadership.election_key,
            "Blueprint evaluation leadership election enabled"
        );
        Some(election as Arc<dyn LeadershipElection>)
    } else {
        if !config.blueprint_leadership.enabled {
            tracing::info!("Blueprint evaluation leadership election disabled by configuration");
        } else {
            tracing::info!(
                "Blueprint evaluation leadership election disabled - Redis not available"
            );
        }
        None
    };

    // Setup scheduler leadership election
    let scheduler_leadership: Option<Arc<dyn LeadershipElection>> = if config
        .scheduler_leadership
        .enabled
        && redis_client.is_some()
    {
        let redis_client = redis_client.clone().unwrap();
        let leadership_config = config.scheduler_leadership.clone();
        let election = Arc::new(RedisLeadershipElection::new(
            redis_client,
            leadership_config.clone(),
        ));

        // Start leadership election manager for scheduler
        let manager = LeadershipManager::new(election.clone(), leadership_config);
        let election_for_shutdown = election.clone();

        spawn_cancellable_task(&tracker, token.clone(), |cancel_token| async move {
            tokio::select! {
                result = manager.start_election_loop() => {
                    if let Err(e) = result {
                        tracing::error!(error = ?e, scope = "scheduler", "Leadership election loop failed");
                        Err(anyhow::anyhow!(e))
                    } else {
                        Ok(())
                    }
                }
                _ = cancel_token.cancelled() => {
                    tracing::info!(scope = "scheduler", "Leadership election loop cancelled, releasing leadership");
                    if let Err(e) = election_for_shutdown.release_leadership().await {
                        tracing::error!(error = ?e, scope = "scheduler", "Failed to gracefully release leadership");
                    }
                    Ok(())
                }
            }
        });

        tracing::info!(
            instance_id = %config.scheduler_leadership.instance_id,
            election_key = %config.scheduler_leadership.election_key,
            "Scheduler leadership election enabled"
        );
        Some(election as Arc<dyn LeadershipElection>)
    } else {
        if !config.scheduler_leadership.enabled {
            tracing::info!("Scheduler leadership election disabled by configuration");
        } else {
            tracing::info!("Scheduler leadership election disabled - Redis not available");
        }
        None
    };

    // Create ServiceAccessTokenManager for service-to-service authentication
    let service_token_manager = {
        let manager = ServiceAccessTokenManager::new(
            Arc::new(http_client.clone()),
            config.oauth.hostname.clone(),
            config.oauth.client_id.clone(),
            config.oauth.client_secret.clone(),
        );

        // Initialize the token manager
        if let Err(e) = manager.init().await {
            eprintln!("Failed to initialize ServiceAccessTokenManager: {}", e);
            eprintln!("This is required for service-to-service authentication");
            std::process::exit(1);
        }

        tracing::info!("ServiceAccessTokenManager initialized successfully");
        Arc::new(manager)
    };

    // Create the PublishRecordEvaluator with service token manager
    let publish_record_evaluator = Arc::new(PublishRecordEvaluator::new(
        Arc::new(http_client.clone()),
        service_token_manager.clone(),
    ));

    // Create the NodeEvaluatorFactory with all evaluators
    let factory = Arc::new(
        NodeEvaluatorFactory::new()
            .register("condition", Arc::new(ConditionEvaluator::new()))
            .register("transform", Arc::new(TransformEvaluator::new()))
            .register(
                "facet_text",
                Arc::new(FacetTextEvaluator::new(identity_resolver.clone())),
            )
            .register(
                "get_record",
                Arc::new(GetRecordEvaluator::new(
                    Arc::new(http_client.clone()),
                    identity_resolver.clone(),
                )),
            )
            .register("jetstream_entry", Arc::new(JetstreamEntryEvaluator::new()))
            .register("parse_aturi", Arc::new(ParseAturiEvaluator::new()))
            .register("periodic_entry", Arc::new(PeriodicEntryEvaluator::new()))
            .register(
                "sentiment_analysis",
                Arc::new(SentimentAnalysisEvaluator::new()),
            )
            .register("webhook_entry", Arc::new(WebhookEntryEvaluator::new()))
            .register("zap_entry", Arc::new(ZapEntryEvaluator::new()))
            .register("publish_record", publish_record_evaluator)
            .register("publish_webhook", webhook_evaluator)
            .register("debug_action", Arc::new(DebugActionEvaluator::new())),
    );

    // Create blueprint queue adapter for async blueprint evaluation based on configuration
    let (blueprint_queue_adapter, blueprint_sender) =
        create_blueprint_queue_adapter(&config.blueprint_queue, redis_pool.clone())?;

    // Create channel for blueprint cache reload requests
    let (cache_reload_sender, cache_reload_receiver) = mpsc::channel::<()>(10);

    // Create throttler for web context based on configuration
    let web_context_throttler: Arc<dyn BlueprintThrottler> = match config
        .blueprint_throttler
        .as_str()
    {
        "redis" => {
            if let Some(redis_pool) = redis_pool.clone() {
                match RedisBlueprintThrottler::new(
                    redis_pool,
                    config.blueprint_throttler_authority_limit,
                    config.blueprint_throttler_authority_window,
                    config.blueprint_throttler_aturi_limit,
                    config.blueprint_throttler_aturi_window,
                ) {
                    Ok(throttler) => {
                        tracing::info!("Using Redis blueprint throttler for web context");
                        Arc::new(throttler)
                    }
                    Err(e) => {
                        tracing::error!(error = ?e, "Failed to create Redis blueprint throttler for web context, falling back to no-op");
                        Arc::new(NoOpBlueprintThrottler::new())
                    }
                }
            } else {
                tracing::warn!(
                    "Redis blueprint throttler configured but Redis not available for web context, using no-op"
                );
                Arc::new(NoOpBlueprintThrottler::new())
            }
        }
        "redis-per-identity" => {
            if let Some(redis_pool) = redis_pool.clone() {
                // All four default values are guaranteed to be present by config validation
                match RedisPerIdentityBlueprintThrottler::new(
                    redis_pool,
                    config.blueprint_throttler_authority_default_limit.unwrap(),
                    config.blueprint_throttler_authority_default_window.unwrap(),
                    config.blueprint_throttler_aturi_default_limit.unwrap(),
                    config.blueprint_throttler_aturi_default_window.unwrap(),
                ) {
                    Ok(throttler) => {
                        tracing::info!(
                            "Using Redis per-identity blueprint throttler for web context - defaults: authority: {}/{} min, aturi: {}/{} min",
                            config.blueprint_throttler_authority_default_limit.unwrap(),
                            config.blueprint_throttler_authority_default_window.unwrap(),
                            config.blueprint_throttler_aturi_default_limit.unwrap(),
                            config.blueprint_throttler_aturi_default_window.unwrap(),
                        );
                        Arc::new(throttler)
                    }
                    Err(e) => {
                        tracing::error!(error = ?e, "Failed to create Redis per-identity blueprint throttler for web context, falling back to no-op");
                        Arc::new(NoOpBlueprintThrottler::new())
                    }
                }
            } else {
                tracing::warn!(
                    "Redis per-identity blueprint throttler configured but Redis not available for web context, using no-op"
                );
                Arc::new(NoOpBlueprintThrottler::new())
            }
        }
        _ => {
            tracing::info!("Using no-op blueprint throttler for web context");
            Arc::new(NoOpBlueprintThrottler::new())
        }
    };

    // Create evaluation result storage based on configuration
    let evaluation_storage: Arc<dyn EvaluationResultStorage> =
        match config.evaluation_storage.storage_type.as_str() {
            "filesystem" => {
                let base_dir = config
                    .evaluation_storage
                    .filesystem_base_directory
                    .as_ref()
                    .expect("Filesystem base directory should be validated in config");
                tracing::info!(
                    "Using filesystem evaluation result storage at: {}",
                    base_dir
                );
                Arc::new(FilesystemEvaluationResultStorage::new(base_dir))
            }
            "noop" => {
                tracing::info!("Using no-op evaluation result storage");
                Arc::new(NoopEvaluationResultStorage::new())
            }
            _ => {
                tracing::info!("Using tracing evaluation result storage");
                Arc::new(TracingEvaluationResultStorage::new())
            }
        };

    // Create web context with blueprint sender for webhook support and service token manager
    let web_context = WebContext::with_senders_throttler_and_storage(
        config.clone(),
        http_client.clone(),
        template_env,
        identity_resolver.clone() as Arc<dyn IdentityResolver>,
        pool.clone(),
        service_document,
        Some(blueprint_sender.clone()),
        Some(cache_reload_sender.clone()),
        redis_pool.clone(),
        web_context_throttler,
        evaluation_storage.clone(),
        service_token_manager.clone(),
    );

    // Start the async blueprint evaluation task
    {
        let eval_task = BlueprintEvaluationTask::with_adapter(
            factory.clone(),
            web_context.blueprint_storage().clone(),
            web_context.node_storage().clone(),
            blueprint_queue_adapter.clone(),
            blueprint_leadership.clone(),
            token.clone(),
            true, // enable tracing
            config.disabled_node_types.clone(),
            config.allowed_publish_collections.clone(),
            evaluation_storage,
        );

        spawn_cancellable_task(&tracker, token.clone(), |cancel_token| async move {
            tokio::select! {
                result = eval_task.run() => {
                    if let Err(e) = result {
                        tracing::error!(error = ?e, "Blueprint evaluation task failed");
                        Err(anyhow::anyhow!(e))
                    } else {
                        Ok(())
                    }
                }
                _ = cancel_token.cancelled() => {
                    tracing::info!("Blueprint evaluation task cancelled");
                    Ok(())
                }
            }
        });
    }

    // Start the scheduler task for periodic entry nodes if enabled
    if config.scheduler.enabled {
        tracing::info!(
            "Starting scheduler task with {}s check interval and {}s cache reload",
            config.scheduler.check_interval_secs,
            config.scheduler.cache_reload_secs
        );

        // Create scheduler state storage
        let scheduler_storage =
            Arc::new(PostgresSchedulerStateStorage::new(Arc::new(pool.clone())));

        // Create and start scheduler task with optional leadership election
        let scheduler_task = SchedulerTask::with_leadership(
            web_context.blueprint_storage().clone(),
            web_context.node_storage().clone(),
            scheduler_storage,
            blueprint_queue_adapter.clone(),
            token.clone(),
            config.allowed_publish_collections.clone(),
            scheduler_leadership.clone(),
        );

        spawn_cancellable_task(&tracker, token.clone(), |cancel_token| async move {
            tokio::select! {
                result = scheduler_task.run() => {
                    if let Err(e) = result {
                        tracing::error!(error = ?e, "Scheduler task failed");
                        Err(anyhow::anyhow!(e))
                    } else {
                        Ok(())
                    }
                }
                _ = cancel_token.cancelled() => {
                    tracing::info!("Scheduler task cancelled");
                    Ok(())
                }
            }
        });
    } else {
        tracing::info!("Scheduler disabled - periodic entry nodes will not be evaluated");
    }

    // Start queue health monitoring if enabled
    if config.blueprint_queue.enable_health_check {
        let health_check_adapter = blueprint_queue_adapter.clone();
        let health_check_interval = config.blueprint_queue.health_check_interval_secs;

        spawn_cancellable_task(&tracker, token.clone(), move |cancel_token| async move {
            tracing::info!(
                "Starting blueprint queue health monitoring with {}s interval",
                health_check_interval
            );

            let mut interval = tokio::time::interval(Duration::from_secs(health_check_interval));
            let mut consecutive_failures = 0u32;
            const MAX_CONSECUTIVE_FAILURES: u32 = 3;

            while !cancel_token.is_cancelled() {
                tokio::select! {
                    _ = interval.tick() => {
                        if health_check_adapter.is_healthy().await {
                            if consecutive_failures > 0 {
                                tracing::info!("Blueprint queue adapter is healthy again");
                                consecutive_failures = 0;
                            }
                        } else {
                            consecutive_failures += 1;
                            tracing::warn!(
                                consecutive_failures = consecutive_failures,
                                max_failures = MAX_CONSECUTIVE_FAILURES,
                                "Blueprint queue adapter health check failed"
                            );

                            if consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
                                tracing::error!(
                                    "Blueprint queue adapter has failed {} consecutive health checks",
                                    consecutive_failures
                                );
                                // Could trigger alerts or recovery actions here
                            }
                        }
                    }
                    () = cancel_token.cancelled() => {
                        tracing::info!("Queue health monitoring task cancelled");
                        break;
                    }
                }
            }
            Ok(())
        });
    }

    let router = build_router(web_context.clone());
    let port = *config.http_port.as_ref();

    // Setup signal handler
    {
        let signal_tracker = tracker.clone();
        let signal_token = token.clone();

        // Spawn signal handler without using the managed task helper since it's special
        tracing::info!("Starting signal handler task");
        tokio::spawn(async move {
            let ctrl_c = async {
                signal::ctrl_c()
                    .await
                    .expect("failed to install Ctrl+C handler");
            };

            #[cfg(unix)]
            let terminate = async {
                signal::unix::signal(signal::unix::SignalKind::terminate())
                    .expect("failed to install signal handler")
                    .recv()
                    .await;
            };

            #[cfg(not(unix))]
            let terminate = std::future::pending::<()>();

            tokio::select! {
                () = signal_token.cancelled() => {
                    tracing::info!("Signal handler task shutting down gracefully");
                },
                _ = terminate => {
                    tracing::info!("Received SIGTERM signal, initiating shutdown");
                },
                _ = ctrl_c => {
                    tracing::info!("Received Ctrl+C signal, initiating shutdown");
                },
            }

            signal_tracker.close();
            signal_token.cancel();
            tracing::info!("Signal handler task completed");
        });
    }

    // Start Jetstream consumer if enabled
    if config.jetstream.enabled {
        tracing::info!(
            "Starting Jetstream consumer with configuration: {:?}",
            config.jetstream
        );

        // Create consumer and processor with partition configuration
        let consumer = Consumer {};

        // Create partition configuration for both consumer and processor
        let partition_config = ifthisthenat::consumer::PartitionConfig::new(
            config.jetstream.instance_id,
            config.jetstream.total_instances,
            config.jetstream.partition_strategy.clone(),
        );

        // Create multiple consumers based on config
        let (blueprint_handler, event_receivers): (
            Arc<dyn EventHandler>,
            Vec<mpsc::Receiver<BlueprintEvent>>,
        ) = if config.jetstream.worker_threads > 1 {
            tracing::info!(
                worker_threads = config.jetstream.worker_threads,
                instance_id = config.jetstream.instance_id,
                total_instances = config.jetstream.total_instances,
                strategy = ?config.jetstream.partition_strategy,
                "Creating multi-consumer handler with {} worker threads",
                config.jetstream.worker_threads
            );
            let (handler, receivers) = consumer
                .create_multi_consumer_handler_with_partition_config(
                    config.jetstream.collections.clone(),
                    config.jetstream.worker_threads,
                    partition_config.clone(),
                    denylist_manager.clone(),
                )
                .map_err(|e| {
                    tracing::error!(
                        error = ?e,
                        instance_id = config.jetstream.instance_id,
                        total_instances = config.jetstream.total_instances,
                        worker_threads = config.jetstream.worker_threads,
                        "Failed to create multi-consumer handler with partition configuration"
                    );
                    e
                })?;
            (handler as Arc<dyn EventHandler>, receivers)
        } else {
            // Single consumer mode
            let (handler, receiver) = consumer
                .create_blueprint_handler_with_partition_config(
                    config.jetstream.collections.clone(),
                    partition_config.clone(),
                    denylist_manager.clone(),
                )
                .map_err(|e| {
                    tracing::error!(
                        error = ?e,
                        instance_id = config.jetstream.instance_id,
                        total_instances = config.jetstream.total_instances,
                        "Failed to create blueprint handler with partition configuration"
                    );
                    e
                })?;
            // Wrap single receiver in a Vec to match multi-consumer interface
            (handler as Arc<dyn EventHandler>, vec![receiver])
        };

        // Create throttler based on configuration
        let throttler: Arc<dyn BlueprintThrottler> = match config.blueprint_throttler.as_str() {
            "redis" => {
                if let Some(redis_pool) = redis_pool.clone() {
                    match RedisBlueprintThrottler::new(
                        redis_pool,
                        config.blueprint_throttler_authority_limit,
                        config.blueprint_throttler_authority_window,
                        config.blueprint_throttler_aturi_limit,
                        config.blueprint_throttler_aturi_window,
                    ) {
                        Ok(throttler) => {
                            tracing::info!(
                                "Redis blueprint throttler configured - authority: {:?}/{:?}min, aturi: {:?}/{:?}min",
                                config.blueprint_throttler_authority_limit,
                                config.blueprint_throttler_authority_window,
                                config.blueprint_throttler_aturi_limit,
                                config.blueprint_throttler_aturi_window,
                            );
                            Arc::new(throttler)
                        }
                        Err(e) => {
                            tracing::error!(error = ?e, "Failed to create Redis blueprint throttler, falling back to no-op");
                            Arc::new(NoOpBlueprintThrottler::new())
                        }
                    }
                } else {
                    tracing::warn!(
                        "Redis blueprint throttler configured but Redis not available, using no-op"
                    );
                    Arc::new(NoOpBlueprintThrottler::new())
                }
            }
            "redis-per-identity" => {
                if let Some(redis_pool) = redis_pool.clone() {
                    // All four default values are guaranteed to be present by config validation
                    match RedisPerIdentityBlueprintThrottler::new(
                        redis_pool,
                        config.blueprint_throttler_authority_default_limit.unwrap(),
                        config.blueprint_throttler_authority_default_window.unwrap(),
                        config.blueprint_throttler_aturi_default_limit.unwrap(),
                        config.blueprint_throttler_aturi_default_window.unwrap(),
                    ) {
                        Ok(throttler) => {
                            tracing::info!(
                                "Redis per-identity blueprint throttler configured - defaults: authority: {}/{} min, aturi: {}/{} min",
                                config.blueprint_throttler_authority_default_limit.unwrap(),
                                config.blueprint_throttler_authority_default_window.unwrap(),
                                config.blueprint_throttler_aturi_default_limit.unwrap(),
                                config.blueprint_throttler_aturi_default_window.unwrap(),
                            );
                            Arc::new(throttler)
                        }
                        Err(e) => {
                            tracing::error!(error = ?e, "Failed to create Redis per-identity blueprint throttler, falling back to no-op");
                            Arc::new(NoOpBlueprintThrottler::new())
                        }
                    }
                } else {
                    tracing::warn!(
                        "Redis per-identity blueprint throttler configured but Redis not available, using no-op"
                    );
                    Arc::new(NoOpBlueprintThrottler::new())
                }
            }
            _ => {
                tracing::info!("Using no-op blueprint throttler");
                Arc::new(NoOpBlueprintThrottler::new())
            }
        };

        // Processor now uses the same blueprint_sender for async evaluation
        let processor = BlueprintProcessor::with_cache_reload_interval(
            web_context.blueprint_storage().clone(),
            web_context.node_storage().clone(),
            blueprint_sender.clone(),
            config.blueprint_cache_reload_seconds.to_duration(),
            Some(partition_config.clone()),
            throttler,
            config.blueprint_cache_max_size,
        );
        let processor = Arc::new(processor);

        // Load blueprints cache before starting processor
        if let Err(e) = processor.load_blueprints_cache().await {
            tracing::error!(error = ?e, "Failed to load blueprints cache");
        }

        // Spawn cache reload handler task
        {
            let processor_clone = processor.clone();
            let mut reload_receiver = cache_reload_receiver;
            spawn_cancellable_task(&tracker, token.clone(), |cancel_token| async move {
                loop {
                    tokio::select! {
                        Some(_) = reload_receiver.recv() => {
                            tracing::debug!("Cache reload requested");
                            match processor_clone.trigger_cache_reload().await {
                                Ok(reloaded) => {
                                    if reloaded {
                                        tracing::info!("Blueprint cache reloaded successfully");
                                    } else {
                                        tracing::debug!("Blueprint cache reload rate limited");
                                    }
                                }
                                Err(e) => {
                                    tracing::error!(error = ?e, "Failed to reload blueprint cache");
                                }
                            }
                        }
                        () = cancel_token.cancelled() => {
                            tracing::info!("Cache reload handler cancelled");
                            break;
                        }
                    }
                }
                Ok(())
            });
        }

        // Spawn processor task(s) with cancellation support
        // Each consumer gets its own processor task
        for (consumer_id, event_receiver) in event_receivers.into_iter().enumerate() {
            let processor_clone = processor.clone();

            spawn_cancellable_task(&tracker, token.clone(), move |cancel_token| async move {
                tracing::info!(consumer_id = consumer_id, "Starting processor for consumer");
                tokio::select! {
                    result = processor_clone.start_processing(event_receiver) => {
                        result.map_err(|e| anyhow::anyhow!("Processor error: {}", e))
                    }
                    () = cancel_token.cancelled() => {
                        Ok(())
                    }
                }
            });
        }

        // Setup cursor handler based on configuration (Redis preferred over file)
        let mut cursor_handler: Option<Arc<dyn EventHandler>> = None;

        if let Some(pool) = redis_pool.as_ref() {
            match consumer
                .create_redis_cursor_handler(
                    pool.clone(),
                    config.redis_cursor_key.clone(),
                    config.redis_cursor_ttl_seconds,
                )
                .await
            {
                Ok(handler) => {
                    tracing::info!(
                        "Using Redis cursor handler with key: {}",
                        config.redis_cursor_key
                    );
                    cursor_handler = Some(handler as Arc<dyn EventHandler>);
                }
                Err(e) => {
                    tracing::warn!(
                        error = ?e,
                        "Failed to create Redis cursor handler, falling back to filesystem cursor handler if available"
                    );
                }
            }
        }

        if cursor_handler.is_none() {
            if let Some(cursor_path) = &config.jetstream.cursor_path {
                tracing::info!(
                    path = %cursor_path,
                    "Using filesystem cursor handler at {}",
                    cursor_path
                );
                let handler: Arc<dyn EventHandler> = Arc::new(CursorWriterHandler::new(
                    "file-cursor-writer".to_string(),
                    cursor_path.clone(),
                ));
                cursor_handler = Some(handler);
            } else {
                tracing::info!(
                    "No cursor persistence configured; Jetstream will resume from live stream on restart"
                );
            }
        }

        // Read initial cursor from Redis or file
        let initial_cursor = if let Some(pool) = redis_pool.as_ref() {
            let cursor = Consumer::read_cursor_from_redis(pool, &config.redis_cursor_key).await;
            if cursor.is_some() {
                tracing::info!(cursor = ?cursor, "Loaded cursor from Redis");
            }
            cursor
        } else if let Some(cursor_path) = &config.jetstream.cursor_path {
            let cursor = Consumer::read_cursor(cursor_path).await;
            if cursor.is_some() {
                tracing::info!(cursor = ?cursor, "Loaded cursor from file");
            }
            cursor
        } else {
            None
        };

        // Spawn Jetstream consumer task with reconnection logic
        spawn_cancellable_task(&tracker, token.clone(), move |cancel_token| {
            let config = config.clone();
            let blueprint_handler = blueprint_handler.clone();
            let cursor_handler = cursor_handler.clone();
            let redis_pool = redis_pool.clone();
            let cursor_path = config.jetstream.cursor_path.clone();
            let redis_cursor_key = config.redis_cursor_key.clone();

            async move {
                let mut last_known_cursor = initial_cursor;
                let mut last_cursor_source: Option<&'static str> = None;
                let mut reconnect_count = 0u32;
                let max_reconnects_per_minute = 30;
                let reconnect_window = Duration::from_secs(120);
                let mut last_disconnect = std::time::Instant::now() - reconnect_window;

                while !cancel_token.is_cancelled() {
                    // Refresh latest cursor from persistent storage before establishing a connection
                    if let Some(pool) = redis_pool.as_ref() {
                        if let Some(cursor) =
                            Consumer::read_cursor_from_redis(pool, &redis_cursor_key).await
                        {
                            if Some(cursor) != last_known_cursor {
                                tracing::info!(
                                    cursor = cursor,
                                    "Resuming Jetstream stream from Redis cursor"
                                );
                                last_known_cursor = Some(cursor);
                                last_cursor_source = Some("redis");
                            }
                        }
                    } else if let Some(path) = cursor_path.as_ref() {
                        if let Some(cursor) = Consumer::read_cursor(path).await {
                            if Some(cursor) != last_known_cursor {
                                tracing::info!(
                                    cursor = cursor,
                                    path = %path,
                                    "Resuming Jetstream stream from filesystem cursor"
                                );
                                last_known_cursor = Some(cursor);
                                last_cursor_source = Some("filesystem");
                            }
                        }
                    }

                    if last_known_cursor.is_none() && last_cursor_source != Some("live") {
                        tracing::debug!(
                            "No persisted cursor available; Jetstream will start from the live stream"
                        );
                        last_cursor_source = Some("live");
                    }

                    let now = std::time::Instant::now();
                    if now.duration_since(last_disconnect) < reconnect_window {
                        reconnect_count += 1;
                        if reconnect_count > max_reconnects_per_minute {
                            tracing::warn!(
                                count = reconnect_count,
                                "Too many reconnects, waiting 60 seconds"
                            );
                            tokio::time::sleep(reconnect_window).await;
                            reconnect_count = 0;
                            last_disconnect = now;
                            continue;
                        }
                    } else {
                        reconnect_count = 0;
                    }

                    let consumer_config = ConsumerTaskConfig {
                        user_agent: config.user_agent.clone(),
                        compression: false,
                        zstd_dictionary_location: String::new(),
                        jetstream_hostname: config.jetstream.hostname.clone(),
                        collections: config.jetstream.collections.clone(),
                        dids: vec![],
                        max_message_size_bytes: Some(15 * 1024 * 1024), // 10MB
                        cursor: last_known_cursor,
                        require_hello: true,
                    };

                    let consumer = JetstreamConsumer::new(consumer_config);

                    // Register handlers
                    if let Err(e) = consumer.register_handler(blueprint_handler.clone()).await {
                        tracing::error!(error = ?e, "Failed to register blueprint handler");
                        continue;
                    }

                    if let Some(handler) = cursor_handler.clone() {
                        if let Err(e) = consumer.register_handler(handler.clone()).await {
                            tracing::error!(error = ?e, "Failed to register cursor handler");
                        }
                    }

                    match consumer.run_background(cancel_token.clone()).await {
                        Ok(()) => {
                            tracing::info!("Jetstream consumer stopped");
                            if cancel_token.is_cancelled() {
                                break;
                            }
                            last_disconnect = std::time::Instant::now();
                            tokio::time::sleep(Duration::from_secs(5)).await;
                        }
                        Err(e) => {
                            tracing::error!(error = ?e, "Jetstream consumer connection failed, will reconnect");
                            last_disconnect = std::time::Instant::now();

                            if !cancel_token.is_cancelled() {
                                tokio::time::sleep(Duration::from_secs(5)).await;
                            }
                        }
                    }
                }
                tracing::info!("jetstream task ending");
                Ok(())
            }
        });
    }

    // Start HTTP server
    spawn_cancellable_task(&tracker, token.clone(), move |cancel_token| {
        let external_base = external_base.clone();
        let version = version.clone();

        async move {
            let listener = TcpListener::bind(format!("0.0.0.0:{}", port))
                .await
                .map_err(|e| anyhow::anyhow!("Failed to bind to port {}: {}", port, e))?;

            tracing::info!(port = port, external_base = %external_base, version = %version, "HTTP server listening");

            let shutdown_token = cancel_token.clone();
            axum::serve(listener, router)
                .with_graceful_shutdown(async move {
                    shutdown_token.cancelled().await;
                })
                .await
                .map_err(|e| anyhow::anyhow!("HTTP server error: {}", e))?;

            Ok(())
        }
    });

    // Wait for all tasks to complete
    tracing::info!("Waiting for all tasks to complete...");
    tracker.wait().await;

    tracing::info!("All tasks completed, application shutting down");

    // Flush Sentry events before shutting down
    // The guard will automatically flush when dropped, but we can ensure it happens
    // if _guard.is_some() {
    //     tracing::info!("Flushing Sentry events...");
    //     drop(_guard.take());
    //     // Give Sentry time to flush
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    // }

    Ok(())
}
