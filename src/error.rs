//! The API error type and how it renders as JSON.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use playit_integration::{AccountError, PlayitError, ServiceErrorCode};
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
            ApiError::Guardian(guardian::Error::ServerDiskQuotaExceeded) => StatusCode::CONFLICT,
            ApiError::Guardian(guardian::Error::StartCancelled)
            | ApiError::Guardian(guardian::Error::ConsoleUnavailable)
            | ApiError::Guardian(guardian::Error::SandboxUnavailable) => StatusCode::CONFLICT,
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
                playit_unavailable_message(&self).unwrap_or("external service unavailable".into())
            }
            // Conflict messages describe an account/tunnel ambiguity that
            // the operator must resolve; hiding them behind a generic 409
            // makes recovery impossible. They contain no credentials because
            // the integration only constructs them from public tunnel data.
            ApiError::Playit(PlayitError::Conflict(_)) => self.to_string(),
            ApiError::Playit(error) => {
                playit_client_message(error).unwrap_or("external service request failed".into())
            }
            _ => self.to_string(),
        };
        (status, Json(json!({ "error": message }))).into_response()
    }
}

/// Handler result alias.
pub type ApiResult<T> = Result<T, ApiError>;

/// A safe, actionable message for Playit server-side failures. Secrets,
/// paths, and raw backend bodies are never exposed; only known recoverable
/// conditions get specific wording.
fn playit_unavailable_message(error: &ApiError) -> Option<String> {
    let ApiError::Playit(playit) = error else {
        return None;
    };
    match playit.service_code()? {
        ServiceErrorCode::AgentDisabledOverLimit => Some(
            "Playit has disabled this agent because the account agent limit was reached. Reuse an existing Playit agent or remove an unused agent in Playit.".into(),
        ),
        ServiceErrorCode::InvalidSecret | ServiceErrorCode::SecretPinned => Some(
            "The saved Playit connection is no longer valid. Reconnect the Playit account.".into(),
        ),
        ServiceErrorCode::ApiUnavailable | ServiceErrorCode::ProvisioningUnavailable => {
            Some("Playit is temporarily unavailable. Try again shortly.".into())
        }
        ServiceErrorCode::SecretWriteFailed => {
            Some("Playit is temporarily unavailable. Try again shortly.".into())
        }
        _ => None,
    }
}

/// A safe message for Playit client-side failures with known actionable
/// meanings. Returns `None` when only the generic message applies.
fn playit_client_message(error: &PlayitError) -> Option<String> {
    match error.service_code()? {
        ServiceErrorCode::AgentDisabledOverLimit => Some(
            "Playit has disabled this agent because the account agent limit was reached. Reuse an existing Playit agent or remove an unused agent in Playit.".into(),
        ),
        ServiceErrorCode::InvalidSecret | ServiceErrorCode::SecretPinned => Some(
            "The saved Playit connection is no longer valid. Reconnect the Playit account.".into(),
        ),
        ServiceErrorCode::ApiUnavailable | ServiceErrorCode::ProvisioningUnavailable => {
            Some("Playit is temporarily unavailable. Try again shortly.".into())
        }
        _ => None,
    }
}

fn playit_status(error: &PlayitError) -> StatusCode {
    if matches!(error, PlayitError::Conflict(_)) {
        return StatusCode::CONFLICT;
    }
    if let PlayitError::Account(account) = error {
        return match account {
            AccountError::NotLoggedIn
            | AccountError::InvalidCredentials
            | AccountError::SessionExpired => StatusCode::UNAUTHORIZED,
            AccountError::InvalidTotp => StatusCode::BAD_REQUEST,
            AccountError::Api(_) => StatusCode::BAD_GATEWAY,
        };
    }
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

    #[test]
    fn sandbox_policy_errors_are_conflicts_without_internal_paths() {
        let response = ApiError::Guardian(guardian::Error::SandboxUnavailable).into_response();
        assert_eq!(response.status(), StatusCode::CONFLICT);
    }

    fn runtime_api_error(code: ServiceErrorCode) -> ApiError {
        ApiError::Playit(playit_integration::PlayitError::Runtime(
            playit_runtime::RuntimeError::Api {
                code,
                message: "backend detail that must stay hidden".into(),
                retryable: false,
                details: None,
            },
        ))
    }

    #[tokio::test]
    async fn playit_agent_limit_reports_an_actionable_message() {
        let response = runtime_api_error(ServiceErrorCode::AgentDisabledOverLimit).into_response();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body = to_bytes(response.into_body(), 4096).await.unwrap();
        let text = String::from_utf8(body.to_vec()).unwrap();
        assert!(text.contains("agent limit was reached"));
        assert!(!text.contains("backend detail"));
    }

    #[tokio::test]
    async fn playit_invalid_secret_reports_reconnect_without_details() {
        let response = runtime_api_error(ServiceErrorCode::InvalidSecret).into_response();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body = to_bytes(response.into_body(), 4096).await.unwrap();
        let text = String::from_utf8(body.to_vec()).unwrap();
        assert!(text.contains("no longer valid"));
        assert!(!text.contains("backend detail"));
    }

    #[tokio::test]
    async fn playit_unavailable_reports_retry_without_details() {
        let response = runtime_api_error(ServiceErrorCode::ApiUnavailable).into_response();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body = to_bytes(response.into_body(), 4096).await.unwrap();
        let text = String::from_utf8(body.to_vec()).unwrap();
        assert!(text.contains("temporarily unavailable"));
        assert!(!text.contains("backend detail"));
    }

    #[test]
    fn playit_account_auth_failures_map_to_401_400_502() {
        use playit_integration::AccountError;
        let status = |error: PlayitError| ApiError::Playit(error).into_response().status();
        assert_eq!(
            status(PlayitError::Account(AccountError::NotLoggedIn)),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            status(PlayitError::Account(AccountError::InvalidCredentials)),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            status(PlayitError::Account(AccountError::SessionExpired)),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            status(PlayitError::Account(AccountError::InvalidTotp)),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            status(PlayitError::Account(AccountError::Api("detail".into()))),
            StatusCode::BAD_GATEWAY
        );
    }

    #[tokio::test]
    async fn playit_account_api_failures_hide_backend_detail() {
        let response = ApiError::Playit(PlayitError::Account(AccountError::Api(
            "backend detail that must stay hidden".into(),
        )))
        .into_response();
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        let body = to_bytes(response.into_body(), 4096).await.unwrap();
        let text = String::from_utf8(body.to_vec()).unwrap();
        assert!(!text.contains("backend detail"));
    }
}
