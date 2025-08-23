use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Json},
};

use crate::http::WebContext;

pub(super) async fn handle_wellknown_did(State(context): State<WebContext>) -> impl IntoResponse {
    (StatusCode::OK, Json(context.service_document.clone()))
}

pub(super) async fn handle_wellknown_atproto_did(
    State(context): State<WebContext>,
) -> impl IntoResponse {
    let did = context.config.issuer_did.as_str();
    (StatusCode::OK, did.to_string())
}
