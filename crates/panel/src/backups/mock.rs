//! Mock provider for tests – simulates cloud behavior without real credentials.
#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::RwLock;

use crate::backups::provider::{
    BackupArtifact, BackupProvider, BackupProviderKind, BackupStream, ProviderHealth, RemoteBackup,
};
use crate::error::ApiError;

#[derive(Default)]
#[allow(dead_code)]
pub struct MockBackupProvider {
    // server_id -> (backup_id -> RemoteBackup)
    store: Arc<RwLock<HashMap<String, HashMap<String, RemoteBackup>>>>,
    fail_next_upload: Arc<RwLock<bool>>,
    fail_next_delete: Arc<RwLock<bool>>,
}

impl MockBackupProvider {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub async fn set_fail_upload(&self, fail: bool) {
        *self.fail_next_upload.write().await = fail;
    }

    pub async fn set_fail_delete(&self, fail: bool) {
        *self.fail_next_delete.write().await = fail;
    }
}

#[async_trait::async_trait]
impl BackupProvider for MockBackupProvider {
    async fn upload(
        &self,
        server_id: &str,
        artifact: &BackupArtifact,
    ) -> Result<RemoteBackup, ApiError> {
        if *self.fail_next_upload.read().await {
            *self.fail_next_upload.write().await = false;
            return Err(ApiError::Internal(anyhow::anyhow!("mock upload failure")));
        }
        let remote = RemoteBackup {
            id: artifact.id.clone(),
            server_id: server_id.to_string(),
            provider: BackupProviderKind::GoogleDrive,
            remote_id: format!("mock-{}", artifact.id),
            created_at: artifact.created_at.clone(),
            size_bytes: artifact.size_bytes,
            checksum_sha256: artifact.checksum_sha256.clone(),
            note: artifact.note.clone(),
        };
        let mut store = self.store.write().await;
        store
            .entry(server_id.to_string())
            .or_default()
            .insert(artifact.id.clone(), remote.clone());
        Ok(remote)
    }

    async fn list(&self, server_id: &str) -> Result<Vec<RemoteBackup>, ApiError> {
        let store = self.store.read().await;
        let mut out = store
            .get(server_id)
            .map(|m| m.values().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        out.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(out)
    }

    async fn delete(&self, server_id: &str, backup_id: &str) -> Result<(), ApiError> {
        if *self.fail_next_delete.read().await {
            *self.fail_next_delete.write().await = false;
            return Err(ApiError::Internal(anyhow::anyhow!("mock delete failure")));
        }
        let mut store = self.store.write().await;
        if let Some(map) = store.get_mut(server_id) {
            if map.remove(backup_id).is_some() {
                return Ok(());
            }
        }
        Err(ApiError::NotFound("backup".into()))
    }

    async fn download(&self, server_id: &str, backup_id: &str) -> Result<BackupStream, ApiError> {
        let store = self.store.read().await;
        let exists = store
            .get(server_id)
            .and_then(|m| m.get(backup_id))
            .is_some();
        if !exists {
            return Err(ApiError::NotFound("backup".into()));
        }
        // Return empty stream for mock
        let cursor = std::io::Cursor::new(b"mock backup data".to_vec());
        Ok(Box::pin(tokio::io::BufReader::new(cursor)))
    }

    async fn health_check(&self) -> Result<ProviderHealth, ApiError> {
        Ok(ProviderHealth {
            ok: true,
            message: "mock ok".into(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backups::provider::BackupProviderKind;

    #[tokio::test]
    async fn mock_upload_and_list() {
        let provider = MockBackupProvider::new();
        let artifact = BackupArtifact {
            id: "b1".into(),
            server_id: "srv-1".into(),
            note: "test".into(),
            staging_path: std::path::PathBuf::from("/tmp/fake"),
            size_bytes: 123,
            checksum_sha256: Some("abc".into()),
            created_at: time::OffsetDateTime::now_utc()
                .format(&time::format_description::well_known::Rfc3339)
                .unwrap(),
        };
        let remote = provider.upload("srv-1", &artifact).await.unwrap();
        assert_eq!(remote.id, "b1");
        assert_eq!(remote.provider, BackupProviderKind::GoogleDrive);
        let list = provider.list("srv-1").await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, "b1");
        provider.delete("srv-1", "b1").await.unwrap();
        assert!(provider.list("srv-1").await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn mock_upload_failure_is_reported() {
        let provider = MockBackupProvider::new();
        provider.set_fail_upload(true).await;
        let artifact = BackupArtifact {
            id: "b1".into(),
            server_id: "srv-1".into(),
            note: "".into(),
            staging_path: std::path::PathBuf::from("/tmp/fake"),
            size_bytes: 0,
            checksum_sha256: None,
            created_at: "2026-08-29T00:00:00Z".into(),
        };
        assert!(provider.upload("srv-1", &artifact).await.is_err());
        // Next upload should succeed (fail flag cleared)
        assert!(provider.upload("srv-1", &artifact).await.is_ok());
    }
}
