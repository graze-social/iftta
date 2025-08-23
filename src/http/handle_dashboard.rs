//! Dashboard handlers for authenticated users

use anyhow::Result;
use axum::{
    Form,
    extract::{Query, State},
    http::header,
    response::{IntoResponse, Redirect},
};
use axum_extra::extract::PrivateCookieJar;
use axum_template::RenderHtml;
use cookie::Cookie;
use minijinja::context;
use serde::Deserialize;
use ulid::Ulid;

use crate::{
    constants::is_entry_node_type,
    errors::HttpError,
    http::{
        auth::{get_session_from_cookie, is_allowed_identity},
        blueprint_helpers::*,
        context::WebContext,
        errors::WebError,
        handle_auth::AUTH_COOKIE_NAME,
    },
    storage::{
        blueprint::{Blueprint, BlueprintStorage},
        node::{Node, NodeStorage},
    },
    validation::Validator,
};

#[derive(Debug, Deserialize)]
pub struct DashboardQuery {
    page: Option<u32>,
    #[serde(default = "default_page_size")]
    size: u32,
}

fn default_page_size() -> u32 {
    10
}

/// Handle GET requests to show the dashboard page
pub(super) async fn handle_dashboard_get(
    State(web_context): State<WebContext>,
    jar: PrivateCookieJar,
    Query(query): Query<DashboardQuery>,
) -> Result<impl IntoResponse, WebError> {
    // Get session from cookie
    let session = get_session_from_cookie(&web_context, &jar).await?;

    // Check if the user is in the allowed identities list
    let allowed = is_allowed_identity(&web_context.config.allowed_identities, &session.did);

    let page = query.page.unwrap_or(1).max(1);
    let size = query.size.min(100).max(1); // Limit page size between 1 and 100
    let offset = (page - 1) * size;

    // Get total count of blueprints for pagination
    let total_blueprints = web_context
        .blueprint_storage
        .count_blueprints(Some(&session.did))
        .await
        .map_err(|e| {
            WebError::Http(HttpError::Unhandled {
                details: format!("Failed to count blueprints: {}", e),
            })
        })?;

    // Get blueprints for the user with pagination
    let blueprints = web_context
        .blueprint_storage
        .list_blueprints_paginated(Some(&session.did), size as usize, offset as usize)
        .await
        .map_err(|e| {
            WebError::Http(HttpError::Unhandled {
                details: format!("Failed to fetch blueprints: {}", e),
            })
        })?;

    // Extract record keys and validation status from blueprints
    let mut blueprints_with_data = Vec::new();
    for bp in blueprints {
        let record_key = extract_record_key(&bp.aturi);
        let nodes = fetch_blueprint_nodes(&web_context, &bp.aturi).await?;
        let is_valid = bp.is_valid(&nodes);
        let node_count = bp.node_order.len();
        blueprints_with_data.push((bp, record_key, is_valid, node_count));
    }

    // Check if user has app password set via AIP
    let has_app_password = if let Some(aip_base_url) = &web_context.config.aip_base_url {
        // Use the user's session access token to check app-password status
        match super::aip_utils::check_app_password_exists(
            &web_context.http_client,
            aip_base_url,
            &session.access_token,
        )
        .await
        {
            Ok(has_password) => has_password,
            Err(e) => {
                tracing::warn!(error = %e, "Failed to check app-password status via AIP, checking local storage");
                false
            }
        }
    } else {
        // AIP not configured, check local session storage
        match web_context
            .session_storage
            .get_session_by_did(&session.did)
            .await
        {
            Ok(Some(user_session)) => user_session.is_app_password(),
            _ => false,
        }
    };

    // Calculate pagination info
    let total_pages = ((total_blueprints as f64) / (size as f64)).ceil() as u32;
    let has_previous = page > 1;
    let has_next = page < total_pages;
    let previous_page = if has_previous { Some(page - 1) } else { None };
    let next_page = if has_next { Some(page + 1) } else { None };

    Ok(RenderHtml(
        "dashboard.html",
        web_context.engine.clone(),
        context! {
            canonical_url => format!("{}/dashboard", web_context.config.external_base),
            user_did => session.did,
            allowed => allowed,
            blueprints => blueprints_with_data,
            has_app_password => has_app_password,
            // Pagination
            current_page => page,
            page_size => size,
            total_blueprints => total_blueprints,
            total_pages => total_pages,
            has_previous => has_previous,
            has_next => has_next,
            previous_page => previous_page,
            next_page => next_page,
        },
    )
    .into_response())
}

/// Form data for creating a new blueprint with nodes
#[derive(Debug, Deserialize)]
pub(super) struct CreateBlueprintForm {
    blueprint_data: String, // JSON string containing nodes
}

/// Node data from the form
#[derive(Debug, Deserialize)]
struct NodeFormData {
    #[serde(rename = "type")]
    node_type: String,
    payload: serde_json::Value,
    configuration: serde_json::Value,
}

/// Blueprint form data
#[derive(Debug, Deserialize)]
struct BlueprintFormData {
    nodes: Vec<NodeFormData>,
}

/// Handler for creating a new blueprint with nodes
pub(super) async fn handle_create_blueprint(
    State(web_context): State<WebContext>,
    jar: PrivateCookieJar,
    Form(form): Form<CreateBlueprintForm>,
) -> Result<impl IntoResponse, WebError> {
    // Get session from cookie
    let session = get_session_from_cookie(&web_context, &jar).await?;

    // Check if the user is allowed to create blueprints
    if !is_allowed_identity(&web_context.config.allowed_identities, &session.did) {
        return Err(WebError::Http(HttpError::Forbidden {
            details: "Features are limited to a waitlist at this time".to_string(),
        }));
    }

    // Parse the blueprint data
    let blueprint_data: BlueprintFormData =
        parse_json_with_error(&form.blueprint_data, "blueprint data")?;

    // Generate a unique AT-URI for the blueprint
    let blueprint_id = Ulid::new().to_string();
    let blueprint_aturi = construct_blueprint_aturi(&session.did, &blueprint_id);

    // Create node AT-URIs and prepare nodes for storage
    let mut node_order = Vec::new();
    let mut nodes_to_create = Vec::new();

    for (_index, node_data) in blueprint_data.nodes.iter().enumerate() {
        let node_id = Ulid::new().to_string();
        let node_aturi = construct_node_aturi(&session.did, &node_id);

        node_order.push(node_aturi.clone());

        let node = Node {
            aturi: node_aturi,
            blueprint: blueprint_aturi.clone(),
            node_type: node_data.node_type.clone(),
            payload: node_data.payload.clone(),
            configuration: node_data.configuration.clone(),
            created_at: chrono::Utc::now(),
        };

        nodes_to_create.push(node);
    }

    // Create the blueprint with node order
    let blueprint = Blueprint {
        aturi: blueprint_aturi.clone(),
        did: session.did.clone(),
        enabled: true,
        node_order,
        error: None,
        created_at: chrono::Utc::now(),
    };

    // Save the blueprint
    web_context
        .blueprint_storage
        .create_blueprint(&blueprint)
        .await
        .map_err(|e| {
            WebError::Http(HttpError::Unhandled {
                details: format!("Failed to create blueprint: {}", e),
            })
        })?;

    // Save all nodes
    for node in nodes_to_create {
        web_context
            .node_storage
            .upsert_node(&node)
            .await
            .map_err(|e| {
                WebError::Http(HttpError::Unhandled {
                    details: format!("Failed to create node: {}", e),
                })
            })?;
    }

    // Redirect to the dashboard
    Ok(Redirect::to("/dashboard").into_response())
}

/// Handler for showing the blueprint creation form
pub(super) async fn handle_create_blueprint_get(
    State(web_context): State<WebContext>,
    jar: PrivateCookieJar,
) -> Result<impl IntoResponse, WebError> {
    // Get session from cookie to ensure user is authenticated
    let session = get_session_from_cookie(&web_context, &jar).await?;

    Ok(RenderHtml(
        "blueprint_create.html",
        web_context.engine.clone(),
        context! {
            canonical_url => format!("{}/dashboard/blueprints/new", web_context.config.external_base),
            user_did => session.did,
        },
    )
    .into_response())
}

/// Handler for logging out
pub(super) async fn handle_logout(
    State(web_context): State<WebContext>,
    jar: PrivateCookieJar,
) -> Result<impl IntoResponse, WebError> {
    // Remove the authentication cookie
    let mut removal_cookie = Cookie::from(AUTH_COOKIE_NAME);
    removal_cookie.set_domain(web_context.config.external_base.clone());
    removal_cookie.set_path("/");

    let updated_jar = jar.remove(removal_cookie);

    // Create response with Clear-Site-Data header for thorough client-side cleanup
    Ok((
        updated_jar,
        [(
            header::HeaderName::from_static("clear-site-data"),
            header::HeaderValue::from_static(
                r#""cache", "cookies", "storage", "executionContexts", "prefetchCache", "prerenderCache""#
            ),
        )],
        Redirect::to("/"),
    ).into_response())
}

/// Validation result for a single node
#[derive(Debug, serde::Serialize)]
struct NodeValidationResult {
    index: usize,
    node_type: String,
    errors: Vec<String>,
}

/// Response for blueprint validation
#[derive(Debug, serde::Serialize)]
struct BlueprintValidationResponse {
    valid: bool,
    errors: Vec<String>,
    node_errors: Vec<NodeValidationResult>,
}

/// Handler for validating a blueprint before creation
pub(super) async fn handle_validate_blueprint(
    State(web_context): State<WebContext>,
    jar: PrivateCookieJar,
    axum::Json(form): axum::Json<CreateBlueprintForm>,
) -> Result<impl IntoResponse, WebError> {
    // Get session from cookie
    let session = get_session_from_cookie(&web_context, &jar).await?;

    // Check if the user is allowed to create blueprints
    if !is_allowed_identity(&web_context.config.allowed_identities, &session.did) {
        return Ok(axum::Json(BlueprintValidationResponse {
            valid: false,
            errors: vec!["Features are limited to a waitlist at this time".to_string()],
            node_errors: vec![],
        })
        .into_response());
    }

    // Parse the blueprint data
    let blueprint_data: BlueprintFormData = match serde_json::from_str(&form.blueprint_data) {
        Ok(data) => data,
        Err(e) => {
            // Return validation error for malformed JSON
            return Ok(axum::Json(BlueprintValidationResponse {
                valid: false,
                errors: vec![format!("Invalid blueprint data JSON: {}", e)],
                node_errors: vec![],
            })
            .into_response());
        }
    };

    let mut errors = Vec::new();
    let mut node_errors = Vec::new();

    // Check if blueprint has at least one node
    if blueprint_data.nodes.is_empty() {
        errors.push("Blueprint must have at least one node".to_string());
    }

    // Validate each node
    for (index, node_data) in blueprint_data.nodes.iter().enumerate() {
        let mut node_validation_errors = Vec::new();

        // Validate node type
        if let Err(e) = Validator::validate_node_type(&node_data.node_type) {
            node_validation_errors.push(e.to_string());
        }

        // Validate node position (entry nodes must be first)
        if let Err(e) = Validator::validate_node_position(&node_data.node_type, index) {
            node_validation_errors.push(e.to_string());
        }

        // Validate node configuration
        if let Err(e) = Validator::validate_node_config(
            &node_data.node_type,
            &node_data.configuration,
            &session.did,
        ) {
            node_validation_errors.push(e.to_string());
        }

        // Validate node payload structure based on node type
        if let Err(e) = Validator::validate_node_payload(&node_data.node_type, &node_data.payload) {
            node_validation_errors.push(e.to_string());
        }

        if !node_validation_errors.is_empty() {
            node_errors.push(NodeValidationResult {
                index,
                node_type: node_data.node_type.clone(),
                errors: node_validation_errors,
            });
        }
    }

    // Check for blueprint-level constraints
    if !blueprint_data.nodes.is_empty() {
        // Ensure first node is an entry node
        let first_node_type = &blueprint_data.nodes[0].node_type;
        if !is_entry_node_type(first_node_type) {
            errors.push(format!(
                "First node must be an entry node (jetstream_entry, webhook_entry, periodic_entry, or zap_entry), got: {}",
                first_node_type
            ));
        }

        // Check that non-entry nodes don't appear after the first position
        for (index, node_data) in blueprint_data.nodes.iter().enumerate().skip(1) {
            if is_entry_node_type(&node_data.node_type) {
                errors.push(format!(
                    "Entry node '{}' at position {} must be the first node",
                    node_data.node_type,
                    index
                ));
            }
        }
    }

    let valid = errors.is_empty() && node_errors.is_empty();

    Ok(axum::Json(BlueprintValidationResponse {
        valid,
        errors,
        node_errors,
    })
    .into_response())
}

/// Handler for the evaluate node tool page
pub(super) async fn handle_evaluate_node_get(
    State(web_context): State<WebContext>,
    jar: PrivateCookieJar,
) -> Result<impl IntoResponse, WebError> {
    // Get session from cookie to ensure authentication
    let session = get_session_from_cookie(&web_context, &jar).await?;

    // Get list of available node types (excluding disabled ones)
    let all_node_types = vec![
        "condition",
        "transform",
        "debug_action",
        "facet_text",
        "get_record",
        "jetstream_entry",
        "parse_aturi",
        "periodic_entry",
        "publish_record",
        "publish_webhook",
        "sentiment_analysis",
        "webhook_entry",
        "zap_entry",
    ];

    let available_node_types: Vec<String> = all_node_types
        .into_iter()
        .filter(|node_type| !web_context.config.disabled_node_types.contains(*node_type))
        .map(|s| s.to_string())
        .collect();

    Ok(RenderHtml(
        "evaluate_node.html",
        web_context.engine.clone(),
        context! {
            canonical_url => format!("{}/dashboard/evaluate-node", web_context.config.external_base),
            user_did => session.did,
            available_node_types => available_node_types,
        },
    )
    .into_response())
}

/// Handler for viewing evaluation results for a blueprint
pub(super) async fn handle_blueprint_evaluation_results(
    State(web_context): State<WebContext>,
    jar: PrivateCookieJar,
    axum::extract::Path(record_key): axum::extract::Path<String>,
) -> Result<impl IntoResponse, WebError> {
    // Get session from cookie to ensure authentication
    let session = get_session_from_cookie(&web_context, &jar).await?;

    // Construct the blueprint AT-URI from the record key
    let blueprint_aturi = construct_blueprint_aturi(&session.did, &record_key);

    // Get the blueprint to ensure it exists and user owns it
    let blueprint = web_context
        .blueprint_storage
        .get_blueprint(&blueprint_aturi)
        .await
        .map_err(|e| {
            WebError::Http(HttpError::Unhandled {
                details: format!("Failed to fetch blueprint: {}", e),
            })
        })?
        .ok_or_else(|| {
            WebError::Http(HttpError::NotFound {
                details: "Blueprint not found".to_string(),
            })
        })?;

    // Ensure the user owns this blueprint
    if blueprint.did != session.did {
        return Err(WebError::Http(HttpError::Forbidden {
            details: "You don't have permission to view this blueprint's results".to_string(),
        }));
    }

    // Get the last 5 evaluation results for this blueprint
    let evaluation_results = web_context
        .evaluation_result_storage
        .list_by_blueprint(&blueprint_aturi, 5)
        .await
        .map_err(|e| {
            WebError::Http(HttpError::Unhandled {
                details: format!("Failed to fetch evaluation results: {}", e),
            })
        })?;

    // Process results for display
    let processed_results: Vec<_> = evaluation_results
        .into_iter()
        .map(|result| {
            let duration_ms = (result.completed_at - result.queued_at).num_milliseconds();
            let is_success = result.result == "OK";

            // Parse error details if failed
            let (failed_node, error_message) = if !is_success && result.result.starts_with('[') {
                if let Some(end_bracket) = result.result.find(']') {
                    let node = result.result[1..end_bracket].to_string();
                    let error = result.result[end_bracket + 1..].trim().to_string();
                    (Some(node), Some(error))
                } else {
                    (None, Some(result.result.clone()))
                }
            } else if !is_success {
                (None, Some(result.result.clone()))
            } else {
                (None, None)
            };

            serde_json::json!({
                "id": result.id,
                "queued_at": result.queued_at.to_rfc3339(),
                "completed_at": result.completed_at.to_rfc3339(),
                "duration_ms": duration_ms,
                "is_success": is_success,
                "failed_node": failed_node,
                "error_message": error_message,
            })
        })
        .collect();

    Ok(RenderHtml(
        "blueprint_evaluation_results.html",
        web_context.engine.clone(),
        context! {
            canonical_url => format!("{}/dashboard/{}/results", web_context.config.external_base, record_key),
            user_did => session.did,
            blueprint_aturi => blueprint_aturi,
            record_key => record_key,
            evaluation_results => processed_results,
            has_results => !processed_results.is_empty(),
        },
    )
    .into_response())
}

/// Handler for the evaluate blueprint tool page
pub(super) async fn handle_evaluate_blueprint_get(
    State(web_context): State<WebContext>,
    jar: PrivateCookieJar,
) -> Result<impl IntoResponse, WebError> {
    // Get session from cookie to ensure authentication
    let session = get_session_from_cookie(&web_context, &jar).await?;

    // Get blueprints for the user
    let blueprints = web_context
        .blueprint_storage
        .list_blueprints(Some(&session.did))
        .await
        .map_err(|e| {
            WebError::Http(HttpError::Unhandled {
                details: format!("Failed to fetch blueprints: {}", e),
            })
        })?;

    // Extract data for each blueprint
    let mut user_blueprints = Vec::new();
    for bp in blueprints {
        let record_key = extract_record_key(&bp.aturi);
        let nodes = fetch_blueprint_nodes(&web_context, &bp.aturi).await?;
        let is_valid = bp.is_valid(&nodes);
        let node_count = bp.node_order.len();

        // Determine entry type from first node
        let entry_type = if !nodes.is_empty() && !bp.node_order.is_empty() {
            let first_node_key = &bp.node_order[0];
            nodes
                .iter()
                .find(|n| n.aturi.ends_with(first_node_key))
                .map(|n| n.node_type.clone())
                .unwrap_or_else(|| "unknown".to_string())
        } else {
            "unknown".to_string()
        };

        user_blueprints.push((bp, record_key, is_valid, node_count, entry_type));
    }

    Ok(RenderHtml(
        "evaluate_blueprint.html",
        web_context.engine.clone(),
        context! {
            user_blueprints => user_blueprints,
            user_did => session.did,
        },
    )
    .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_validate_blueprint_valid() {
        // This test would require a full WebContext setup
        // For now, we'll just verify the validation logic works

        let valid_blueprint = BlueprintFormData {
            nodes: vec![
                NodeFormData {
                    node_type: "jetstream_entry".to_string(),
                    payload: serde_json::json!({
                        "collection": "app.bsky.feed.post",
                        "operation": "create"
                    }),
                    configuration: serde_json::json!({}),
                },
                NodeFormData {
                    node_type: "condition".to_string(),
                    payload: serde_json::json!({
                        "==": [{"val": ["text"]}, "test"]
                    }),
                    configuration: serde_json::json!({}),
                },
            ],
        };

        // Test that entry node is first
        assert!(is_entry_node_type(&valid_blueprint.nodes[0].node_type));
        assert!(!is_entry_node_type(&valid_blueprint.nodes[1].node_type));
    }

    #[test]
    fn test_is_entry_node() {
        assert!(is_entry_node_type("jetstream_entry"));
        assert!(is_entry_node_type("webhook_entry"));
        assert!(is_entry_node_type("periodic_entry"));
        assert!(is_entry_node_type("zap_entry"));
        assert!(!is_entry_node_type("condition"));
        assert!(!is_entry_node_type("transform"));
        assert!(!is_entry_node_type("publish_record"));
    }

    #[test]
    fn test_validate_node_payload() {
        // Test jetstream_entry validation
        assert!(
            Validator::validate_node_payload(
                "jetstream_entry",
                &serde_json::json!({
                    "collection": "app.bsky.feed.post",
                    "operation": "create"
                })
            )
            .is_ok()
        );

        assert!(
            Validator::validate_node_payload("jetstream_entry", &serde_json::json!("invalid"))
                .is_err()
        );

        // Test publish_record validation - accepts string or object
        assert!(
            Validator::validate_node_payload("publish_record", &serde_json::json!("record"))
                .is_ok()
        );

        assert!(
            Validator::validate_node_payload(
                "publish_record",
                &serde_json::json!({
                    "val": ["record"]
                })
            )
            .is_ok()
        );

        // Rejects non-string, non-object types
        assert!(
            Validator::validate_node_payload("publish_record", &serde_json::json!(true)).is_err()
        );

        // Test webhook validation - payload can be string or object
        assert!(
            Validator::validate_node_payload(
                "publish_webhook_direct",
                &serde_json::json!("webhook_data")
            )
            .is_ok()
        );

        assert!(
            Validator::validate_node_payload(
                "publish_webhook_direct",
                &serde_json::json!({
                    "val": ["webhook", "data"]
                })
            )
            .is_ok()
        );

        // Rejects non-string, non-object types
        assert!(
            Validator::validate_node_payload("publish_webhook_direct", &serde_json::json!(123))
                .is_err()
        );

        // Test condition validation
        assert!(
            Validator::validate_node_payload("condition", &serde_json::json!({"==": [1, 1]}))
                .is_ok()
        );

        assert!(Validator::validate_node_payload("condition", &serde_json::json!(null)).is_err());
    }
}
