use std::time::Duration;

use axum::{
    Router, middleware,
    routing::{get, post},
};
use http::{
    Method,
    header::{ACCEPT, ACCEPT_LANGUAGE},
};
use tower_http::trace::TraceLayer;
use tower_http::{classify::ServerErrorsFailureClass, timeout::TimeoutLayer};
use tower_http::{cors::CorsLayer, services::ServeDir};
use tracing::Span;

use crate::http::{
    context::WebContext,
    handle_auth::{handle_login_callback, handle_login_get, handle_login_post},
    handle_blueprints::{
        handle_add_node, handle_blueprint_edit,
        handle_delete_blueprint as handle_delete_blueprint_web,
        handle_delete_node as handle_delete_blueprint_node, handle_update_node,
    },
    handle_dashboard::{
        handle_blueprint_evaluation_results, handle_create_blueprint, handle_create_blueprint_get,
        handle_dashboard_get, handle_evaluate_blueprint_get, handle_evaluate_node_get,
        handle_logout, handle_validate_blueprint,
    },
    handle_documentation::handle_documentation,
    handle_index::handle_index,
    handle_prototypes::{
        handle_create_prototype, handle_instantiate_prototype, handle_prototype_builder_get,
        handle_prototypes_get, handle_toggle_prototype_shared, handle_validate_prototype,
    },
    handle_public_prototypes::{
        handle_instantiate_shared_prototype, handle_public_prototypes, handle_view_prototype,
    },
    handle_webhooks::handle_webhook,
    handle_wellknown::{handle_wellknown_atproto_did, handle_wellknown_did},
    handle_xrpc::{
        handle_create_blueprint as handle_create_blueprint_xrpc, handle_delete_blueprint,
        handle_delete_node, handle_evaluate_blueprint, handle_evaluate_node, handle_get_blueprint,
        handle_get_blueprints, handle_set_node, handle_update_blueprint,
    },
    handler_app_password::{get_app_password_status, set_app_password},
    middleware_auth::require_auth,
};

pub fn build_router(web_context: WebContext) -> Router {
    let serve_dir = ServeDir::new(web_context.config.http_static_path.clone());

    // Create protected dashboard routes that require authentication
    let protected_routes = Router::new()
        .route("/dashboard", get(handle_dashboard_get))
        .route("/dashboard/new", post(handle_create_blueprint)) // Old route for backward compatibility
        .route(
            "/dashboard/blueprints/new",
            get(handle_create_blueprint_get),
        ) // Show creation form
        .route(
            "/dashboard/blueprints/create",
            post(handle_create_blueprint),
        ) // Handle form submission
        .route(
            "/dashboard/blueprints/validate",
            post(handle_validate_blueprint),
        ) // Validate blueprint before creation
        .route("/logout", get(handle_logout))
        .route("/dashboard/{record_key}/edit", get(handle_blueprint_edit))
        .route(
            "/dashboard/{record_key}/delete",
            post(handle_delete_blueprint_web),
        )
        .route(
            "/dashboard/{record_key}/results",
            get(handle_blueprint_evaluation_results),
        )
        .route("/dashboard/{record_key}/nodes", post(handle_add_node))
        .route(
            "/dashboard/{blueprint_key}/nodes/{node_key}/update",
            post(handle_update_node),
        )
        .route(
            "/dashboard/{blueprint_key}/nodes/{node_key}/delete",
            post(handle_delete_blueprint_node),
        )
        // Prototype routes
        .route("/dashboard/prototypes", get(handle_prototypes_get))
        .route(
            "/dashboard/prototypes/builder",
            get(handle_prototype_builder_get),
        )
        .route(
            "/dashboard/prototypes/create",
            post(handle_create_prototype),
        )
        .route(
            "/dashboard/prototypes/validate",
            post(handle_validate_prototype),
        )
        .route(
            "/dashboard/prototypes/instantiate",
            post(handle_instantiate_prototype),
        )
        .route(
            "/dashboard/prototypes/toggle-shared",
            post(handle_toggle_prototype_shared),
        )
        // App-password routes
        .route("/dashboard/app-password", post(set_app_password))
        .route(
            "/dashboard/app-password/status",
            get(get_app_password_status),
        )
        // Evaluation tools
        .route("/dashboard/evaluate-node", get(handle_evaluate_node_get))
        .route(
            "/dashboard/evaluate-blueprint",
            get(handle_evaluate_blueprint_get),
        )
        .layer(middleware::from_fn_with_state(
            web_context.clone(),
            require_auth,
        ));

    let router = Router::new()
        .route("/.well-known/did.json", get(handle_wellknown_did))
        .route(
            "/.well-known/atproto-did",
            get(handle_wellknown_atproto_did),
        )
        .route("/", get(handle_index))
        .route("/documentation", get(handle_documentation))
        // Public prototype directory routes
        .route("/prototypes", get(handle_public_prototypes))
        .route("/prototypes/view", get(handle_view_prototype))
        .route(
            "/prototypes/instantiate",
            post(handle_instantiate_shared_prototype),
        )
        // Authentication routes
        .route("/login", get(handle_login_get))
        .route("/login", post(handle_login_post))
        .route("/login/callback", get(handle_login_callback))
        // Protected routes
        .merge(protected_routes)
        // XRPC routes
        .route(
            "/xrpc/tools.graze.ifthisthenat.getBlueprints",
            get(handle_get_blueprints),
        )
        .route(
            "/xrpc/tools.graze.ifthisthenat.getBlueprint",
            get(handle_get_blueprint),
        )
        .route(
            "/xrpc/tools.graze.ifthisthenat.createBlueprint",
            post(handle_create_blueprint_xrpc),
        )
        .route(
            "/xrpc/tools.graze.ifthisthenat.updateBlueprint",
            post(handle_update_blueprint),
        )
        .route(
            "/xrpc/tools.graze.ifthisthenat.deleteBlueprint",
            post(handle_delete_blueprint),
        )
        .route(
            "/xrpc/tools.graze.ifthisthenat.setNode",
            post(handle_set_node),
        )
        .route(
            "/xrpc/tools.graze.ifthisthenat.deleteNode",
            post(handle_delete_node),
        )
        .route(
            "/xrpc/tools.graze.ifthisthenat.evaluateBlueprint",
            post(handle_evaluate_blueprint),
        )
        .route(
            "/xrpc/tools.graze.ifthisthenat.evaluateNode",
            post(handle_evaluate_node),
        );

    // Conditionally register webhook endpoint only if webhook_entry node type is not disabled
    let router = if !web_context
        .config
        .disabled_node_types
        .contains("webhook_entry")
    {
        tracing::info!("Registering webhook endpoint - webhook_entry node type is enabled");
        router.route("/webhooks/{blueprint_record_key}", post(handle_webhook))
    } else {
        tracing::warn!(
            "Webhook endpoint disabled - webhook_entry node type is disabled in configuration"
        );
        router
    };

    let origins = [web_context.config.external_base.parse().unwrap()];

    // Enhanced HTTP tracing layer
    let trace_layer = TraceLayer::new_for_http()
        .make_span_with(|request: &http::Request<_>| {
            // Extract trace context from headers if present
            let trace_id = request
                .headers()
                .get("x-trace-id")
                .and_then(|h| h.to_str().ok())
                .map(String::from)
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

            tracing::info_span!(
                "http_request",
                method = %request.method(),
                uri = %request.uri(),
                trace_id = %trace_id,
                request_id = %uuid::Uuid::new_v4(),
            )
        })
        .on_request(|request: &http::Request<_>, _span: &Span| {
            tracing::info!(
                "started processing request {} {}",
                request.method(),
                request.uri().path()
            );
        })
        .on_response(
            |response: &http::Response<_>, latency: Duration, _span: &Span| {
                tracing::info!(
                    status = response.status().as_u16(),
                    latency_ms = latency.as_millis(),
                    "finished processing request"
                );
            },
        )
        .on_failure(
            |err: ServerErrorsFailureClass, latency: Duration, _span: &Span| {
                tracing::error!(
                    error = ?err,
                    latency_ms = latency.as_millis(),
                    "request failed"
                );
            },
        );

    router
        .nest_service("/static", serve_dir.clone())
        .fallback_service(serve_dir)
        .layer((trace_layer, TimeoutLayer::new(Duration::from_secs(30))))
        .layer(
            CorsLayer::new()
                .allow_origin(origins)
                .allow_methods([Method::GET, Method::POST])
                .allow_headers([ACCEPT_LANGUAGE, ACCEPT]),
        )
        .with_state(web_context)
}
