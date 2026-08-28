//! The API error type and how it renders as JSON.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

/// Every failure an API handler can return.
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    /// No credentials, or credentials that no longer resolve.
    #[error("authentication required")]
    Unauthorized,

    /// Authenticated, but not allowed to do this.
    #[error("not allowed")]
    Forbidden,

    /// The addressed resource does not exist.
    #[error("{0} not found")]
    NotFound(String),

    /// The request was understood but is invalid.
    #[error("{0}")]
    BadRequest(String),

    /// A guardian rejected or failed the operation.
    #[error("{0}")]
    Guardian(#[from] guardian::Error),

    /// The optional Playit daemon rejected or could not complete the operation.
    #[error("{0}")]
    Playit(#[from] playit_integration::PlayitError),

    /// Anything unexpected.
    #[error("{0}")]
    Internal(#[from] anyhow::Error),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match &self {
            ApiError::Unauthorized => StatusCode::UNAUTHORIZED,
            ApiError::Forbidden => StatusCode::FORBIDDEN,
            ApiError::NotFound(_) => StatusCode::NOT_FOUND,
            ApiError::BadRequest(_) => StatusCode::BAD_REQUEST,
            // An invalid transition is the client's fault, not the server's.
            ApiError::Guardian(guardian::Error::InvalidTransition { .. }) => StatusCode::CONFLICT,
            ApiError::Playit(_) => StatusCode::SERVICE_UNAVAILABLE,
            ApiError::Guardian(_) | ApiError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };

        if status == StatusCode::INTERNAL_SERVER_ERROR {
            tracing::error!(error = %self, "request failed");
        }

        (status, Json(json!({ "error": self.to_string() }))).into_response()
    }
}

/// Handler result alias.
pub type ApiResult<T> = Result<T, ApiError>;
