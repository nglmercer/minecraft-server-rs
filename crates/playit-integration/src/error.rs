//! Errors returned by the Playit adapter.

use playit_ipc::ipc::IpcError;
use playit_ipc::model::ServiceErrorCode;
use playit_runtime::RuntimeError;

/// A failed Playit operation.
#[derive(Debug, thiserror::Error)]
pub enum PlayitError {
    /// The external daemon could not be reached or completed the IPC exchange.
    #[error("Playit IPC error: {0}")]
    Ipc(#[from] IpcError),
    /// The embedded runtime failed while performing the operation.
    #[error("Playit runtime error: {0}")]
    Runtime(#[from] RuntimeError),
    /// Playit is not currently available to the panel.
    #[error("Playit integration unavailable: {0}")]
    Unavailable(String),
    /// Playit answered, but did not accept the requested command.
    #[error("Playit rejected the request: {0}")]
    Rejected(String),
    /// Playit returned a response that cannot be used safely.
    #[error("invalid Playit response: {0}")]
    Protocol(String),
}

impl PlayitError {
    /// Whether this error means the Playit service is currently unavailable.
    pub fn is_unavailable(&self) -> bool {
        match self {
            Self::Ipc(
                IpcError::ConnectionFailed(_) | IpcError::NotRunning | IpcError::IoError(_),
            )
            | Self::Unavailable(_) => true,
            Self::Runtime(error) => runtime_error_is_unavailable(error),
            Self::Rejected(_) | Self::Protocol(_) | Self::Ipc(_) => false,
        }
    }

    /// Whether the backend and this panel disagree about the protocol.
    pub fn is_unsupported(&self) -> bool {
        match self {
            Self::Ipc(IpcError::ProtocolMismatch { .. }) => true,
            Self::Runtime(error) => {
                runtime_error_has_code(error, ServiceErrorCode::UnsupportedProtocol)
            }
            _ => false,
        }
    }
}

fn runtime_error_is_unavailable(error: &RuntimeError) -> bool {
    matches!(error, RuntimeError::Stopped | RuntimeError::Io(_))
        || runtime_error_has_code(error, ServiceErrorCode::ApiUnavailable)
}

fn runtime_error_has_code(error: &RuntimeError, expected: ServiceErrorCode) -> bool {
    let code = match error {
        RuntimeError::Secret { code, .. }
        | RuntimeError::Setup { code, .. }
        | RuntimeError::Api { code, .. }
        | RuntimeError::InvalidState { code, .. } => code,
        RuntimeError::Io(_) | RuntimeError::Stopped => return false,
    };
    match expected {
        ServiceErrorCode::ApiUnavailable => matches!(code, ServiceErrorCode::ApiUnavailable),
        ServiceErrorCode::UnsupportedProtocol => {
            matches!(code, ServiceErrorCode::UnsupportedProtocol)
        }
        _ => false,
    }
}
