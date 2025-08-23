use anyhow::Result;
use axum::{
    Form,
    extract::{Path, State},
    response::{IntoResponse, Redirect},
};
use axum_extra::extract::PrivateCookieJar;
use axum_template::RenderHtml;
use chrono::Utc;
use minijinja::context;
use serde::Deserialize;
use ulid::Ulid;

use crate::{
    errors::HttpError,
    http::{
        auth::get_session_from_cookie, blueprint_helpers::*, context::WebContext, errors::WebError,
    },
    storage::{
        blueprint::BlueprintStorage,
        node::{Node, NodeStorage},
    },
    validation::Validator,
};

// NOTE: handle_blueprints_get and handle_create_blueprint handlers have been removed
// as they are unused. Dashboard handlers provide this functionality.

/// Handler for displaying the blueprint edit page
pub(super) async fn handle_blueprint_edit(
    State(web_context): State<WebContext>,
    jar: PrivateCookieJar,
    Path(record_key): Path<String>,
) -> Result<impl IntoResponse, WebError> {
    // Get session from cookie
    let session = get_session_from_cookie(&web_context, &jar).await?;

    // Compose the full AT-URI from the user's DID and record key
    let aturi = construct_blueprint_aturi(&session.did, &record_key);

    // Fetch the blueprint and verify ownership
    let blueprint = fetch_blueprint_with_ownership(&web_context, &aturi, &session.did).await?;

    // Fetch nodes for the blueprint
    let nodes = fetch_blueprint_nodes(&web_context, &aturi).await?;

    // Check if blueprint is valid
    let is_valid = blueprint.is_valid(&nodes);

    // Extract node keys from node ATURIs for cleaner URLs
    let nodes_with_keys: Vec<_> = nodes
        .into_iter()
        .map(|node| {
            let node_key = extract_record_key(&node.aturi);
            (node, node_key)
        })
        .collect();

    Ok(RenderHtml(
        "blueprint_edit.html",
        web_context.engine.clone(),
        context! {
            canonical_url => format!("{}/blueprints/{}/edit", web_context.config.external_base, record_key),
            user_did => session.did,
            blueprint => blueprint,
            nodes => nodes_with_keys,
            record_key => record_key,
            is_valid => is_valid,
        },
    )
    .into_response())
}

/// Form data for adding a node to a blueprint
#[derive(Debug, Deserialize)]
pub(super) struct AddNodeForm {
    pub node_type: String,
    pub payload: String,
    pub configuration: String,
}

/// Form data for updating a node
#[derive(Debug, Deserialize)]
pub(super) struct UpdateNodeForm {
    pub node_type: String,
    pub payload: String,
    pub configuration: String,
}

/// Handler for adding a node to a blueprint
pub(super) async fn handle_add_node(
    State(web_context): State<WebContext>,
    jar: PrivateCookieJar,
    Path(record_key): Path<String>,
    Form(form): Form<AddNodeForm>,
) -> Result<impl IntoResponse, WebError> {
    // Get session from cookie
    let session = get_session_from_cookie(&web_context, &jar).await?;

    // Compose the full AT-URI from the user's DID and record key
    let aturi = construct_blueprint_aturi(&session.did, &record_key);

    // Fetch the blueprint and verify ownership
    let mut blueprint = fetch_blueprint_with_ownership(&web_context, &aturi, &session.did).await?;

    // Validate node type
    if let Err(e) = Validator::validate_node_type(&form.node_type) {
        return Err(WebError::Http(HttpError::BadRequest {
            details: e.to_string(),
        }));
    }

    // Check node position
    let position = blueprint.node_order.len();
    if let Err(e) = Validator::validate_node_position(&form.node_type, position) {
        return Err(WebError::Http(HttpError::BadRequest {
            details: e.to_string(),
        }));
    }

    // Parse JSON payloads
    let payload: serde_json::Value = parse_json_with_error(&form.payload, "payload")?;
    let configuration: serde_json::Value =
        parse_optional_json(&form.configuration, "configuration")?;

    // Validate node configuration
    if let Err(e) = Validator::validate_node_config(&form.node_type, &configuration, &session.did) {
        return Err(WebError::Http(HttpError::BadRequest {
            details: e.to_string(),
        }));
    }

    // Generate AT-URI for the node
    let node_id = Ulid::new().to_string();
    let node_aturi = construct_node_aturi(&session.did, &node_id);

    // Create the node
    let node = Node {
        aturi: node_aturi.clone(),
        blueprint: aturi.clone(),
        node_type: form.node_type,
        payload,
        configuration,
        created_at: Utc::now(),
    };

    // Store the node
    web_context
        .node_storage()
        .upsert_node(&node)
        .await
        .map_err(|e| {
            WebError::Http(HttpError::Unhandled {
                details: format!("Failed to create node: {}", e),
            })
        })?;

    // Update blueprint's node_order
    blueprint.node_order.push(node_aturi);
    web_context
        .blueprint_storage()
        .update_blueprint(&blueprint)
        .await
        .map_err(|e| {
            WebError::Http(HttpError::Unhandled {
                details: format!("Failed to update blueprint: {}", e),
            })
        })?;

    // Trigger blueprint cache reload
    web_context.trigger_blueprint_cache_reload().await;

    // Redirect back to the edit page using just the record key
    Ok(Redirect::to(&format!(
        "/dashboard/{}/edit",
        record_key
    )))
}

/// Handler for deleting a node from a blueprint
pub(super) async fn handle_delete_node(
    State(web_context): State<WebContext>,
    jar: PrivateCookieJar,
    Path((blueprint_key, node_key)): Path<(String, String)>,
) -> Result<impl IntoResponse, WebError> {
    // Get session from cookie
    let session = get_session_from_cookie(&web_context, &jar).await?;

    // Compose the full AT-URIs from the user's DID and record keys
    let blueprint_aturi = construct_blueprint_aturi(&session.did, &blueprint_key);
    let node_aturi = construct_node_aturi(&session.did, &node_key);

    // Fetch the blueprint and verify ownership
    let mut blueprint =
        fetch_blueprint_with_ownership(&web_context, &blueprint_aturi, &session.did).await?;

    // Delete the node
    web_context
        .node_storage()
        .delete_node(&node_aturi)
        .await
        .map_err(|e| {
            WebError::Http(HttpError::Unhandled {
                details: format!("Failed to delete node: {}", e),
            })
        })?;

    // Update blueprint's node_order
    blueprint.node_order.retain(|n| n != &node_aturi);
    web_context
        .blueprint_storage()
        .update_blueprint(&blueprint)
        .await
        .map_err(|e| {
            WebError::Http(HttpError::Unhandled {
                details: format!("Failed to update blueprint: {}", e),
            })
        })?;

    // Trigger blueprint cache reload
    web_context.trigger_blueprint_cache_reload().await;

    // Redirect back to the edit page using just the record key
    Ok(Redirect::to(&format!(
        "/dashboard/{}/edit",
        blueprint_key
    )))
}

/// Handler for deleting a blueprint
pub(super) async fn handle_delete_blueprint(
    State(web_context): State<WebContext>,
    jar: PrivateCookieJar,
    Path(record_key): Path<String>,
) -> Result<impl IntoResponse, WebError> {
    // Get session from cookie
    let session = get_session_from_cookie(&web_context, &jar).await?;

    // Compose the full AT-URI from the user's DID and record key
    let aturi = construct_blueprint_aturi(&session.did, &record_key);

    // Fetch the blueprint and verify ownership
    let _blueprint = fetch_blueprint_with_ownership(&web_context, &aturi, &session.did).await?;

    // Delete all nodes associated with the blueprint
    delete_blueprint_nodes(&web_context, &aturi).await?;

    // Delete the blueprint
    web_context
        .blueprint_storage()
        .delete_blueprint(&aturi)
        .await
        .map_err(|e| {
            WebError::Http(HttpError::Unhandled {
                details: format!("Failed to delete blueprint: {}", e),
            })
        })?;

    // Trigger blueprint cache reload
    web_context.trigger_blueprint_cache_reload().await;

    // Redirect to the blueprints page
    Ok(Redirect::to("/dashboard"))
}

/// Handler for updating a node
pub(super) async fn handle_update_node(
    State(web_context): State<WebContext>,
    jar: PrivateCookieJar,
    Path((blueprint_key, node_key)): Path<(String, String)>,
    Form(form): Form<UpdateNodeForm>,
) -> Result<impl IntoResponse, WebError> {
    // Get session from cookie
    let session = get_session_from_cookie(&web_context, &jar).await?;

    // Compose the full AT-URIs from the user's DID and record keys
    let blueprint_aturi = construct_blueprint_aturi(&session.did, &blueprint_key);
    let node_aturi = construct_node_aturi(&session.did, &node_key);

    // Fetch the blueprint and verify ownership
    let blueprint =
        fetch_blueprint_with_ownership(&web_context, &blueprint_aturi, &session.did).await?;

    // Fetch the existing node to verify it exists and belongs to this blueprint
    let existing_node = web_context
        .node_storage()
        .get_node(&node_aturi)
        .await
        .map_err(|e| {
            WebError::Http(HttpError::Unhandled {
                details: format!("Failed to fetch node: {}", e),
            })
        })?
        .ok_or_else(|| {
            WebError::Http(HttpError::NotFound {
                details: "Node not found".to_string(),
            })
        })?;

    // Verify the node belongs to this blueprint
    if existing_node.blueprint != blueprint_aturi {
        return Err(WebError::Http(HttpError::Forbidden {
            details: "Node does not belong to this blueprint".to_string(),
        }));
    }

    // Validate node type
    if let Err(e) = Validator::validate_node_type(&form.node_type) {
        return Err(WebError::Http(HttpError::BadRequest {
            details: e.to_string(),
        }));
    }

    // Check if this is the first node in the blueprint being updated
    if !blueprint.node_order.is_empty() && blueprint.node_order[0] == node_aturi {
        // Validate position 0 for the new node type
        if let Err(e) = Validator::validate_node_position(&form.node_type, 0) {
            return Err(WebError::Http(HttpError::BadRequest {
                details: e.to_string(),
            }));
        }
    }

    // Parse JSON payloads
    let payload: serde_json::Value = parse_json_with_error(&form.payload, "payload")?;
    let configuration: serde_json::Value =
        parse_optional_json(&form.configuration, "configuration")?;

    // Validate node configuration
    if let Err(e) = Validator::validate_node_config(&form.node_type, &configuration, &session.did) {
        return Err(WebError::Http(HttpError::BadRequest {
            details: e.to_string(),
        }));
    }

    // Create updated node with the same AT-URI and created_at
    let updated_node = Node {
        aturi: node_aturi.clone(),
        blueprint: blueprint_aturi.clone(),
        node_type: form.node_type,
        payload,
        configuration,
        created_at: existing_node.created_at, // Preserve original creation time
    };

    // Update the node in storage
    web_context
        .node_storage()
        .upsert_node(&updated_node)
        .await
        .map_err(|e| {
            WebError::Http(HttpError::Unhandled {
                details: format!("Failed to update node: {}", e),
            })
        })?;

    // Trigger blueprint cache reload
    web_context.trigger_blueprint_cache_reload().await;

    // Redirect back to the edit page
    Ok(Redirect::to(&format!(
        "/dashboard/{}/edit",
        blueprint_key
    )))
}
