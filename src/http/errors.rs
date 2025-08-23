use crate::errors::HttpError;
use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};

#[derive(Debug, thiserror::Error)]
pub(super) enum WebError {
    #[error("HTTP error: {0}")]
    Http(HttpError),
}

impl IntoResponse for WebError {
    fn into_response(self) -> Response {
        match self {
            WebError::Http(err) => match err {
                HttpError::Unauthorized { details } => {
                    (StatusCode::UNAUTHORIZED, details).into_response()
                }
                HttpError::RequestValidation { details } => {
                    (StatusCode::BAD_REQUEST, details).into_response()
                }
                HttpError::Unhandled { details } => {
                    tracing::error!(details = ?details, "Unhandled error");
                    (StatusCode::INTERNAL_SERVER_ERROR, "Internal server error").into_response()
                }
                _ => (StatusCode::INTERNAL_SERVER_ERROR, "Internal server error").into_response(),
            },
        }
    }
}
