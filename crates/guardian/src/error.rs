//! Error type shared by every guardian operation.

use std::path::PathBuf;

/// Convenience alias used throughout the crate.
pub type Result<T> = std::result::Result<T, Error>;

/// Everything that can go wrong while provisioning or supervising a server.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Java could not be discovered and could not be installed.
    #[error("no usable Java {0} found, and installing one failed: {1}")]
    JavaUnavailable(u32, String),

    /// The underlying Java discovery/provisioning library failed.
    #[error("java: {0}")]
    Java(#[from] java_path::Error),

    /// Resolving or downloading the server artifact failed.
    #[error("minecraft core: {0}")]
    Core(#[from] minecraft_core::Error),

    /// A filesystem operation failed, with the path that caused it.
    #[error("io error at {path}: {source}")]
    Io {
        /// The path being operated on.
        path: PathBuf,
        /// The underlying error.
        #[source]
        source: std::io::Error,
    },

    /// A filesystem operation with no meaningful path to report.
    #[error("io error: {0}")]
    PlainIo(#[from] std::io::Error),

    /// Serialising or deserialising a config/state file failed.
    #[error("serde: {0}")]
    Serde(#[from] serde_json::Error),

    /// The requested transition is impossible from the current status.
    #[error("server is {current}, cannot {action}")]
    InvalidTransition {
        /// Status at the time of the request.
        current: &'static str,
        /// The rejected action.
        action: &'static str,
    },

    /// stdin was not available, so the command could not be delivered.
    #[error("server console is not writable")]
    ConsoleUnavailable,

    /// The EULA has not been accepted, so starting would be pointless.
    #[error("the Minecraft EULA has not been accepted")]
    EulaNotAccepted,

    /// A backup id was not a plain, safe file stem.
    #[error("invalid backup id: {0}")]
    InvalidBackupId(String),

    /// No backup with that id exists.
    #[error("backup {0} not found")]
    BackupNotFound(String),

    /// An archive member tried to write outside the destination directory.
    #[error("archive entry {0} would escape the destination")]
    UnsafeArchiveEntry(String),

    /// Provisioning exceeded the configured limit.
    #[error("preparation did not finish within {0}s")]
    PrepareTimedOut(u64),

    /// A blocking task panicked or was cancelled.
    #[error("background task failed: {0}")]
    Task(String),
}

impl Error {
    /// Attach a path to an [`std::io::Error`].
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Error::Io { path: path.into(), source }
    }
}
