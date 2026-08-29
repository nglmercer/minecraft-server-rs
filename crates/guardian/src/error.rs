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

    /// An archive exceeded an explicit extraction safety limit.
    #[error("archive exceeds its safety limits: {0}")]
    ArchiveLimit(String),

    /// Restoring this archive would exceed the server's configured disk quota.
    #[error("server storage quota exceeded")]
    ServerDiskQuotaExceeded,

    /// Provisioning exceeded the configured limit.
    #[error("preparation did not finish within {0}s")]
    PrepareTimedOut(u64),

    /// A blocking task panicked or was cancelled.
    #[error("background task failed: {0}")]
    Task(String),

    /// A configuration value would make the process or its supervisor unsafe
    /// to run.
    #[error("invalid server configuration: {0}")]
    InvalidConfiguration(String),

    /// A console command exceeded the deliberately small command boundary.
    #[error("invalid console command: {0}")]
    InvalidCommand(&'static str),

    /// Provisioning or launching was cancelled by an operator.
    #[error("server start was cancelled")]
    StartCancelled,

    /// No supported OS sandbox is available and unsandboxed execution was not
    /// explicitly enabled by the operator.
    #[error(
        "a Minecraft process sandbox is unavailable; pass --allow-unsandboxed-servers to explicitly allow unsafe execution"
    )]
    SandboxUnavailable,
}

impl Error {
    /// Attach a path to an [`std::io::Error`].
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Error::Io {
            path: path.into(),
            source,
        }
    }

    /// A message safe to show in a server operator's console. Internal
    /// filesystem paths and dependency error chains stay in structured logs.
    pub fn client_message(&self) -> String {
        match self {
            Error::InvalidTransition { .. }
            | Error::ConsoleUnavailable
            | Error::EulaNotAccepted
            | Error::InvalidBackupId(_)
            | Error::BackupNotFound(_)
            | Error::UnsafeArchiveEntry(_)
            | Error::ArchiveLimit(_)
            | Error::ServerDiskQuotaExceeded
            | Error::PrepareTimedOut(_)
            | Error::InvalidConfiguration(_)
            | Error::InvalidCommand(_)
            | Error::StartCancelled
            | Error::SandboxUnavailable => self.to_string(),
            Error::JavaUnavailable(major, _) => format!("Java {major} is unavailable"),
            Error::Java(_)
            | Error::Core(_)
            | Error::Io { .. }
            | Error::PlainIo(_)
            | Error::Serde(_)
            | Error::Task(_) => "an internal error prevented the server operation".into(),
        }
    }
}
