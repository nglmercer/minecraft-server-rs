//! Errors returned by the Playit adapter.

use playit_ipc::ipc::IpcError;

/// A failed Playit operation.
#[derive(Debug, thiserror::Error)]
pub enum PlayitError {
    /// The daemon could not be reached, or rejected the IPC exchange.
    #[error("Playit IPC error: {0}")]
    Ipc(#[from] IpcError),
    /// The daemon answered, but did not accept the requested command.
    #[error("Playit rejected the request: {0}")]
    Rejected(String),
    /// The daemon returned a response that cannot be used safely.
    #[error("invalid Playit response: {0}")]
    Protocol(String),
}

impl PlayitError {
    /// Whether this error means the daemon is currently unavailable.
    pub fn is_unavailable(&self) -> bool {
        matches!(
            self,
            Self::Ipc(IpcError::ConnectionFailed(_) | IpcError::NotRunning | IpcError::IoError(_))
        )
    }

    /// Whether the daemon and this panel disagree about the IPC protocol.
    pub fn is_unsupported(&self) -> bool {
        matches!(self, Self::Ipc(IpcError::ProtocolMismatch { .. }))
    }
}
