//! Documentation page handler

use axum::{extract::State, response::IntoResponse};
use axum_extra::extract::PrivateCookieJar;
use axum_template::RenderHtml;
use minijinja::context;

use crate::http::{context::WebContext, handle_auth::AUTH_COOKIE_NAME};

/// Handle GET requests to show the documentation page
pub(super) async fn handle_documentation(
    State(web_context): State<WebContext>,
    jar: PrivateCookieJar,
) -> impl IntoResponse {
    // Check if user is logged in
    let user_did = if let Some(cookie) = jar.get(AUTH_COOKIE_NAME) {
        let session_id = cookie.value();
        // Try to get session from storage
        web_context
            .session_storage
            .get_session(session_id)
            .await
            .ok()
            .flatten()
            .map(|session| session.did)
    } else {
        None
    };

    RenderHtml(
        "documentation.html",
        web_context.engine.clone(),
        context! {
            canonical_url => format!("{}/documentation", web_context.config.external_base),
            user_did,
        },
    )
    .into_response()
}
