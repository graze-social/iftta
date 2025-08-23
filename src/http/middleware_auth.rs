use axum::{
    extract::{Request, State},
    middleware::Next,
    response::{IntoResponse, Redirect, Response},
};
use axum_extra::extract::PrivateCookieJar;

use crate::{
    errors::HttpError,
    http::{context::WebContext, errors::WebError, handle_auth::AUTH_COOKIE_NAME},
};

/// Middleware to check if a user is authenticated
pub(super) async fn require_auth(
    State(web_context): State<WebContext>,
    jar: PrivateCookieJar,
    request: Request,
    next: Next,
) -> Result<Response, WebError> {
    // Get session ID from cookie
    let cookie = jar.get(AUTH_COOKIE_NAME);

    if let Some(cookie) = cookie {
        let session_id = cookie.value();

        // Get session from storage
        let session = web_context
            .session_storage
            .get_session(session_id)
            .await
            .map_err(|e| {
                WebError::Http(HttpError::Unhandled {
                    details: format!("Failed to retrieve session: {}", e),
                })
            })?;

        if session.is_some() {
            // Session is valid, proceed with the request
            return Ok(next.run(request).await);
        }
    }

    // No valid session, redirect to login
    Ok(Redirect::to("/login").into_response())
}
