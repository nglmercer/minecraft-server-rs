//! Provider trait and neutral metadata types.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::pin::Pin;
use tokio::io::AsyncRead;

use crate::error::ApiError;
pub use crate::store::{BackupProviderKind, StoredBackup};

/// Temporary artifact produced before upload. The provider does not need a
/// filesystem path – cloud providers stream the file.
#[derive(Debug)]
#[allow(dead_code)]
pub struct BackupArtifact {
    pub id: String,
    pub server_id: String,
    pub note: String,
    /// Absolute path to the temporary local archive (staging).
    pub staging_path: std::path::PathBuf,
    pub size_bytes: u64,
    pub checksum_sha256: Option<String>,
    pub created_at: String,
}

/// Provider's view of a remote backup, after `upload` or `list`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteBackup {
    pub id: String,
    pub server_id: String,
    pub provider: BackupProviderKind,
    pub remote_id: String,
    pub created_at: String,
    pub size_bytes: u64,
    pub checksum_sha256: Option<String>,
    pub note: String,
}

impl From<StoredBackup> for RemoteBackup {
    fn from(s: StoredBackup) -> Self {
        Self {
            id: s.id,
            server_id: s.server_id,
            provider: s.provider,
            remote_id: s.remote_id,
            created_at: s.created_at,
            size_bytes: s.size_bytes,
            checksum_sha256: s.checksum_sha256,
            note: s.note,
        }
    }
}

pub type BackupStream = Pin<Box<dyn AsyncRead + Send>>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderHealth {
    pub ok: bool,
    pub message: String,
}

#[async_trait]
pub trait BackupProvider: Send + Sync {
    async fn upload(
        &self,
        server_id: &str,
        artifact: &BackupArtifact,
    ) -> Result<RemoteBackup, ApiError>;

    async fn list(&self, server_id: &str) -> Result<Vec<RemoteBackup>, ApiError>;

    async fn delete(&self, server_id: &str, backup_id: &str) -> Result<(), ApiError>;

    async fn download(&self, server_id: &str, backup_id: &str) -> Result<BackupStream, ApiError>;

    async fn health_check(&self) -> Result<ProviderHealth, ApiError>;
}
