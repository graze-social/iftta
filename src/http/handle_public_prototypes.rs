//! Public prototype directory handlers.

use axum::{
    Json,
    extract::{Query, State},
    response::{Html, IntoResponse},
};
use axum_extra::extract::PrivateCookieJar;
use axum_template::TemplateEngine;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::{
    errors::HttpError,
    http::{auth::get_session_from_cookie, context::WebContext, errors::WebError},
};

/// Query parameters for the public prototype directory
#[derive(Debug, Deserialize)]
pub struct PrototypesQuery {
    #[serde(default = "default_page")]
    page: u32,
    #[serde(default = "default_page_size")]
    size: u32,
}

fn default_page() -> u32 {
    1
}

fn default_page_size() -> u32 {
    20
}

/// Template context for the public prototype directory
#[derive(Serialize)]
struct PublicPrototypesContext {
    prototypes: Vec<PrototypeDisplay>,
    current_page: u32,
    total_pages: u32,
    has_next: bool,
    has_prev: bool,
    is_authenticated: bool,
    user_did: Option<String>,
}

/// Display model for prototypes in the public directory
#[derive(Serialize)]
struct PrototypeDisplay {
    aturi: String,
    name: String,
    description: String,
    owner_did: String,
    owner_handle: String,
    placeholder_count: usize,
    created_at: String,
}

/// Handle GET requests to the public prototype directory
pub(super) async fn handle_public_prototypes(
    State(web_context): State<WebContext>,
    jar: PrivateCookieJar,
    Query(query): Query<PrototypesQuery>,
) -> Result<impl IntoResponse, WebError> {
    // Check if user is authenticated (optional for this page)
    let session = get_session_from_cookie(&web_context, &jar).await.ok();
    let is_authenticated = session.is_some();
    let user_did = session.map(|s| s.did);

    // Calculate pagination
    let limit = query.size as i64;
    let offset = ((query.page - 1) * query.size) as i64;

    // Fetch shared prototypes
    let prototypes = web_context
        .prototype_storage
        .list_shared_prototypes(Some(limit + 1), Some(offset))
        .await
        .map_err(|e| {
            WebError::Http(HttpError::Unhandled {
                details: format!("Failed to fetch shared prototypes: {}", e),
            })
        })?;

    // Check if there are more pages
    let has_more = prototypes.len() > query.size as usize;
    let mut display_prototypes: Vec<PrototypeDisplay> = Vec::new();

    for p in prototypes.into_iter().take(query.size as usize) {
        // Try to resolve the DID to get the handle
        let owner_handle = match web_context.identity_resolver.resolve(&p.did).await {
            Ok(doc) => {
                // Extract handle from also_known_as field
                // The also_known_as field typically contains AT-URI formatted handles
                doc.also_known_as
                    .iter()
                    .find_map(|aka| {
                        if aka.starts_with("at://") {
                            aka.strip_prefix("at://").map(|s| s.to_string())
                        } else {
                            None
                        }
                    })
                    .unwrap_or_else(|| {
                        format!("did:{}", p.did.split(':').last().unwrap_or("unknown"))
                    })
            }
            Err(_) => {
                // Fallback to showing part of the DID if resolution fails
                format!("did:{}", p.did.split(':').last().unwrap_or("unknown"))
            }
        };

        display_prototypes.push(PrototypeDisplay {
            aturi: p.aturi,
            name: p.name,
            description: p.description,
            owner_did: p.did,
            owner_handle,
            placeholder_count: p.placeholders.len(),
            created_at: p.created_at.format("%Y-%m-%d").to_string(),
        });
    }

    // For now, assume we don't know the total count (could add a count query)
    let context = PublicPrototypesContext {
        prototypes: display_prototypes,
        current_page: query.page,
        total_pages: 0, // Would need a count query to determine this
        has_next: has_more,
        has_prev: query.page > 1,
        is_authenticated,
        user_did,
    };

    // Render the template
    let html = web_context
        .engine
        .render("public_prototypes.html", &context)
        .map_err(|e| {
            WebError::Http(HttpError::Unhandled {
                details: format!("Template error: {}", e),
            })
        })?;

    Ok(Html(html).into_response())
}

/// Query parameters for viewing a prototype
#[derive(Debug, Deserialize)]
pub struct ViewPrototypeQuery {
    aturi: String,
}

/// Handle GET requests to view a specific prototype
pub(super) async fn handle_view_prototype(
    State(web_context): State<WebContext>,
    jar: PrivateCookieJar,
    Query(query): Query<ViewPrototypeQuery>,
) -> Result<impl IntoResponse, WebError> {
    let aturi = query.aturi;

    // Fetch the prototype
    let prototype = web_context
        .prototype_storage
        .get_prototype(&aturi)
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

    // Check if the prototype is shared or if the user owns it
    let session = get_session_from_cookie(&web_context, &jar).await.ok();
    let is_authenticated = session.is_some();
    let user_did = session.as_ref().map(|s| s.did.clone());
    let is_owner = user_did.as_ref().map_or(false, |did| did == &prototype.did);

    // If prototype is not shared and user is not the owner, deny access
    if !prototype.shared && !is_owner {
        return Err(WebError::Http(HttpError::Unhandled {
            details: format!("Access denied: This prototype is private"),
        }));
    }

    // Try to resolve the DID to get the handle
    let owner_handle = match web_context.identity_resolver.resolve(&prototype.did).await {
        Ok(doc) => {
            // Extract handle from also_known_as field
            doc.also_known_as
                .iter()
                .find_map(|aka| {
                    if aka.starts_with("at://") {
                        aka.strip_prefix("at://").map(|s| s.to_string())
                    } else {
                        None
                    }
                })
                .unwrap_or_else(|| {
                    format!(
                        "did:{}",
                        prototype.did.split(':').last().unwrap_or("unknown")
                    )
                })
        }
        Err(_) => {
            // Fallback to showing part of the DID if resolution fails
            format!(
                "did:{}",
                prototype.did.split(':').last().unwrap_or("unknown")
            )
        }
    };

    // Prepare context for template
    let context = serde_json::json!({
        "prototype": {
            "aturi": prototype.aturi,
            "name": prototype.name,
            "description": prototype.description,
            "owner_did": prototype.did,
            "owner_handle": owner_handle,
            "placeholders": prototype.placeholders,
            "nodes": prototype.nodes,
            "shared": prototype.shared,
            "created_at": prototype.created_at.format("%Y-%m-%d").to_string(),
            "updated_at": prototype.updated_at.format("%Y-%m-%d").to_string(),
        },
        "is_authenticated": is_authenticated,
        "user_did": user_did,
        "is_owner": is_owner,
        "can_instantiate": is_authenticated,
    });

    // Render the template
    let html = web_context
        .engine
        .render("view_prototype.html", &context)
        .map_err(|e| {
            WebError::Http(HttpError::Unhandled {
                details: format!("Template error: {}", e),
            })
        })?;

    Ok(Html(html).into_response())
}

/// Handle POST requests to instantiate a prototype (requires authentication)
pub(super) async fn handle_instantiate_shared_prototype(
    State(web_context): State<WebContext>,
    jar: PrivateCookieJar,
    Json(request): Json<InstantiateRequest>,
) -> Result<impl IntoResponse, WebError> {
    // Require authentication
    let session = get_session_from_cookie(&web_context, &jar).await?;

    // Fetch the prototype
    let prototype = web_context
        .prototype_storage
        .get_prototype(&request.prototype_aturi)
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

    // Check if the prototype is shared or if the user owns it
    if !prototype.shared && prototype.did != session.did {
        return Err(WebError::Http(HttpError::Unhandled {
            details: format!("Access denied: This prototype is private"),
        }));
    }

    // Create prototype service and instantiate
    let prototype_service = crate::prototype_service::PrototypeService::new(
        web_context.prototype_storage.clone(),
        web_context.blueprint_storage.clone(),
        web_context.node_storage.clone(),
    );

    let blueprint_aturi = prototype_service
        .instantiate_blueprint(
            &request.prototype_aturi,
            &session.did,
            request.placeholder_values,
        )
        .await
        .map_err(|e| {
            WebError::Http(HttpError::Unhandled {
                details: format!("Failed to instantiate prototype: {}", e),
            })
        })?;

    // Return the new blueprint AT-URI
    Ok(Json(serde_json::json!({
        "success": true,
        "blueprint_aturi": blueprint_aturi,
    }))
    .into_response())
}

#[derive(Debug, Deserialize)]
pub struct InstantiateRequest {
    prototype_aturi: String,
    placeholder_values: HashMap<String, String>,
}
