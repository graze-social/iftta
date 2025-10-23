//! HTTP handlers for prototype management

use anyhow::Result;
use axum::{
    Form, Json,
    extract::{Query, State},
    response::{IntoResponse, Redirect},
};
use axum_extra::extract::PrivateCookieJar;
use axum_template::RenderHtml;
use chrono::Utc;
use minijinja::context;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use ulid::Ulid;

use crate::{
    errors::HttpError,
    http::{
        auth::{get_session_from_cookie, is_allowed_identity},
        context::WebContext,
        errors::WebError,
    },
    prototype_service::PrototypeService,
    storage::prototype::{PlaceholderDef, Prototype},
    validation::Validator,
};

#[derive(Debug, Deserialize)]
pub struct PrototypesQuery {
    page: Option<u32>,
    #[serde(default = "default_page_size")]
    size: u32,
}

fn default_page_size() -> u32 {
    20
}

/// Handle GET requests to show the prototypes page
pub(super) async fn handle_prototypes_get(
    State(web_context): State<WebContext>,
    jar: PrivateCookieJar,
    Query(query): Query<PrototypesQuery>,
) -> Result<impl IntoResponse, WebError> {
    // Get session from cookie
    let session = get_session_from_cookie(&web_context, &jar).await?;

    let page = query.page.unwrap_or(1).max(1);
    let size = query.size.min(100).max(1);
    let offset = (page - 1) * size;

    // Get prototypes for the user
    let prototypes = web_context
        .prototype_storage
        .list_prototypes(Some(&session.did))
        .await
        .map_err(|e| {
            WebError::Http(HttpError::Unhandled {
                details: format!("Failed to fetch prototypes: {}", e),
            })
        })?;

    // Paginate in memory (could be optimized with database pagination)
    let total_prototypes = prototypes.len();
    let paginated_prototypes: Vec<_> = prototypes
        .into_iter()
        .skip(offset as usize)
        .take(size as usize)
        .collect();

    // Process prototypes to add additional display data
    let mut prototypes_with_data = Vec::new();
    for prototype in paginated_prototypes {
        let placeholder_count = prototype.placeholders.len();
        let record_key = prototype.aturi.rsplit('/').next().unwrap_or("").to_string();

        prototypes_with_data.push((prototype, record_key, placeholder_count));
    }

    // Calculate pagination info
    let total_pages = ((total_prototypes as f64) / (size as f64)).ceil() as u32;
    let has_previous = page > 1;
    let has_next = page < total_pages;
    let previous_page = if has_previous { Some(page - 1) } else { None };
    let next_page = if has_next { Some(page + 1) } else { None };

    Ok(RenderHtml(
        "prototypes.html",
        web_context.engine.clone(),
        context! {
            canonical_url => format!("{}/dashboard/prototypes", web_context.config.external_base),
            user_did => session.did,
            prototypes => prototypes_with_data,
            // Pagination
            current_page => page,
            page_size => size,
            total_prototypes => total_prototypes,
            total_pages => total_pages,
            has_previous => has_previous,
            has_next => has_next,
            previous_page => previous_page,
            next_page => next_page,
        },
    )
    .into_response())
}

/// Handle GET requests to show the prototype builder page
pub(super) async fn handle_prototype_builder_get(
    State(web_context): State<WebContext>,
    jar: PrivateCookieJar,
) -> Result<impl IntoResponse, WebError> {
    // Get session from cookie to ensure user is authenticated
    let session = get_session_from_cookie(&web_context, &jar).await?;

    Ok(RenderHtml(
        "prototype_builder.html",
        web_context.engine.clone(),
        context! {
            canonical_url => format!("{}/dashboard/prototypes/builder", web_context.config.external_base),
            user_did => session.did,
        },
    )
    .into_response())
}

/// Form data for creating a prototype
#[derive(Debug, Deserialize)]
pub struct CreatePrototypeForm {
    name: String,
    description: String,
    nodes: String,        // JSON string
    placeholders: String, // JSON string
    #[serde(default)]
    shared: bool,
}

/// Handle POST requests to create a prototype
pub(super) async fn handle_create_prototype(
    State(web_context): State<WebContext>,
    jar: PrivateCookieJar,
    Form(form): Form<CreatePrototypeForm>,
) -> Result<impl IntoResponse, WebError> {
    // Get session from cookie
    let session = get_session_from_cookie(&web_context, &jar).await?;

    // Check if the user is allowed to create prototypes
    if !is_allowed_identity(&web_context.config.allowed_identities, &session.did) {
        return Err(WebError::Http(HttpError::Forbidden {
            details: "Features are limited to a waitlist at this time".to_string(),
        }));
    }

    // Parse nodes JSON
    let nodes: serde_json::Value = serde_json::from_str(&form.nodes).map_err(|e| {
        WebError::Http(HttpError::Unhandled {
            details: format!("Invalid nodes JSON: {}", e),
        })
    })?;

    // Parse placeholders JSON
    let placeholders: Vec<PlaceholderDef> =
        serde_json::from_str(&form.placeholders).map_err(|e| {
            WebError::Http(HttpError::Unhandled {
                details: format!("Invalid placeholders JSON: {}", e),
            })
        })?;

    // Generate prototype AT-URI
    let prototype_id = Ulid::new().to_string();
    let prototype_aturi = format!(
        "at://{}/tools.graze.ifthisthenat.prototype/{}",
        session.did, prototype_id
    );

    // Create the prototype
    let prototype = Prototype {
        aturi: prototype_aturi.clone(),
        did: session.did.clone(),
        name: form.name,
        description: form.description,
        nodes,
        placeholders,
        shared: form.shared,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };

    // Validate and create the prototype
    let prototype_service = PrototypeService::new(
        web_context.prototype_storage.clone(),
        web_context.blueprint_storage.clone(),
        web_context.node_storage.clone(),
    );

    prototype_service
        .create_prototype(&prototype)
        .await
        .map_err(|e| {
            WebError::Http(HttpError::Unhandled {
                details: format!("Failed to create prototype: {}", e),
            })
        })?;

    // Redirect to the prototypes list
    Ok(Redirect::to("/dashboard/prototypes").into_response())
}

/// Validation request for a prototype
#[derive(Debug, Deserialize)]
pub struct ValidatePrototypeRequest {
    nodes: serde_json::Value,
    placeholders: Vec<PlaceholderDef>,
}

/// Validation response for a prototype
#[derive(Debug, Serialize)]
pub struct ValidatePrototypeResponse {
    valid: bool,
    errors: Vec<String>,
    warnings: Vec<String>,
}

/// Handle POST requests to validate a prototype
pub(super) async fn handle_validate_prototype(
    State(web_context): State<WebContext>,
    jar: PrivateCookieJar,
    Json(request): Json<ValidatePrototypeRequest>,
) -> Result<impl IntoResponse, WebError> {
    // Get session from cookie
    let session = get_session_from_cookie(&web_context, &jar).await?;

    // Check if the user is allowed to validate prototypes
    if !is_allowed_identity(&web_context.config.allowed_identities, &session.did) {
        return Ok(Json(ValidatePrototypeResponse {
            valid: false,
            errors: vec!["Features are limited to a waitlist at this time".to_string()],
            warnings: vec![],
        })
        .into_response());
    }

    let mut errors = Vec::new();
    let mut warnings = Vec::new();

    // Create a temporary prototype for validation
    let temp_prototype = Prototype {
        aturi: "temp".to_string(),
        did: session.did.clone(),
        name: "temp".to_string(),
        description: "temp".to_string(),
        nodes: request.nodes.clone(),
        placeholders: request.placeholders,
        shared: false,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };

    // Validate the prototype structure
    let validation = temp_prototype.validate();

    // Add validation errors
    errors.extend(validation.errors);

    // Add undefined placeholder errors
    if !validation.undefined_placeholders.is_empty() {
        let undefined_list = validation
            .undefined_placeholders
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
        errors.push(format!(
            "Undefined placeholders referenced in nodes: {}",
            undefined_list
        ));
    }

    // Add unused placeholder warnings
    if !validation.unused_placeholders.is_empty() {
        let unused_list = validation
            .unused_placeholders
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
        warnings.push(format!(
            "Defined placeholders that are never used: {}",
            unused_list
        ));
    }

    // Validate blueprint structure (nodes array)
    if let Some(nodes_array) = request.nodes.as_array() {
        // Must have at least one node
        if nodes_array.is_empty() {
            errors.push("Prototype must have at least one node".to_string());
        }

        // Validate each node structure
        for (index, node) in nodes_array.iter().enumerate() {
            if let Some(node_obj) = node.as_object() {
                // Check for required node_type field
                if let Some(node_type) = node_obj.get("node_type").and_then(|v| v.as_str()) {
                    // Validate node type
                    if let Err(e) = Validator::validate_node_type(node_type) {
                        errors.push(format!("Node {}: {}", index, e));
                    }

                    // Validate node position (entry nodes must be first)
                    if let Err(e) = Validator::validate_node_position(node_type, index) {
                        errors.push(format!("Node {}: {}", index, e));
                    }

                    // Note: We can't fully validate configuration and payload here
                    // because they may contain placeholders that haven't been replaced yet
                } else {
                    errors.push(format!("Node {} missing required 'node_type' field", index));
                }
            } else {
                errors.push(format!("Node {} must be an object", index));
            }
        }
    } else {
        errors.push("Nodes must be an array".to_string());
    }

    let valid = errors.is_empty();

    Ok(Json(ValidatePrototypeResponse {
        valid,
        errors,
        warnings,
    })
    .into_response())
}

/// Form data for instantiating a prototype
#[derive(Debug, Deserialize)]
pub struct InstantiatePrototypeForm {
    prototype_aturi: String,
    #[serde(default)]
    placeholder_values: HashMap<String, String>,
}

/// Handle POST requests to instantiate a prototype into a blueprint
pub(super) async fn handle_instantiate_prototype(
    State(web_context): State<WebContext>,
    jar: PrivateCookieJar,
    Form(form): Form<InstantiatePrototypeForm>,
) -> Result<impl IntoResponse, WebError> {
    // Get session from cookie
    let session = get_session_from_cookie(&web_context, &jar).await?;

    // Check if the user is allowed to instantiate prototypes
    if !is_allowed_identity(&web_context.config.allowed_identities, &session.did) {
        return Err(WebError::Http(HttpError::Forbidden {
            details: "Features are limited to a waitlist at this time".to_string(),
        }));
    }

    // Create prototype service
    let prototype_service = PrototypeService::new(
        web_context.prototype_storage.clone(),
        web_context.blueprint_storage.clone(),
        web_context.node_storage.clone(),
    );

    // Instantiate the blueprint
    let blueprint_aturi = prototype_service
        .instantiate_blueprint(&form.prototype_aturi, &session.did, form.placeholder_values)
        .await
        .map_err(|e| {
            WebError::Http(HttpError::Unhandled {
                details: format!("Failed to instantiate prototype: {}", e),
            })
        })?;

    tracing::info!(
        prototype = form.prototype_aturi,
        blueprint = blueprint_aturi,
        user = session.did,
        "Prototype instantiated successfully"
    );

    // Redirect to the dashboard to see the new blueprint
    Ok(Redirect::to("/dashboard").into_response())
}

/// Form data for toggling prototype shared status
#[derive(Debug, Deserialize)]
pub struct ToggleSharedForm {
    prototype_aturi: String,
    shared: bool,
}

/// Handle POST requests to toggle prototype shared status
pub(super) async fn handle_toggle_prototype_shared(
    State(web_context): State<WebContext>,
    jar: PrivateCookieJar,
    Form(form): Form<ToggleSharedForm>,
) -> Result<impl IntoResponse, WebError> {
    // Get session from cookie
    let session = get_session_from_cookie(&web_context, &jar).await?;

    // Check if the user is allowed to toggle prototype sharing
    if !is_allowed_identity(&web_context.config.allowed_identities, &session.did) {
        return Err(WebError::Http(HttpError::Forbidden {
            details: "Features are limited to a waitlist at this time".to_string(),
        }));
    }

    // Fetch the prototype to verify ownership
    let prototype = web_context
        .prototype_storage
        .get_prototype(&form.prototype_aturi)
        .await
        .map_err(|e| {
            WebError::Http(HttpError::Unhandled {
                details: format!("Failed to fetch prototype: {}", e),
            })
        })?
        .ok_or_else(|| {
            WebError::Http(HttpError::Unhandled {
                details: format!("Prototype not found"),
            })
        })?;

    // Verify ownership
    if prototype.did != session.did {
        return Err(WebError::Http(HttpError::Unhandled {
            details: format!("You don't have permission to modify this prototype"),
        }));
    }

    // Update the prototype with new shared status
    let mut updated_prototype = prototype;
    updated_prototype.shared = form.shared;
    updated_prototype.updated_at = chrono::Utc::now();

    // Save the update
    web_context
        .prototype_storage
        .update_prototype(&updated_prototype)
        .await
        .map_err(|e| {
            WebError::Http(HttpError::Unhandled {
                details: format!("Failed to update prototype: {}", e),
            })
        })?;

    tracing::info!(
        prototype = form.prototype_aturi,
        shared = form.shared,
        user = session.did,
        "Prototype sharing status updated"
    );

    // Return JSON response for AJAX calls
    Ok(Json(serde_json::json!({
        "success": true,
        "shared": form.shared,
    }))
    .into_response())
}
