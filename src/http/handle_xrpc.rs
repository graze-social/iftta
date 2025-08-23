use axum::{
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Json},
};
use axum_extra::extract::PrivateCookieJar;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::json;
use ulid::Ulid;

use crate::{
    atproto::utilities::filter_map_dids,
    engine::{
        node_evaluator_factory::NodeEvaluatorFactory, node_type_condition::ConditionEvaluator,
        node_type_debug_action::DebugActionEvaluator, node_type_facet_text::FacetTextEvaluator,
        node_type_get_record::GetRecordEvaluator,
        node_type_jetstream_entry::JetstreamEntryEvaluator,
        node_type_parse_aturi::ParseAturiEvaluator,
        node_type_periodic_entry::PeriodicEntryEvaluator,
        node_type_publish_record::PublishRecordEvaluator,
        node_type_publish_webhook_direct::PublishWebhookDirectEvaluator,
        node_type_sentiment_analysis::SentimentAnalysisEvaluator,
        node_type_transform::TransformEvaluator, node_type_webhook_entry::WebhookEntryEvaluator,
        node_type_zap_entry::ZapEntryEvaluator,
    },
    http::{
        WebContext,
        auth::{get_authenticated_did, is_allowed_identity},
    },
    storage::{
        blueprint::{Blueprint, BlueprintStorage},
        node::{Node, NodeStorage},
    },
    tasks::submit_blueprint,
    validation::Validator,
};

#[derive(Debug, Serialize)]
pub(super) struct GetBlueprintsResponse {
    pub blueprints: Vec<Blueprint>,
}

pub(super) async fn handle_get_blueprints(
    State(context): State<WebContext>,
    jar: PrivateCookieJar,
    headers: HeaderMap,
) -> impl IntoResponse {
    // If no DID is provided, use the authenticated user's DID
    let did = match get_authenticated_did(&context, &jar, &headers).await {
        Ok(auth) => auth.did().to_string(),
        Err(error_response) => return error_response.into_response(),
    };

    match context.blueprint_storage.list_blueprints(Some(&did)).await {
        Ok(blueprints) => {
            let response = GetBlueprintsResponse { blueprints };
            (StatusCode::OK, Json(response)).into_response()
        }
        Err(e) => {
            tracing::debug!("error encountered listing blueprints");
            let error = json!({
                "error": "BlueprintListFailed",
                "message": e.to_string()
            });
            (StatusCode::INTERNAL_SERVER_ERROR, Json(error)).into_response()
        }
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct GetBlueprintQuery {
    pub aturi: String,
}

#[derive(Debug, Serialize)]
pub(super) struct GetBlueprintResponse {
    pub blueprint: Blueprint,
    pub nodes: Vec<Node>,
}

pub(super) async fn handle_get_blueprint(
    State(context): State<WebContext>,
    Query(params): Query<GetBlueprintQuery>,
    jar: PrivateCookieJar,
    headers: HeaderMap,
) -> impl IntoResponse {
    // Require authentication
    let authenticated_did = match get_authenticated_did(&context, &jar, &headers).await {
        Ok(auth) => auth.did().to_string(),
        Err(error_response) => return error_response.into_response(),
    };

    let blueprint = match context.blueprint_storage.get_blueprint(&params.aturi).await {
        Ok(Some(bp)) => bp,
        Ok(None) => {
            let error = json!({
                "error": "BlueprintNotFound",
                "message": format!("Blueprint {} not found", params.aturi)
            });
            return (StatusCode::NOT_FOUND, Json(error)).into_response();
        }
        Err(e) => {
            let error = json!({
                "error": "BlueprintFetchFailed",
                "message": e.to_string()
            });
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(error)).into_response();
        }
    };

    // Verify the blueprint belongs to the authenticated user
    if blueprint.did != authenticated_did {
        let error = json!({
            "error": "Forbidden",
            "message": "You can only access your own blueprints"
        });
        return (StatusCode::FORBIDDEN, Json(error)).into_response();
    }

    let nodes = match context.node_storage.list_nodes(&params.aturi).await {
        Ok(nodes) => nodes,
        Err(e) => {
            let error = json!({
                "error": "NodesFetchFailed",
                "message": e.to_string()
            });
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(error)).into_response();
        }
    };

    let response = GetBlueprintResponse { blueprint, nodes };
    (StatusCode::OK, Json(response)).into_response()
}

#[derive(Debug, Deserialize)]
pub(super) struct UpdateBlueprintRequest {
    pub aturi: String,
    // Note: This field is kept for API compatibility but ignored in favor of authenticated_did
    #[allow(dead_code)]
    pub did: String,
    pub node_order: Vec<String>,
    pub nodes: Vec<NodeUpdate>,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

#[derive(Debug, Deserialize)]
pub(super) struct NodeUpdate {
    pub aturi: Option<String>,
    pub node_type: String,
    pub payload: serde_json::Value,
    #[serde(default = "default_configuration")]
    pub configuration: serde_json::Value,
}

pub(super) async fn handle_update_blueprint(
    State(context): State<WebContext>,
    jar: PrivateCookieJar,
    headers: HeaderMap,
    Json(request): Json<UpdateBlueprintRequest>,
) -> impl IntoResponse {
    // Require authentication
    let authenticated_did = match get_authenticated_did(&context, &jar, &headers).await {
        Ok(auth) => auth.did().to_string(),
        Err(error_response) => return error_response.into_response(),
    };

    // Check if the user is allowed to update blueprints
    if !is_allowed_identity(&context.config.allowed_identities, &authenticated_did) {
        let error = json!({
            "error": "Forbidden",
            "message": "Features are limited to a waitlist at this time"
        });
        return (StatusCode::FORBIDDEN, Json(error)).into_response();
    }

    if !request.aturi.starts_with("at://") {
        let error = json!({
            "error": "InvalidAtUri",
            "message": "AT-URI must start with at://"
        });
        return (StatusCode::BAD_REQUEST, Json(error)).into_response();
    }

    // Verify the AT-URI belongs to the authenticated user
    if !request
        .aturi
        .starts_with(&format!("at://{}/", authenticated_did))
    {
        let error = json!({
            "error": "InvalidAtUri",
            "message": "AT-URI must belong to the authenticated user"
        });
        return (StatusCode::BAD_REQUEST, Json(error)).into_response();
    }

    // Convert NodeUpdate to Node for validation
    let mut nodes = Vec::new();
    for node_update in &request.nodes {
        let aturi = node_update.aturi.clone().unwrap_or_else(|| {
            format!(
                "at://{}/tools.graze.ifthisthenat.node.definition/{}",
                authenticated_did,
                Ulid::new()
            )
        });

        nodes.push(Node {
            aturi,
            blueprint: request.aturi.clone(),
            node_type: node_update.node_type.clone(),
            payload: node_update.payload.clone(),
            configuration: node_update.configuration.clone(),
            created_at: Utc::now(),
        });
    }

    // Validate blueprint nodes using the centralized validator
    if let Err(e) = Validator::validate_blueprint_nodes(&nodes) {
        let error = json!({
            "error": "InvalidBlueprint",
            "message": e.to_string()
        });
        return (StatusCode::BAD_REQUEST, Json(error)).into_response();
    }

    // Resolve handles to DIDs for jetstream_entry nodes and validate configurations
    for node in &mut nodes {
        // Resolve handles if it's a jetstream_entry node
        if node.node_type == "jetstream_entry" {
            if let Some(did_filter) = node.configuration.get_mut("did") {
                if let Some(dids_array) = did_filter.as_array() {
                    // Extract strings from the JSON array
                    let input_strings: Vec<String> = dids_array
                        .iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect();

                    // Use filter_map_dids to resolve handles to DIDs
                    match filter_map_dids(
                        &context.http_client,
                        &context.config.resolve_handle_hostname,
                        input_strings,
                    )
                    .await
                    {
                        Ok(resolved_dids) => {
                            // Replace the array with resolved DIDs
                            *did_filter = serde_json::json!(resolved_dids);
                        }
                        Err(e) => {
                            let error = json!({
                                "error": "HandleResolutionFailed",
                                "message": format!("Failed to resolve handles to DIDs: {}", e)
                            });
                            return (StatusCode::BAD_REQUEST, Json(error)).into_response();
                        }
                    }
                }
            }
        }

        // Validate node configuration (handles are now resolved to DIDs)
        if let Err(e) = Validator::validate_node_config(
            &node.node_type,
            &node.configuration,
            &authenticated_did,
        ) {
            let error = json!({
                "error": "InvalidConfiguration",
                "message": e.to_string()
            });
            return (StatusCode::BAD_REQUEST, Json(error)).into_response();
        }
    }

    let blueprint = Blueprint {
        aturi: request.aturi.clone(),
        did: authenticated_did.clone(),
        node_order: request.node_order.clone(),
        enabled: request.enabled,
        error: None, // Clear any previous error when updating
        created_at: Utc::now(),
    };

    if let Err(e) = context.blueprint_storage.create_blueprint(&blueprint).await {
        let error = json!({
            "error": "BlueprintCreateFailed",
            "message": e.to_string()
        });
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(error)).into_response();
    }

    if let Err(e) = context
        .node_storage
        .delete_nodes_for_blueprint(&request.aturi)
        .await
    {
        let error = json!({
            "error": "NodeDeleteFailed",
            "message": e.to_string()
        });
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(error)).into_response();
    }

    // Store all nodes (now with resolved handles)
    for node in &nodes {
        if let Err(e) = context.node_storage.upsert_node(node).await {
            let error = json!({
                "error": "NodeUpsertFailed",
                "message": e.to_string()
            });
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(error)).into_response();
        }
    }

    // Trigger cache reload in the background
    context.trigger_blueprint_cache_reload().await;

    let response = json!({"success": true});
    (StatusCode::OK, Json(response)).into_response()
}

#[derive(Debug, Deserialize)]
pub(super) struct CreateBlueprintRequest {
    pub nodes: Vec<CreateNodeRequest>,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

fn default_enabled() -> bool {
    true
}

#[derive(Debug, Deserialize)]
pub(super) struct CreateNodeRequest {
    pub node_type: String,
    pub payload: serde_json::Value,
    #[serde(default = "default_configuration")]
    pub configuration: serde_json::Value,
}

#[derive(Debug, Serialize)]
pub(super) struct CreateBlueprintResponse {
    pub blueprint: Blueprint,
    pub nodes: Vec<Node>,
}

pub(super) async fn handle_create_blueprint(
    State(context): State<WebContext>,
    jar: PrivateCookieJar,
    headers: HeaderMap,
    Json(request): Json<CreateBlueprintRequest>,
) -> impl IntoResponse {
    // Require authentication
    let authenticated_did = match get_authenticated_did(&context, &jar, &headers).await {
        Ok(auth) => auth.did().to_string(),
        Err(error_response) => return error_response.into_response(),
    };

    // Check if the user is allowed to create blueprints
    if !is_allowed_identity(&context.config.allowed_identities, &authenticated_did) {
        let error = json!({
            "error": "Forbidden",
            "message": "Features are limited to a waitlist at this time"
        });
        return (StatusCode::FORBIDDEN, Json(error)).into_response();
    }

    // Validate that nodes array is not empty
    if request.nodes.is_empty() {
        let error = json!({
            "error": "InvalidRequest",
            "message": "At least one node is required to create a blueprint"
        });
        return (StatusCode::BAD_REQUEST, Json(error)).into_response();
    }

    // Generate blueprint AT-URI
    let blueprint_aturi = format!(
        "at://{}/tools.graze.ifthisthenat.blueprint/{}",
        authenticated_did,
        Ulid::new()
    );

    // Convert CreateNodeRequest to Node with generated AT-URIs and collect node_order
    let mut nodes = Vec::new();
    let mut node_order = Vec::new();

    for node_request in request.nodes {
        let node_aturi = format!(
            "at://{}/tools.graze.ifthisthenat.node.definition/{}",
            authenticated_did,
            Ulid::new()
        );

        node_order.push(node_aturi.clone());

        nodes.push(Node {
            aturi: node_aturi,
            blueprint: blueprint_aturi.clone(),
            node_type: node_request.node_type,
            payload: node_request.payload,
            configuration: node_request.configuration,
            created_at: Utc::now(),
        });
    }

    // Validate blueprint nodes using the centralized validator
    if let Err(e) = Validator::validate_blueprint_nodes(&nodes) {
        let error = json!({
            "error": "InvalidBlueprint",
            "message": e.to_string()
        });
        return (StatusCode::BAD_REQUEST, Json(error)).into_response();
    }

    // Resolve handles to DIDs for jetstream_entry nodes and validate configurations
    for node in &mut nodes {
        // Resolve handles if it's a jetstream_entry node
        if node.node_type == "jetstream_entry" {
            if let Some(did_filter) = node.configuration.get_mut("did") {
                if let Some(dids_array) = did_filter.as_array() {
                    // Extract strings from the JSON array
                    let input_strings: Vec<String> = dids_array
                        .iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect();

                    // Use filter_map_dids to resolve handles to DIDs
                    match filter_map_dids(
                        &context.http_client,
                        &context.config.resolve_handle_hostname,
                        input_strings,
                    )
                    .await
                    {
                        Ok(resolved_dids) => {
                            // Replace the array with resolved DIDs
                            *did_filter = serde_json::json!(resolved_dids);
                        }
                        Err(e) => {
                            let error = json!({
                                "error": "HandleResolutionFailed",
                                "message": format!("Failed to resolve handles to DIDs: {}", e)
                            });
                            return (StatusCode::BAD_REQUEST, Json(error)).into_response();
                        }
                    }
                }
            }
        }

        // Validate node configuration (handles are now resolved to DIDs)
        if let Err(e) = Validator::validate_node_config(
            &node.node_type,
            &node.configuration,
            &authenticated_did,
        ) {
            let error = json!({
                "error": "InvalidConfiguration",
                "message": e.to_string()
            });
            return (StatusCode::BAD_REQUEST, Json(error)).into_response();
        }
    }

    // Create the blueprint
    let blueprint = Blueprint {
        aturi: blueprint_aturi.clone(),
        did: authenticated_did.clone(),
        node_order: node_order.clone(),
        enabled: request.enabled,
        error: None, // New blueprints start without errors
        created_at: Utc::now(),
    };

    // Store the blueprint
    if let Err(e) = context.blueprint_storage.create_blueprint(&blueprint).await {
        let error = json!({
            "error": "BlueprintCreateFailed",
            "message": e.to_string()
        });
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(error)).into_response();
    }

    // Store all nodes
    for node in &nodes {
        if let Err(e) = context.node_storage.upsert_node(node).await {
            // If node creation fails, try to clean up the blueprint
            let _ = context
                .blueprint_storage
                .delete_blueprint(&blueprint_aturi)
                .await;

            let error = json!({
                "error": "NodeCreateFailed",
                "message": e.to_string()
            });
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(error)).into_response();
        }
    }

    // Trigger cache reload in the background
    context.trigger_blueprint_cache_reload().await;

    // Return the created blueprint and nodes
    let response = CreateBlueprintResponse { blueprint, nodes };

    (StatusCode::CREATED, Json(response)).into_response()
}

#[derive(Debug, Deserialize)]
pub(super) struct DeleteBlueprintRequest {
    pub aturi: String,
}

pub(super) async fn handle_delete_blueprint(
    State(context): State<WebContext>,
    jar: PrivateCookieJar,
    headers: HeaderMap,
    Json(request): Json<DeleteBlueprintRequest>,
) -> impl IntoResponse {
    // Require authentication
    let authenticated_did = match get_authenticated_did(&context, &jar, &headers).await {
        Ok(auth) => auth.did().to_string(),
        Err(error_response) => return error_response.into_response(),
    };

    // Check if the user is allowed to delete blueprints
    if !is_allowed_identity(&context.config.allowed_identities, &authenticated_did) {
        let error = json!({
            "error": "Forbidden",
            "message": "Features are limited to a waitlist at this time"
        });
        return (StatusCode::FORBIDDEN, Json(error)).into_response();
    }

    // Verify the AT-URI belongs to the authenticated user
    if !request
        .aturi
        .starts_with(&format!("at://{}/", authenticated_did))
    {
        let error = json!({
            "error": "Forbidden",
            "message": "You can only delete your own blueprints"
        });
        return (StatusCode::FORBIDDEN, Json(error)).into_response();
    }

    // Optionally, fetch the blueprint first to verify it exists and belongs to the user
    if let Ok(Some(blueprint)) = context
        .blueprint_storage
        .get_blueprint(&request.aturi)
        .await
        && blueprint.did != authenticated_did
    {
        let error = json!({
            "error": "Forbidden",
            "message": "You can only delete your own blueprints"
        });
        return (StatusCode::FORBIDDEN, Json(error)).into_response();
    }

    if let Err(e) = context
        .blueprint_storage
        .delete_blueprint(&request.aturi)
        .await
    {
        let error = json!({
            "error": "BlueprintDeleteFailed",
            "message": e.to_string()
        });
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(error)).into_response();
    }

    // Trigger cache reload in the background
    context.trigger_blueprint_cache_reload().await;

    let response = json!({"success": true});
    (StatusCode::OK, Json(response)).into_response()
}

#[derive(Debug, Deserialize)]
pub(super) struct SetNodeRequest {
    pub aturi: String,
    pub blueprint: String,
    pub node_type: String,
    pub payload: serde_json::Value,
    #[serde(default = "default_configuration")]
    pub configuration: serde_json::Value,
}

fn default_configuration() -> serde_json::Value {
    json!({})
}

pub(super) async fn handle_set_node(
    State(context): State<WebContext>,
    jar: PrivateCookieJar,
    headers: HeaderMap,
    Json(request): Json<SetNodeRequest>,
) -> impl IntoResponse {
    // Require authentication
    let authenticated_did = match get_authenticated_did(&context, &jar, &headers).await {
        Ok(auth) => auth.did().to_string(),
        Err(error_response) => return error_response.into_response(),
    };

    // Check if the user is allowed to set nodes
    if !is_allowed_identity(&context.config.allowed_identities, &authenticated_did) {
        let error = json!({
            "error": "Forbidden",
            "message": "Features are limited to a waitlist at this time"
        });
        return (StatusCode::FORBIDDEN, Json(error)).into_response();
    }

    // Verify the node AT-URI belongs to the authenticated user
    if !request
        .aturi
        .starts_with(&format!("at://{}/", authenticated_did))
    {
        let error = json!({
            "error": "Forbidden",
            "message": "Node AT-URI must belong to the authenticated user"
        });
        return (StatusCode::FORBIDDEN, Json(error)).into_response();
    }

    // Verify the blueprint AT-URI belongs to the authenticated user
    if !request
        .blueprint
        .starts_with(&format!("at://{}/", authenticated_did))
    {
        let error = json!({
            "error": "Forbidden",
            "message": "Blueprint must belong to the authenticated user"
        });
        return (StatusCode::FORBIDDEN, Json(error)).into_response();
    }

    // Validate node type
    if let Err(e) = Validator::validate_node_type(&request.node_type) {
        let error = json!({
            "error": "InvalidNodeType",
            "message": e.to_string()
        });
        return (StatusCode::BAD_REQUEST, Json(error)).into_response();
    }

    // Check if this would be the first node in the blueprint
    let position = if let Ok(Some(blueprint)) = context
        .blueprint_storage
        .get_blueprint(&request.blueprint)
        .await
    {
        blueprint.node_order.len()
    } else {
        0
    };

    // Validate node position
    if let Err(e) = Validator::validate_node_position(&request.node_type, position) {
        let error = json!({
            "error": "InvalidNodePosition",
            "message": e.to_string()
        });
        return (StatusCode::BAD_REQUEST, Json(error)).into_response();
    }

    // Resolve handles to DIDs for jetstream_entry nodes
    let mut resolved_configuration = request.configuration.clone();
    if request.node_type == "jetstream_entry" {
        if let Some(did_filter) = resolved_configuration.get_mut("did") {
            if let Some(dids_array) = did_filter.as_array() {
                // Extract strings from the JSON array
                let input_strings: Vec<String> = dids_array
                    .iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect();

                // Use filter_map_dids to resolve handles to DIDs
                match filter_map_dids(
                    &context.http_client,
                    &context.config.resolve_handle_hostname,
                    input_strings,
                )
                .await
                {
                    Ok(resolved_dids) => {
                        // Replace the array with resolved DIDs
                        *did_filter = serde_json::json!(resolved_dids);
                    }
                    Err(e) => {
                        let error = json!({
                            "error": "HandleResolutionFailed",
                            "message": format!("Failed to resolve handles to DIDs: {}", e)
                        });
                        return (StatusCode::BAD_REQUEST, Json(error)).into_response();
                    }
                }
            }
        }
    }

    // Validate node configuration (handles are now resolved to DIDs)
    if let Err(e) = Validator::validate_node_config(
        &request.node_type,
        &resolved_configuration,
        &authenticated_did,
    ) {
        let error = json!({
            "error": "InvalidConfiguration",
            "message": e.to_string()
        });
        return (StatusCode::BAD_REQUEST, Json(error)).into_response();
    }

    let node = Node {
        aturi: request.aturi,
        blueprint: request.blueprint,
        node_type: request.node_type,
        payload: request.payload,
        configuration: resolved_configuration,
        created_at: Utc::now(),
    };

    if let Err(e) = context.node_storage.upsert_node(&node).await {
        let error = json!({
            "error": "NodeSetFailed",
            "message": e.to_string()
        });
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(error)).into_response();
    }

    let response = json!({"success": true});
    (StatusCode::OK, Json(response)).into_response()
}

#[derive(Debug, Deserialize)]
pub(super) struct DeleteNodeRequest {
    pub aturi: String,
}

pub(super) async fn handle_delete_node(
    State(context): State<WebContext>,
    jar: PrivateCookieJar,
    headers: HeaderMap,
    Json(request): Json<DeleteNodeRequest>,
) -> impl IntoResponse {
    // Require authentication
    let authenticated_did = match get_authenticated_did(&context, &jar, &headers).await {
        Ok(auth) => auth.did().to_string(),
        Err(error_response) => return error_response.into_response(),
    };

    // Check if the user is allowed to delete nodes
    if !is_allowed_identity(&context.config.allowed_identities, &authenticated_did) {
        let error = json!({
            "error": "Forbidden",
            "message": "Features are limited to a waitlist at this time"
        });
        return (StatusCode::FORBIDDEN, Json(error)).into_response();
    }

    // Verify the node AT-URI belongs to the authenticated user
    if !request
        .aturi
        .starts_with(&format!("at://{}/", authenticated_did))
    {
        let error = json!({
            "error": "Forbidden",
            "message": "You can only delete your own nodes"
        });
        return (StatusCode::FORBIDDEN, Json(error)).into_response();
    }

    // Optionally, fetch the node to verify it exists and get its blueprint
    if let Ok(Some(node)) = context.node_storage.get_node(&request.aturi).await {
        // Verify the blueprint also belongs to the user
        if !node
            .blueprint
            .starts_with(&format!("at://{}/", authenticated_did))
        {
            let error = json!({
                "error": "Forbidden",
                "message": "Node belongs to a blueprint you don't own"
            });
            return (StatusCode::FORBIDDEN, Json(error)).into_response();
        }

        // Also need to update the blueprint's node_order to remove this node
        if let Ok(Some(mut blueprint)) = context
            .blueprint_storage
            .get_blueprint(&node.blueprint)
            .await
        {
            blueprint.node_order.retain(|n| n != &request.aturi);
            let _ = context.blueprint_storage.update_blueprint(&blueprint).await;
        }
    }

    if let Err(e) = context.node_storage.delete_node(&request.aturi).await {
        let error = json!({
            "error": "NodeDeleteFailed",
            "message": e.to_string()
        });
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(error)).into_response();
    }

    let response = json!({"success": true});
    (StatusCode::OK, Json(response)).into_response()
}

#[derive(Debug, Deserialize)]
pub struct EvaluateBlueprintRequest {
    pub aturi: String,
    pub payload: serde_json::Value,
}

#[derive(Debug, Serialize)]
pub struct EvaluateBlueprintResponse {
    pub success: bool,
    pub evaluation_id: String,
}

/// Handler for evaluating a blueprint with a given payload
///
/// This XRPC endpoint allows external services (like Zapier integration) to
/// trigger blueprint evaluation directly by providing the blueprint AT-URI
/// and the input payload.
pub async fn handle_evaluate_blueprint(
    State(context): State<WebContext>,
    jar: PrivateCookieJar,
    headers: HeaderMap,
    Json(request): Json<EvaluateBlueprintRequest>,
) -> impl IntoResponse {
    // Require authentication
    let authenticated_did = match get_authenticated_did(&context, &jar, &headers).await {
        Ok(auth) => auth.did().to_string(),
        Err(error_response) => return error_response.into_response(),
    };

    // Check if the user is allowed to evaluate blueprints
    if !is_allowed_identity(&context.config.allowed_identities, &authenticated_did) {
        let error = json!({
            "error": "Forbidden",
            "message": "Features are limited to a waitlist at this time"
        });
        return (StatusCode::FORBIDDEN, Json(error)).into_response();
    }

    // Fetch the blueprint
    let blueprint = match context
        .blueprint_storage
        .get_blueprint(&request.aturi)
        .await
    {
        Ok(Some(bp)) => bp,
        Ok(None) => {
            let error = json!({
                "error": "BlueprintNotFound",
                "message": format!("Blueprint {} not found", request.aturi)
            });
            return (StatusCode::NOT_FOUND, Json(error)).into_response();
        }
        Err(e) => {
            tracing::error!(error = ?e, "Failed to fetch blueprint");
            let error = json!({
                "error": "BlueprintFetchFailed",
                "message": e.to_string()
            });
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(error)).into_response();
        }
    };

    // Verify the blueprint belongs to the authenticated user
    if blueprint.did != authenticated_did {
        let error = json!({
            "error": "Forbidden",
            "message": "You can only evaluate your own blueprints"
        });
        return (StatusCode::FORBIDDEN, Json(error)).into_response();
    }

    // Check if blueprint is enabled
    if !blueprint.enabled {
        let error = json!({
            "error": "BlueprintDisabled",
            "message": "Blueprint is disabled and cannot be evaluated"
        });
        return (StatusCode::BAD_REQUEST, Json(error)).into_response();
    }

    // Fetch nodes for the blueprint
    let nodes = match context.node_storage.list_nodes(&blueprint.aturi).await {
        Ok(nodes) => nodes,
        Err(e) => {
            tracing::error!(error = ?e, blueprint = %blueprint.aturi, "Failed to fetch nodes");
            let error = json!({
                "error": "NodesFetchFailed",
                "message": e.to_string()
            });
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(error)).into_response();
        }
    };

    // Validate blueprint has nodes
    if nodes.is_empty() {
        let error = json!({
            "error": "InvalidBlueprint",
            "message": "Blueprint has no nodes"
        });
        return (StatusCode::BAD_REQUEST, Json(error)).into_response();
    }

    // Validate the blueprint structure
    if let Err(e) = Validator::validate_blueprint_nodes(&nodes) {
        tracing::warn!(blueprint = %blueprint.aturi, error = ?e, "Invalid blueprint structure");
        let error = json!({
            "error": "InvalidBlueprint",
            "message": e.to_string()
        });
        return (StatusCode::BAD_REQUEST, Json(error)).into_response();
    }

    // Validate publish_record collections if constraints are configured
    if let Err(e) =
        Validator::validate_publish_collections(&nodes, &context.config.allowed_publish_collections)
    {
        tracing::warn!(
            blueprint = %blueprint.aturi,
            error = ?e,
            "Blueprint contains disallowed collections"
        );
        let error = json!({
            "error": "BlueprintValidationFailed",
            "message": e.to_string()
        });
        return (StatusCode::FORBIDDEN, Json(error)).into_response();
    }

    // Check if the first node is an entry node
    let entry_node = &nodes[0];
    let valid_entry_types = [
        "webhook_entry",
        "zap_entry",
        "jetstream_entry",
        "periodic_entry",
    ];

    if !valid_entry_types.contains(&entry_node.node_type.as_str()) {
        tracing::warn!(
            blueprint = %blueprint.aturi,
            node_type = %entry_node.node_type,
            "Blueprint does not have a valid entry node as first node"
        );
        let error = json!({
            "error": "InvalidBlueprintType",
            "message": format!("Blueprint must have an entry node as first node, found: {}", entry_node.node_type)
        });
        return (StatusCode::BAD_REQUEST, Json(error)).into_response();
    }

    // Get the blueprint sender from context
    let blueprint_sender = match context.blueprint_sender() {
        Some(sender) => sender,
        None => {
            tracing::error!("Blueprint evaluation not configured");
            let error = json!({
                "error": "ServiceUnavailable",
                "message": "Blueprint evaluation is not configured"
            });
            return (StatusCode::SERVICE_UNAVAILABLE, Json(error)).into_response();
        }
    };

    // Generate a unique evaluation ID
    let evaluation_id = uuid::Uuid::new_v4().to_string();

    // Submit the blueprint for async evaluation
    tracing::info!(
        blueprint = %blueprint.aturi,
        evaluation_id = %evaluation_id,
        "Submitting blueprint for evaluation via XRPC"
    );

    match submit_blueprint(
        blueprint_sender,
        blueprint.aturi.clone(),
        request.payload,
        Some(evaluation_id.clone()),
    )
    .await
    {
        Ok(_) => {
            tracing::debug!(blueprint = %blueprint.aturi, "Blueprint submitted for evaluation");
            let response = EvaluateBlueprintResponse {
                success: true,
                evaluation_id,
            };
            (StatusCode::ACCEPTED, Json(response)).into_response()
        }
        Err(e) => {
            tracing::error!(error = ?e, blueprint = %blueprint.aturi, "Failed to submit blueprint");
            let error = json!({
                "error": "EvaluationFailed",
                "message": format!("Failed to queue evaluation: {}", e)
            });
            (StatusCode::INTERNAL_SERVER_ERROR, Json(error)).into_response()
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct EvaluateNodeRequest {
    pub node_type: String,
    pub configuration: serde_json::Value,
    pub payload: serde_json::Value,
    pub input: serde_json::Value,
}

#[derive(Debug, Serialize)]
pub struct EvaluateNodeResponse {
    pub success: bool,
    pub output: Option<serde_json::Value>,
    pub message: Option<String>,
}

/// Handler for evaluating a single node with given configuration and payload
///
/// This XRPC endpoint allows testing and debugging individual nodes by
/// evaluating them in isolation with specific configuration and input data.
pub async fn handle_evaluate_node(
    State(context): State<WebContext>,
    jar: PrivateCookieJar,
    headers: HeaderMap,
    Json(request): Json<EvaluateNodeRequest>,
) -> impl IntoResponse {
    // Require authentication and get the auth method
    let auth_method = match get_authenticated_did(&context, &jar, &headers).await {
        Ok(auth) => auth,
        Err(error_response) => return error_response.into_response(),
    };

    let authenticated_did = auth_method.did().to_string();

    // Check if the user is allowed to evaluate nodes
    if !is_allowed_identity(&context.config.allowed_identities, &authenticated_did) {
        let error = json!({
            "error": "Forbidden",
            "message": "Features are limited to a waitlist at this time"
        });
        return (StatusCode::FORBIDDEN, Json(error)).into_response();
    }

    // Validate the node type is not disabled
    if context
        .config
        .disabled_node_types
        .contains(&request.node_type)
    {
        let error = json!({
            "error": "NodeTypeDisabled",
            "message": format!("Node type '{}' is disabled", request.node_type)
        });
        return (StatusCode::FORBIDDEN, Json(error)).into_response();
    }

    // Create a minimal node evaluator factory
    // Note: Some evaluators require additional dependencies which we'll handle with defaults
    let factory = std::sync::Arc::new(
        NodeEvaluatorFactory::new()
            .register("condition", std::sync::Arc::new(ConditionEvaluator::new()))
            .register("transform", std::sync::Arc::new(TransformEvaluator::new()))
            .register(
                "jetstream_entry",
                std::sync::Arc::new(JetstreamEntryEvaluator::new()),
            )
            .register(
                "periodic_entry",
                std::sync::Arc::new(PeriodicEntryEvaluator::new()),
            )
            .register(
                "webhook_entry",
                std::sync::Arc::new(WebhookEntryEvaluator::new()),
            )
            .register("zap_entry", std::sync::Arc::new(ZapEntryEvaluator::new()))
            .register(
                "debug_action",
                std::sync::Arc::new(DebugActionEvaluator::new()),
            )
            // For facet_text, we need an identity resolver - use the one from context
            .register(
                "facet_text",
                std::sync::Arc::new(FacetTextEvaluator::new(context.identity_resolver.clone())),
            )
            // For parse_aturi, simple evaluator with no dependencies
            .register(
                "parse_aturi",
                std::sync::Arc::new(ParseAturiEvaluator::new()),
            )
            // For get_record, we need http client and identity resolver
            .register(
                "get_record",
                std::sync::Arc::new(GetRecordEvaluator::new(
                    std::sync::Arc::new(context.http_client.clone()),
                    context.identity_resolver.clone(),
                )),
            )
            // For sentiment_analysis, simple evaluator with no dependencies
            .register(
                "sentiment_analysis",
                std::sync::Arc::new(SentimentAnalysisEvaluator::new()),
            )
            // For publish_webhook, use the direct evaluator for immediate execution
            .register(
                "publish_webhook",
                std::sync::Arc::new(PublishWebhookDirectEvaluator::new(std::sync::Arc::new(
                    context.http_client.clone(),
                ))),
            )
            // For publish_record, we need more complex setup - create with context dependencies
            // Use the service token manager for publish_record operations
            .register(
                "publish_record",
                std::sync::Arc::new(PublishRecordEvaluator::new(
                    std::sync::Arc::new(context.http_client.clone()),
                    context.service_token_manager.clone(),
                )),
            ),
    );

    // Check if the node type is supported
    if !factory.supports(&request.node_type) {
        let error = json!({
            "error": "UnsupportedNodeType",
            "message": format!("Node type '{}' is not supported", request.node_type)
        });
        return (StatusCode::BAD_REQUEST, Json(error)).into_response();
    }

    // Validate node configuration if it's a publish_webhook or publish_record node
    if request.node_type == "publish_webhook" || request.node_type == "publish_record" {
        if let Err(e) = Validator::validate_node_config(
            &request.node_type,
            &request.configuration,
            &authenticated_did,
        ) {
            let error = json!({
                "error": "InvalidConfiguration",
                "message": e.to_string()
            });
            return (StatusCode::BAD_REQUEST, Json(error)).into_response();
        }
    }

    // Create a temporary node for evaluation
    let node = Node {
        aturi: format!("at://{}/temp.node/{}", authenticated_did, Ulid::new()),
        blueprint: format!("at://{}/temp.blueprint/{}", authenticated_did, Ulid::new()),
        node_type: request.node_type.clone(),
        payload: request.payload,
        configuration: request.configuration,
        created_at: Utc::now(),
    };

    // Evaluate the node
    let result = match factory.evaluate(&node, &request.input).await {
        Ok(Some(output)) => {
            tracing::debug!(
                node_type = %request.node_type,
                "Node evaluation succeeded with output"
            );
            EvaluateNodeResponse {
                success: true,
                output: Some(output),
                message: None,
            }
        }
        Ok(None) => {
            tracing::debug!(
                node_type = %request.node_type,
                "Node evaluation succeeded with no output (filtered)"
            );
            EvaluateNodeResponse {
                success: true,
                output: None,
                message: Some("Node evaluation succeeded but filtered out the data".to_string()),
            }
        }
        Err(e) => {
            tracing::warn!(
                node_type = %request.node_type,
                error = ?e,
                "Node evaluation failed"
            );
            EvaluateNodeResponse {
                success: false,
                output: None,
                message: Some(e.to_string()),
            }
        }
    };

    (StatusCode::OK, Json(result)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{
        node_type_condition::ConditionEvaluator, node_type_debug_action::DebugActionEvaluator,
        node_type_transform::TransformEvaluator,
    };
    use serde_json::json;
    use std::sync::Arc;

    #[tokio::test]
    async fn test_evaluate_node_condition_true() {
        let request = EvaluateNodeRequest {
            node_type: "condition".to_string(),
            configuration: json!({}),
            payload: json!({"==": [{"val": ["status"]}, "active"]}),
            input: json!({"status": "active"}),
        };

        // Create a factory and test evaluation
        let factory = Arc::new(
            NodeEvaluatorFactory::new().register("condition", Arc::new(ConditionEvaluator::new())),
        );

        let node = Node {
            node_type: "condition".to_string(),
            aturi: "test".to_string(),
            blueprint: "test".to_string(),
            configuration: request.configuration.clone(),
            payload: request.payload.clone(),
            created_at: chrono::Utc::now(),
        };

        let result = factory.evaluate(&node, &request.input).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), Some(json!({"status": "active"})));
    }

    #[tokio::test]
    async fn test_evaluate_node_condition_false() {
        let request = EvaluateNodeRequest {
            node_type: "condition".to_string(),
            configuration: json!({}),
            payload: json!({"==": [{"val": ["status"]}, "active"]}),
            input: json!({"status": "inactive"}),
        };

        let factory = Arc::new(
            NodeEvaluatorFactory::new().register("condition", Arc::new(ConditionEvaluator::new())),
        );

        let node = Node {
            node_type: "condition".to_string(),
            aturi: "test".to_string(),
            blueprint: "test".to_string(),
            configuration: request.configuration.clone(),
            payload: request.payload.clone(),
            created_at: chrono::Utc::now(),
        };

        let result = factory.evaluate(&node, &request.input).await;
        assert!(result.is_ok());
        // When condition fails, returns None to stop processing
        assert_eq!(result.unwrap(), None);
    }

    #[tokio::test]
    async fn test_evaluate_node_transform() {
        let request = EvaluateNodeRequest {
            node_type: "transform".to_string(),
            configuration: json!({}),
            payload: json!({
                "greeting": {"cat": ["Hello, ", {"val": ["name"]}]},
                "timestamp": {"now": []}
            }),
            input: json!({"name": "World"}),
        };

        let factory = Arc::new(
            NodeEvaluatorFactory::new().register("transform", Arc::new(TransformEvaluator::new())),
        );

        let node = Node {
            node_type: "transform".to_string(),
            aturi: "test".to_string(),
            blueprint: "test".to_string(),
            configuration: request.configuration.clone(),
            payload: request.payload.clone(),
            created_at: chrono::Utc::now(),
        };

        let result = factory.evaluate(&node, &request.input).await;
        assert!(result.is_ok());
        let output = result.unwrap().unwrap();
        assert_eq!(output["greeting"], "Hello, World");
        // The now() function returns an ISO timestamp string
        assert!(output["timestamp"].is_string());
    }

    #[tokio::test]
    async fn test_evaluate_node_unsupported_type() {
        let request = EvaluateNodeRequest {
            node_type: "unsupported_type".to_string(),
            configuration: json!({}),
            payload: json!({}),
            input: json!({}),
        };

        let factory = Arc::new(NodeEvaluatorFactory::new());

        let node = Node {
            node_type: "unsupported_type".to_string(),
            aturi: "test".to_string(),
            blueprint: "test".to_string(),
            configuration: request.configuration.clone(),
            payload: json!({}),
            created_at: chrono::Utc::now(),
        };

        let result = factory.evaluate(&node, &request.input).await;
        assert!(result.is_err());
        // Check that the error mentions the unsupported type
        let error_msg = result.unwrap_err().to_string();
        assert!(error_msg.contains("unsupported_type") || error_msg.contains("Unsupported"));
    }

    #[tokio::test]
    async fn test_evaluate_node_debug_action() {
        let request = EvaluateNodeRequest {
            node_type: "debug_action".to_string(),
            configuration: json!({}),
            payload: json!({}),
            input: json!({"test": "data"}),
        };

        let factory = Arc::new(
            NodeEvaluatorFactory::new()
                .register("debug_action", Arc::new(DebugActionEvaluator::new())),
        );

        let node = Node {
            node_type: "debug_action".to_string(),
            aturi: "test".to_string(),
            blueprint: "test".to_string(),
            configuration: request.configuration.clone(),
            payload: json!({}),
            created_at: chrono::Utc::now(),
        };

        let result = factory.evaluate(&node, &request.input).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), Some(json!({"test": "data"})));
    }
}
