//! Local backup provider – wraps the existing guardian backup logic.

use std::path::PathBuf;

use crate::backups::provider::{
    BackupArtifact, BackupProvider, BackupProviderKind, BackupStream, ProviderHealth, RemoteBackup,
};
use crate::error::ApiError;

pub struct LocalBackupProvider {
    data_dir: PathBuf,
}

impl LocalBackupProvider {
    pub fn new(data_dir: PathBuf) -> Self {
        Self { data_dir }
    }

    fn backup_dir(&self, server_id: &str) -> PathBuf {
        self.data_dir.join("backups").join(server_id)
    }
}

#[async_trait::async_trait]
impl BackupProvider for LocalBackupProvider {
    async fn upload(
        &self,
        server_id: &str,
        artifact: &BackupArtifact,
    ) -> Result<RemoteBackup, ApiError> {
        // For local, "upload" is just moving the staging file into the backup directory.
        // We already have a consistent archive at staging_path; we need to atomically
        // place it into backup_dir and create metadata.
        let backup_dir = self.backup_dir(server_id);
        crate::state::secure_directory(&backup_dir)
            .await
            .map_err(ApiError::Internal)?;

        // Copy staging file to backup_dir with the artifact's id as filename
        let target = backup_dir.join(format!("{}.tar.gz", artifact.id));
        let staging = artifact.staging_path.clone();
        tokio::task::spawn_blocking(move || -> std::io::Result<()> {
            std::fs::copy(&staging, &target)?;
            Ok(())
        })
        .await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("copy failed: {e}")))?
        .map_err(|e| ApiError::Internal(e.into()))?;

        // Create metadata JSON alongside, mirroring guardian::backup behavior
        let backup = guardian::Backup {
            id: artifact.id.clone(),
            created_at: artifact.created_at.clone(),
            size: artifact.size_bytes,
            note: artifact.note.clone(),
        };
        let meta_bytes =
            serde_json::to_vec_pretty(&backup).map_err(|e| ApiError::Internal(e.into()))?;
        let backup_dir_clone = backup_dir.clone();
        let id_clone = artifact.id.clone();
        tokio::task::spawn_blocking(move || {
            let fs = guardian::ScopedFs::open(&backup_dir_clone)
                .map_err(|e| ApiError::Internal(anyhow::anyhow!("opening backup dir: {e}")))?;
            fs.write_atomic(format!("{}.json", id_clone), &meta_bytes)
                .map_err(|e| ApiError::Internal(e.into()))?;
            Ok::<(), ApiError>(())
        })
        .await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("task failed: {e}")))?
        .map_err(|e| e)?;

        Ok(RemoteBackup {
            id: artifact.id.clone(),
            server_id: server_id.to_string(),
            provider: BackupProviderKind::Local,
            remote_id: artifact.id.clone(),
            created_at: artifact.created_at.clone(),
            size_bytes: artifact.size_bytes,
            checksum_sha256: artifact.checksum_sha256.clone(),
            note: artifact.note.clone(),
        })
    }

    async fn list(&self, server_id: &str) -> Result<Vec<RemoteBackup>, ApiError> {
        let backup_dir = self.backup_dir(server_id);
        let backups = guardian::backup::list(&backup_dir)
            .await
            .map_err(|e| ApiError::Internal(e.into()))?;
        let mut out = Vec::new();
        for b in backups {
            // Try to read checksum if we stored it in StoredBackup, but guardian::Backup doesn't have it.
            // For local, we keep checksum in StoredBackup metadata if available, but for now read from backup dir's StoredBackup list.
            // We'll just return without checksum; the stored metadata will be merged at a higher layer.
            out.push(RemoteBackup {
                id: b.id.clone(),
                server_id: server_id.to_string(),
                provider: BackupProviderKind::Local,
                remote_id: b.id.clone(),
                created_at: b.created_at.clone(),
                size_bytes: b.size,
                checksum_sha256: None,
                note: b.note.clone(),
            });
        }
        Ok(out)
    }

    async fn delete(&self, server_id: &str, backup_id: &str) -> Result<(), ApiError> {
        let backup_dir = self.backup_dir(server_id);
        guardian::backup::delete(&backup_dir, backup_id)
            .await
            .map_err(|e| match e {
                guardian::Error::BackupNotFound(_) => ApiError::NotFound("backup".into()),
                _ => ApiError::Internal(e.into()),
            })?;
        Ok(())
    }

    async fn download(&self, server_id: &str, backup_id: &str) -> Result<BackupStream, ApiError> {
        let backup_dir = self.backup_dir(server_id);
        let fs = crate::filesystem::open(backup_dir.clone())
            .await
            .map_err(|e| ApiError::Internal(e.into()))?;
        let archive_name = format!("{backup_id}.tar.gz");
        let file = tokio::task::spawn_blocking(move || fs.open_file(&archive_name))
            .await
            .map_err(|e| ApiError::Internal(anyhow::anyhow!("task failed: {e}")))?
            .map_err(|_| ApiError::NotFound("backup".into()))?;
        let tokio_file = tokio::fs::File::from_std(file);
        let stream: BackupStream = Box::pin(tokio_file);
        Ok(stream)
    }

    async fn health_check(&self) -> Result<ProviderHealth, ApiError> {
        Ok(ProviderHealth {
            ok: true,
            message: "local provider ok".into(),
        })
    }
}
