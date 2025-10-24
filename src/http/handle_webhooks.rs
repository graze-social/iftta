use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde_json::{Value, json};
use std::sync::Arc;
use tracing::{debug, error, info, warn};

use crate::{
    http::WebContext,
    storage::{blueprint::BlueprintStorage, node::NodeStorage},
    tasks::submit_blueprint,
    validation::Validator,
};

/// Handler for webhook requests to trigger blueprint evaluation
///
/// POST /webhooks/{blueprint_record_key}
///
/// This endpoint:
/// 1. Constructs the blueprint AT-URI from the record key
/// 2. Fetches the blueprint and validates it exists
/// 3. Validates the first node is a webhook_entry node
/// 4. Submits the webhook data for async blueprint evaluation
pub async fn handle_webhook(
    State(context): State<WebContext>,
    Path(blueprint_record_key): Path<String>,
    Json(payload): Json<Value>,
) -> impl IntoResponse {
    // Try to fetch blueprint by searching for one ending with the record key
    // Since we don't know the DID, we need to list all blueprints
    let all_blueprints = match context.blueprint_storage.list_blueprints(None).await {
        Ok(blueprints) => blueprints,
        Err(e) => {
            error!(error = ?e, "Failed to list blueprints");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "error": "Failed to fetch blueprints",
                    "message": e.to_string()
                })),
            )
                .into_response();
        }
    };

    // Find blueprint matching the record key
    let blueprint = all_blueprints
        .into_iter()
        .find(|bp| bp.aturi.ends_with(&format!("/{}", blueprint_record_key)));

    let blueprint = match blueprint {
        Some(bp) => bp,
        None => {
            warn!(record_key = %blueprint_record_key, "Blueprint not found");
            return (
                StatusCode::NOT_FOUND,
                Json(json!({
                    "error": "Blueprint not found",
                    "message": format!("No blueprint found with record key: {}", blueprint_record_key)
                }))
            ).into_response();
        }
    };

    // Fetch nodes for the blueprint
    let nodes = match context.node_storage.list_nodes(&blueprint.aturi).await {
        Ok(nodes) => nodes,
        Err(e) => {
            error!(error = ?e, blueprint = %blueprint.aturi, "Failed to fetch nodes");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "error": "Failed to fetch blueprint nodes",
                    "message": e.to_string()
                })),
            )
                .into_response();
        }
    };

    // Validate blueprint has nodes and first node is webhook_entry
    if nodes.is_empty() {
        warn!(blueprint = %blueprint.aturi, "Blueprint has no nodes");
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": "Invalid blueprint",
                "message": "Blueprint has no nodes"
            })),
        )
            .into_response();
    }

    // Validate the blueprint structure
    if let Err(e) = Validator::validate_blueprint_nodes(&nodes) {
        warn!(blueprint = %blueprint.aturi, error = ?e, "Invalid blueprint structure");
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": "Invalid blueprint",
                "message": e.to_string()
            })),
        )
            .into_response();
    }

    // Validate publish_record collections if constraints are configured
    if let Err(e) =
        Validator::validate_publish_collections(&nodes, &context.config.allowed_publish_collections)
    {
        warn!(
            blueprint = %blueprint.aturi,
            error = ?e,
            "Blueprint contains disallowed collections"
        );
        return (
            StatusCode::FORBIDDEN,
            Json(json!({
                "error": "Blueprint validation failed",
                "message": e.to_string()
            })),
        )
            .into_response();
    }

    // Get the first node (which is guaranteed to be an entry node after validation)
    let entry_node = &nodes[0];

    // Verify it's specifically a webhook_entry node
    if entry_node.node_type != "webhook_entry" {
        warn!(
            blueprint = %blueprint.aturi,
            node_type = %entry_node.node_type,
            "Blueprint does not have webhook_entry as first node"
        );
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": "Invalid blueprint type",
                "message": format!("Blueprint must have webhook_entry as first node, found: {}", entry_node.node_type)
            }))
        ).into_response();
    }

    // Check if this blueprint should be throttled
    match context.throttler().throttle(&blueprint.aturi).await {
        Ok(true) => {
            // Blueprint is throttled, return 200 OK immediately
            info!(
                blueprint = %blueprint.aturi,
                record_key = %blueprint_record_key,
                "Webhook request throttled"
            );
            return (
                StatusCode::OK,
                Json(json!({
                    "status": "throttled",
                    "message": "Request accepted but throttled",
                    "blueprint": blueprint.aturi
                })),
            )
                .into_response();
        }
        Ok(false) => {
            // Not throttled, proceed with evaluation
        }
        Err(e) => {
            // Error checking throttle, log but proceed anyway
            warn!(
                error = ?e,
                blueprint = %blueprint.aturi,
                "Error checking throttle, proceeding with evaluation"
            );
        }
    }

    // Get the blueprint sender from context (will be added to WebContext)
    let blueprint_sender = match context.blueprint_sender() {
        Some(sender) => sender,
        None => {
            error!("Blueprint evaluation not configured");
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({
                    "error": "Service unavailable",
                    "message": "Blueprint evaluation is not configured"
                })),
            )
                .into_response();
        }
    };

    // Submit the blueprint for async evaluation
    info!(
        blueprint = %blueprint.aturi,
        record_key = %blueprint_record_key,
        "Processing webhook request"
    );

    match submit_blueprint(
        blueprint_sender,
        blueprint.aturi.clone(),
        Arc::new(payload),
        Some(uuid::Uuid::new_v4().to_string()),
    )
    .await
    {
        Ok(_) => {
            debug!(blueprint = %blueprint.aturi, "Blueprint submitted for evaluation");
            (
                StatusCode::ACCEPTED,
                Json(json!({
                    "status": "accepted",
                    "message": "Blueprint evaluation queued",
                    "blueprint": blueprint.aturi
                })),
            )
                .into_response()
        }
        Err(e) => {
            error!(error = ?e, blueprint = %blueprint.aturi, "Failed to submit blueprint");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "error": "Failed to queue evaluation",
                    "message": e.to_string()
                })),
            )
                .into_response()
        }
    }
}

/// Alternative handler that accepts blueprint AT-URI in the request body
/// This can be useful for testing or when the caller knows the exact AT-URI
#[allow(dead_code)]
pub async fn handle_webhook_with_aturi(
    State(context): State<WebContext>,
    Json(request): Json<WebhookRequest>,
) -> impl IntoResponse {
    // Fetch the blueprint
    let blueprint = match context
        .blueprint_storage
        .get_blueprint(&request.aturi)
        .await
    {
        Ok(Some(bp)) => bp,
        Ok(None) => {
            warn!(aturi = %request.aturi, "Blueprint not found");
            return (
                StatusCode::NOT_FOUND,
                Json(json!({
                    "error": "Blueprint not found",
                    "message": format!("Blueprint {} not found", request.aturi)
                })),
            )
                .into_response();
        }
        Err(e) => {
            error!(error = ?e, "Failed to fetch blueprint");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "error": "Failed to fetch blueprint",
                    "message": e.to_string()
                })),
            )
                .into_response();
        }
    };

    // Fetch nodes for validation
    let nodes = match context.node_storage.list_nodes(&blueprint.aturi).await {
        Ok(nodes) => nodes,
        Err(e) => {
            error!(error = ?e, blueprint = %blueprint.aturi, "Failed to fetch nodes");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "error": "Failed to fetch blueprint nodes",
                    "message": e.to_string()
                })),
            )
                .into_response();
        }
    };

    // Validate the blueprint structure
    if let Err(e) = Validator::validate_blueprint_nodes(&nodes) {
        warn!(blueprint = %blueprint.aturi, error = ?e, "Invalid blueprint structure");
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": "Invalid blueprint",
                "message": e.to_string()
            })),
        )
            .into_response();
    }

    // Validate publish_record collections if constraints are configured
    if let Err(e) =
        Validator::validate_publish_collections(&nodes, &context.config.allowed_publish_collections)
    {
        warn!(
            blueprint = %blueprint.aturi,
            error = ?e,
            "Blueprint contains disallowed collections"
        );
        return (
            StatusCode::FORBIDDEN,
            Json(json!({
                "error": "Blueprint validation failed",
                "message": e.to_string()
            })),
        )
            .into_response();
    }

    // Get the first node (which is guaranteed to be an entry node after validation)
    let entry_node = &nodes[0];

    if entry_node.node_type != "webhook_entry" {
        warn!(
            blueprint = %blueprint.aturi,
            node_type = %entry_node.node_type,
            "Blueprint does not have webhook_entry as first node"
        );
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": "Invalid blueprint type",
                "message": format!("Blueprint must have webhook_entry as first node, found: {}", entry_node.node_type)
            }))
        ).into_response();
    }

    // Check if this blueprint should be throttled
    match context.throttler().throttle(&blueprint.aturi).await {
        Ok(true) => {
            // Blueprint is throttled, return 200 OK immediately
            info!(
                blueprint = %blueprint.aturi,
                "Webhook request throttled"
            );
            return (
                StatusCode::OK,
                Json(json!({
                    "status": "throttled",
                    "message": "Request accepted but throttled",
                    "blueprint": blueprint.aturi
                })),
            )
                .into_response();
        }
        Ok(false) => {
            // Not throttled, proceed with evaluation
        }
        Err(e) => {
            // Error checking throttle, log but proceed anyway
            warn!(
                error = ?e,
                blueprint = %blueprint.aturi,
                "Error checking throttle, proceeding with evaluation"
            );
        }
    }

    // Get the blueprint sender
    let blueprint_sender = match context.blueprint_sender() {
        Some(sender) => sender,
        None => {
            error!("Blueprint evaluation not configured");
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({
                    "error": "Service unavailable",
                    "message": "Blueprint evaluation is not configured"
                })),
            )
                .into_response();
        }
    };

    // Submit for evaluation
    match submit_blueprint(
        blueprint_sender,
        blueprint.aturi.clone(),
        Arc::new(request.payload),
        Some(uuid::Uuid::new_v4().to_string()),
    )
    .await
    {
        Ok(_) => {
            debug!(blueprint = %blueprint.aturi, "Blueprint submitted for evaluation");
            (
                StatusCode::ACCEPTED,
                Json(json!({
                    "status": "accepted",
                    "message": "Blueprint evaluation queued",
                    "blueprint": blueprint.aturi
                })),
            )
                .into_response()
        }
        Err(e) => {
            error!(error = ?e, blueprint = %blueprint.aturi, "Failed to submit blueprint");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "error": "Failed to queue evaluation",
                    "message": e.to_string()
                })),
            )
                .into_response()
        }
    }
}

#[derive(serde::Deserialize)]
#[allow(dead_code)]
pub struct WebhookRequest {
    pub aturi: String,
    pub payload: Value,
}
