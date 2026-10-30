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

    /// A database error. The detail is logged, not returned to the client.
    #[error("database error")]
    Database(#[from] sqlx::Error),
}

impl AppError {
    fn status(&self) -> StatusCode {
        match self {
            AppError::NotFound(_) => StatusCode::NOT_FOUND,
            AppError::BadRequest(_) => StatusCode::BAD_REQUEST,
            AppError::Unauthorized(_) => StatusCode::UNAUTHORIZED,
            AppError::Forbidden(_) => StatusCode::FORBIDDEN,
            AppError::Database(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = self.status();
        if let AppError::Database(ref e) = self {
            tracing::error!(error = %e, "database error");
        }
        let body = Json(json!({
            "status": status.as_u16(),
            "error": status.canonical_reason().unwrap_or("Error"),
            "message": self.to_string(),
        }));
        (status, body).into_response()
    }
}

/// Convenient handler result alias.
pub type ApiResult<T> = Result<T, AppError>;
