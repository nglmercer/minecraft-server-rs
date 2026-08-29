//! The API error type and how it renders as JSON.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use playit_integration::{PlayitError, ServiceErrorCode};
use serde_json::json;
use uuid::Uuid;

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

    /// The request conflicts with the current panel state.
    #[error("{0}")]
    Conflict(String),

    /// The caller is temporarily rate limited.
    #[error("too many requests")]
    TooManyRequests,

    /// A guardian rejected or failed the operation.
    #[error("{0}")]
    Guardian(#[from] guardian::Error),

    /// The Playit service rejected or could not complete the operation.
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
            ApiError::Conflict(_) => StatusCode::CONFLICT,
            ApiError::TooManyRequests => StatusCode::TOO_MANY_REQUESTS,
            // An invalid transition is the client's fault, not the server's.
            ApiError::Guardian(guardian::Error::InvalidTransition { .. }) => StatusCode::CONFLICT,
            ApiError::Playit(error) => playit_status(error),
            ApiError::Guardian(guardian::Error::InvalidConfiguration(_))
            | ApiError::Guardian(guardian::Error::InvalidCommand(_))
            | ApiError::Guardian(guardian::Error::EulaNotAccepted)
            | ApiError::Guardian(guardian::Error::InvalidBackupId(_))
            | ApiError::Guardian(guardian::Error::UnsafeArchiveEntry(_))
            | ApiError::Guardian(guardian::Error::ArchiveLimit(_)) => StatusCode::BAD_REQUEST,
            ApiError::Guardian(guardian::Error::StartCancelled)
            | ApiError::Guardian(guardian::Error::ConsoleUnavailable) => StatusCode::CONFLICT,
            ApiError::Guardian(_) | ApiError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };

        if status == StatusCode::INTERNAL_SERVER_ERROR {
            let request_id = Uuid::new_v4().simple().to_string();
            tracing::error!(request_id = %request_id, error = ?self, "request failed");
            return (
                status,
                Json(json!({
                    "error": "internal server error",
                    "request_id": request_id,
                })),
            )
                .into_response();
        }

        let message = match &self {
            ApiError::Playit(_) if status.is_server_error() => {
                "external service unavailable".into()
            }
            ApiError::Playit(_) => "external service request failed".into(),
            _ => self.to_string(),
        };
        (status, Json(json!({ "error": message }))).into_response()
    }
}

/// Handler result alias.
pub type ApiResult<T> = Result<T, ApiError>;

fn playit_status(error: &PlayitError) -> StatusCode {
    match error.service_code() {
        Some(ServiceErrorCode::InvalidTunnelRequest)
        | Some(ServiceErrorCode::InvalidRequest)
        | Some(ServiceErrorCode::InvalidRequestType) => StatusCode::BAD_REQUEST,
        Some(ServiceErrorCode::PermissionDenied) => StatusCode::FORBIDDEN,
        Some(ServiceErrorCode::TunnelNotFound) => StatusCode::NOT_FOUND,
        Some(ServiceErrorCode::UnsupportedProtocol) => StatusCode::NOT_IMPLEMENTED,
        Some(ServiceErrorCode::ApiRejected) => StatusCode::BAD_GATEWAY,
        Some(ServiceErrorCode::ApiUnavailable)
        | Some(ServiceErrorCode::ProvisioningUnavailable)
        | Some(ServiceErrorCode::InvalidSecret)
        | Some(ServiceErrorCode::SecretPinned)
        | Some(ServiceErrorCode::SecretWriteFailed)
        | Some(ServiceErrorCode::AgentDisabledOverLimit) => StatusCode::SERVICE_UNAVAILABLE,
        Some(ServiceErrorCode::Internal) | None if error.is_unavailable() => {
            StatusCode::SERVICE_UNAVAILABLE
        }
        Some(ServiceErrorCode::Internal) | None => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;

    #[tokio::test]
    async fn unexpected_errors_return_a_safe_message_and_request_id() {
        let response = ApiError::Internal(anyhow::anyhow!(
            "failed to open C:\\private\\panel.json: permission denied"
        ))
        .into_response();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = to_bytes(response.into_body(), 4096).await.unwrap();
        let text = String::from_utf8(body.to_vec()).unwrap();
        assert!(text.contains("internal server error"));
        assert!(text.contains("request_id"));
        assert!(!text.contains("panel.json"));
        assert!(!text.contains("permission denied"));
    }

    #[test]
    fn expected_errors_keep_their_client_safe_message() {
        let response = ApiError::BadRequest("path is invalid".into()).into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}
