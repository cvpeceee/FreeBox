//! HTTP error responses.
//!
//! All handler errors are converted to JSON responses via the [`AppError`] type.
//! This guarantees a consistent error shape across the entire API:
//!
//! ```json
//! { "error": "Not Found", "message": "file abc123 does not exist" }
//! ```

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use thiserror::Error;

/// Application-level error type returned by all handlers.
#[derive(Debug, Error)]
pub enum AppError {
    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Unauthorized: {0}")]
    Unauthorized(String),

    #[error("Forbidden: {0}")]
    Forbidden(String),

    #[error("Bad request: {0}")]
    BadRequest(String),

    #[error("Conflict: {0}")]
    Conflict(String),

    /// Catch-all for unexpected errors — details are logged but NOT returned
    /// to the client (to avoid information leakage).
    #[error("Internal server error")]
    Internal(#[from] anyhow::Error),
}

/// Convert an [`AppError`] into an Axum HTTP response with a JSON body.
impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, error_title) = match &self {
            AppError::NotFound(_) => (StatusCode::NOT_FOUND, "Not Found"),
            AppError::Unauthorized(_) => (StatusCode::UNAUTHORIZED, "Unauthorized"),
            AppError::Forbidden(_) => (StatusCode::FORBIDDEN, "Forbidden"),
            AppError::BadRequest(_) => (StatusCode::BAD_REQUEST, "Bad Request"),
            AppError::Conflict(_) => (StatusCode::CONFLICT, "Conflict"),
            AppError::Internal(e) => {
                // Log the full error chain internally; return a generic message.
                tracing::error!(error = %e, "Internal server error");
                (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error")
            }
        };

        // Include the human-readable message for all non-500 errors.
        let message = match &self {
            AppError::Internal(_) => "An unexpected error occurred".to_string(),
            other => other.to_string(),
        };

        let body = Json(json!({ "error": error_title, "message": message }));
        (status, body).into_response()
    }
}

/// Convenience alias — handlers return `Result<T, AppError>`.
pub type Result<T> = std::result::Result<T, AppError>;

// Allow easy conversion from freebox_core errors.
impl From<freebox_core::Error> for AppError {
    fn from(e: freebox_core::Error) -> Self {
        match e {
            freebox_core::Error::NotFound { key } => AppError::NotFound(key),
            freebox_core::Error::Unauthenticated => {
                AppError::Unauthorized("invalid credentials".into())
            }
            freebox_core::Error::Unauthorized { reason } => AppError::Forbidden(reason),
            freebox_core::Error::TokenExpired => AppError::Unauthorized("token expired".into()),
            other => AppError::Internal(anyhow::anyhow!("{}", other)),
        }
    }
}
