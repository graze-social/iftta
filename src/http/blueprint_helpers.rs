//! Helper functions for blueprint and node operations to reduce code duplication.

use crate::{
    errors::HttpError,
    http::{context::WebContext, errors::WebError},
    storage::{blueprint::Blueprint, node::Node},
};
use anyhow::Result;

/// Extract the record key from an AT-URI (the last segment after '/')
pub(super) fn extract_record_key(aturi: &str) -> String {
    aturi.rsplit('/').next().unwrap_or("").to_string()
}

/// Construct a blueprint AT-URI from a DID and record key
pub(super) fn construct_blueprint_aturi(did: &str, record_key: &str) -> String {
    format!(
        "at://{}/tools.graze.ifthisthenat.blueprint/{}",
        did,
        record_key
    )
}

/// Construct a node AT-URI from a DID and record key
pub(super) fn construct_node_aturi(did: &str, record_key: &str) -> String {
    format!("at://{}/tools.graze.ifthisthenat.node/{}", did, record_key)
}

/// Fetch a blueprint and verify ownership
pub(super) async fn fetch_blueprint_with_ownership(
    web_context: &WebContext,
    aturi: &str,
    user_did: &str,
) -> Result<Blueprint, WebError> {
    let blueprint = web_context
        .blueprint_storage
        .get_blueprint(aturi)
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

    // Check ownership
    if blueprint.did != user_did {
        return Err(WebError::Http(HttpError::Forbidden {
            details: "You don't have permission to access this blueprint".to_string(),
        }));
    }

    Ok(blueprint)
}

/// Fetch nodes for a blueprint
pub(super) async fn fetch_blueprint_nodes(
    web_context: &WebContext,
    blueprint_aturi: &str,
) -> Result<Vec<Node>, WebError> {
    web_context
        .node_storage
        .list_nodes(blueprint_aturi)
        .await
        .map_err(|e| {
            WebError::Http(HttpError::Unhandled {
                details: format!("Failed to fetch nodes: {}", e),
            })
        })
}

/// Delete all nodes associated with a blueprint
pub(super) async fn delete_blueprint_nodes(
    web_context: &WebContext,
    blueprint_aturi: &str,
) -> Result<(), WebError> {
    let nodes = fetch_blueprint_nodes(web_context, blueprint_aturi).await?;

    for node in nodes {
        web_context
            .node_storage
            .delete_node(&node.aturi)
            .await
            .map_err(|e| {
                WebError::Http(HttpError::Unhandled {
                    details: format!("Failed to delete node: {}", e),
                })
            })?;
    }

    Ok(())
}

/// Parse JSON with proper error handling
pub(super) fn parse_json_with_error<T>(json_str: &str, field_name: &str) -> Result<T, WebError>
where
    T: serde::de::DeserializeOwned,
{
    serde_json::from_str(json_str).map_err(|e| {
        WebError::Http(HttpError::BadRequest {
            details: format!("Invalid {} JSON: {}", field_name, e),
        })
    })
}

/// Parse optional JSON with proper error handling (returns empty object if empty string)
pub(super) fn parse_optional_json(
    json_str: &str,
    field_name: &str,
) -> Result<serde_json::Value, WebError> {
    if json_str.is_empty() {
        Ok(serde_json::json!({}))
    } else {
        parse_json_with_error(json_str, field_name)
    }
}
