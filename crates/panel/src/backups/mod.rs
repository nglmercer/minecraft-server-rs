//! Pluggable backup providers and retention policies.
//!
//! The HTTP API does not contain provider-specific logic; it works through the
//! `BackupProvider` trait. Adding a new provider only requires implementing the
//! trait and registering it in the factory.

pub mod google_drive;
pub mod local;
pub mod mock;
pub mod provider;
pub mod retention;
pub mod secret;

pub use crate::store::{
    BackupProviderKind, BackupRetentionPolicy, GoogleDriveConfig, StoredBackup,
};
#[allow(unused_imports)]
pub use provider::{BackupArtifact, BackupProvider, BackupStream, ProviderHealth, RemoteBackup};
#[allow(unused_imports)]
pub use secret::SecretStorage;

use std::sync::Arc;

use crate::state::AppState;
use crate::store::ServerRecord;

/// Factory: return the provider that should be used for `server`.
///
/// Global settings are used when `server.backup_policy` is `None`, otherwise
/// the server's override is used. No `if provider == "google_drive"` is
/// scattered through the API.
pub async fn provider_for(
    state: &AppState,
    server: &ServerRecord,
) -> anyhow::Result<Arc<dyn BackupProvider>> {
    let (kind, google_drive) = if let Some(policy) = server.backup_policy.as_ref() {
        let global = state.store.read().await.backup_storage.clone();
        (
            policy.provider.clone(),
            policy.google_drive.clone().or(global.google_drive.clone()),
        )
    } else {
        let global = state.store.read().await.backup_storage.clone();
        (global.provider.clone(), global.google_drive.clone())
    };

    match kind {
        BackupProviderKind::Local => Ok(Arc::new(local::LocalBackupProvider::new(
            state.data_dir.clone(),
        ))),
        BackupProviderKind::GoogleDrive => {
            let config = google_drive.ok_or_else(|| {
                anyhow::anyhow!("google drive provider selected but not configured")
            })?;
            Ok(Arc::new(google_drive::GoogleDriveBackupProvider::new(
                state.data_dir.clone(),
                config,
            )))
        }
    }
}

/// Reconstruct the provider that owns `stored`, using immutable backup metadata
/// rather than the server's current settings. This ensures a backup remains
/// accessible after provider/folder switches.
pub async fn provider_for_stored(
    state: &AppState,
    stored: &StoredBackup,
) -> anyhow::Result<Arc<dyn BackupProvider>> {
    match stored.provider {
        BackupProviderKind::Local => Ok(Arc::new(local::LocalBackupProvider::new(
            state.data_dir.clone(),
        ))),
        BackupProviderKind::GoogleDrive => {
            // Prefer the immutable location captured at backup time
            let config = if let (Some(folder_id), Some(cred)) = (
                stored.google_drive_folder_id.clone(),
                stored.google_drive_credential_ref.clone(),
            ) {
                GoogleDriveConfig {
                    folder_id,
                    credential_ref: cred,
                }
            } else {
                // Fallback to current server/global settings for legacy backups
                let server = state.store.server(&stored.server_id).await;
                if let Some(rec) = server {
                    let (_, gd) = if let Some(policy) = rec.backup_policy.as_ref() {
                        let global = state.store.read().await.backup_storage.clone();
                        (
                            policy.provider.clone(),
                            policy.google_drive.clone().or(global.google_drive.clone()),
                        )
                    } else {
                        let global = state.store.read().await.backup_storage.clone();
                        (global.provider.clone(), global.google_drive.clone())
                    };
                    gd.ok_or_else(|| {
                        anyhow::anyhow!(
                            "google drive provider not configured for legacy backup {}",
                            stored.id
                        )
                    })?
                } else {
                    // Server no longer exists but backup metadata remains (should have been cleaned,
                    // but for direct remote_id deletion we still need a config)
                    let global = state.store.read().await.backup_storage.clone();
                    global.google_drive.ok_or_else(|| {
                        anyhow::anyhow!("google drive not configured for backup {}", stored.id)
                    })?
                }
            };
            Ok(Arc::new(google_drive::GoogleDriveBackupProvider::new(
                state.data_dir.clone(),
                config,
            )))
        }
    }
}

/// Effective retention for a server (global vs override).
pub async fn retention_for(state: &AppState, server: &ServerRecord) -> BackupRetentionPolicy {
    if let Some(policy) = server.backup_policy.as_ref() {
        policy.retention.clone()
    } else {
        state.store.read().await.backup_storage.retention.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::BackupProviderKind;

    #[tokio::test]
    async fn local_provider_contract() {
        let tmp = tempfile::tempdir().unwrap();
        let provider = local::LocalBackupProvider::new(tmp.path().to_path_buf());
        let server_id = "srv-contract-local";
        let staging = tmp.path().join("staging.tar.gz");
        tokio::fs::write(&staging, b"fake backup data")
            .await
            .unwrap();
        let artifact = provider::BackupArtifact {
            id: "test1".into(),
            server_id: server_id.into(),
            note: "contract test".into(),
            staging_path: staging.clone(),
            size_bytes: 16,
            checksum_sha256: Some("abc".into()),
            created_at: time::OffsetDateTime::now_utc()
                .format(&time::format_description::well_known::Rfc3339)
                .unwrap(),
        };
        let remote = provider.upload(server_id, &artifact).await.unwrap();
        assert_eq!(remote.id, "test1");
        assert_eq!(remote.provider, BackupProviderKind::Local);
        let list = provider.list(server_id).await.unwrap();
        assert_eq!(list.len(), 1);
        let dl = provider.download(server_id, "test1").await;
        assert!(dl.is_ok());
        provider.delete(server_id, "test1").await.unwrap();
        assert!(provider.list(server_id).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn retention_with_mock_provider_preserves_new_backup_on_failure() {
        // Critical invariant: failed upload must not delete existing backup
        let provider = mock::MockBackupProvider::new();
        let server_id = "srv-retention";
        // Existing backup A
        let a = provider::BackupArtifact {
            id: "A".into(),
            server_id: server_id.into(),
            note: "".into(),
            staging_path: std::path::PathBuf::from("/tmp/fake"),
            size_bytes: 10,
            checksum_sha256: None,
            created_at: (time::OffsetDateTime::now_utc() - time::Duration::days(2))
                .format(&time::format_description::well_known::Rfc3339)
                .unwrap(),
        };
        provider.upload(server_id, &a).await.unwrap();
        // Try to upload B but fail
        provider.set_fail_upload(true).await;
        let b = provider::BackupArtifact {
            id: "B".into(),
            server_id: server_id.into(),
            note: "".into(),
            staging_path: std::path::PathBuf::from("/tmp/fake"),
            size_bytes: 10,
            checksum_sha256: None,
            created_at: time::OffsetDateTime::now_utc()
                .format(&time::format_description::well_known::Rfc3339)
                .unwrap(),
        };
        assert!(provider.upload(server_id, &b).await.is_err());
        // A must still exist
        let list = provider.list(server_id).await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, "A");
    }
}
