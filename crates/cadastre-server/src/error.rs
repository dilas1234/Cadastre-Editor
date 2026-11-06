//! Application error type and its HTTP representation.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

/// Everything a handler can fail with. Converts into an RFC-7807-ish JSON body.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    /// A requested resource does not exist.
    #[error("{0} not found")]
    NotFound(String),

    /// The request was malformed or violated a domain rule.
    #[error("{0}")]
    BadRequest(String),

    /// Authentication is required but missing or unusable.
    #[error("{0}")]
    Unauthorized(String),

    /// The caller lacks the required role.
    #[error("{0}")]
    Forbidden(String),

    /// The request conflicts with current state (e.g. a resource is already locked).
    #[error("{0}")]
    Conflict(String),

    /// The request was well-formed but cannot be fulfilled (e.g. a raster too large to preview).
    #[error("{0}")]
    UnprocessableEntity(String),

    /// A database error. The detail is logged, not returned to the client.
    #[error("database error")]
    Database(#[from] sqlx::Error),

    /// An unexpected server-side failure. The detail is logged; a generic message is returned.
    #[error("{0}")]
    Internal(String),
}

impl AppError {
    fn status(&self) -> StatusCode {
        match self {
            AppError::NotFound(_) => StatusCode::NOT_FOUND,
            AppError::BadRequest(_) => StatusCode::BAD_REQUEST,
            AppError::Unauthorized(_) => StatusCode::UNAUTHORIZED,
            AppError::Forbidden(_) => StatusCode::FORBIDDEN,
            AppError::Conflict(_) => StatusCode::CONFLICT,
            AppError::UnprocessableEntity(_) => StatusCode::UNPROCESSABLE_ENTITY,
            AppError::Database(_) => StatusCode::INTERNAL_SERVER_ERROR,
            AppError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = self.status();
        if let AppError::Database(ref e) = self {
            tracing::error!(error = %e, "database error");
        }
        if let AppError::Internal(ref detail) = self {
            tracing::error!(detail = %detail, "internal error");
        }
        // The internal detail is logged above, never returned to the client.
        let message = match self {
            AppError::Internal(_) => "internal server error".to_string(),
            ref other => other.to_string(),
        };
        let body = Json(json!({
            "status": status.as_u16(),
            "error": status.canonical_reason().unwrap_or("Error"),
            "message": message,
        }));
        (status, body).into_response()
    }
}

/// Convenient handler result alias.
pub type ApiResult<T> = Result<T, AppError>;
