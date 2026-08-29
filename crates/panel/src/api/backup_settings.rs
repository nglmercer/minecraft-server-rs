//! Backup storage settings API.

use axum::extract::{Path, State};
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::auth::AdminIdentity;
use crate::backups::secret::SecretStorage;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use crate::store::{BackupProviderKind, BackupRetentionPolicy, GoogleDriveConfig};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupSettingsView {
    pub provider: BackupProviderKind,
    pub retention: BackupRetentionPolicy,
    pub google_drive: Option<GoogleDriveView>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoogleDriveView {
    pub folder_id: String,
    pub credentials_present: bool,
    pub configured: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateBackupSettings {
    pub provider: Option<BackupProviderKind>,
    pub retention: Option<BackupRetentionPolicy>,
    pub google_drive: Option<GoogleDriveUpdate>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GoogleDriveUpdate {
    pub folder_id: Option<String>,
    pub credential_ref: Option<String>,
}

async fn get_global(
    State(state): State<Arc<AppState>>,
    AdminIdentity(_): AdminIdentity,
) -> ApiResult<Json<BackupSettingsView>> {
    let data = state.store.read().await;
    let s = &data.backup_storage;
    let view = BackupSettingsView {
        provider: s.provider.clone(),
        retention: s.retention.clone(),
        google_drive: s.google_drive.as_ref().map(|c| GoogleDriveView {
            folder_id: c.folder_id.clone(),
            credentials_present: false,
            configured: true,
        }),
    };
    // Check credentials presence without exposing
    let mut view = view;
    if let Some(g) = &data.backup_storage.google_drive {
        let storage = SecretStorage::new(state.data_dir.clone());
        view.google_drive.as_mut().unwrap().credentials_present =
            storage.exists(&g.credential_ref).await;
    }
    Ok(Json(view))
}

async fn patch_global(
    State(state): State<Arc<AppState>>,
    AdminIdentity(admin): AdminIdentity,
    Json(body): Json<UpdateBackupSettings>,
) -> ApiResult<Json<BackupSettingsView>> {
    if let Some(ret) = &body.retention {
        ret.validate().map_err(ApiError::BadRequest)?;
    }
    if let Some(gd) = &body.google_drive {
        if let Some(fid) = &gd.folder_id {
            if fid.trim().is_empty() || fid.len() > 512 {
                return Err(ApiError::BadRequest("invalid folder_id".into()));
            }
        }
    }
    let updated = state
        .store
        .try_update(|data| -> ApiResult<BackupSettingsView> {
            if let Some(provider) = body.provider.clone() {
                data.backup_storage.provider = provider;
            }
            if let Some(retention) = body.retention.clone() {
                retention.validate().map_err(ApiError::BadRequest)?;
                data.backup_storage.retention = retention;
            }
            if let Some(gd) = body.google_drive.clone() {
                let mut cfg =
                    data.backup_storage
                        .google_drive
                        .clone()
                        .unwrap_or(GoogleDriveConfig {
                            folder_id: String::new(),
                            credential_ref: "google-drive".into(),
                        });
                if let Some(fid) = gd.folder_id {
                    cfg.folder_id = fid;
                }
                if let Some(cref) = gd.credential_ref {
                    cfg.credential_ref = cref;
                }
                cfg.validate().map_err(ApiError::BadRequest)?;
                data.backup_storage.google_drive = Some(cfg);
            }
            let s = &data.backup_storage;
            Ok(BackupSettingsView {
                provider: s.provider.clone(),
                retention: s.retention.clone(),
                google_drive: s.google_drive.as_ref().map(|c| GoogleDriveView {
                    folder_id: c.folder_id.clone(),
                    credentials_present: false,
                    configured: true,
                }),
            })
        })
        .await
        .map_err(|e| ApiError::Internal(e.into()))?
        .map_err(|e| e)?;
    tracing::info!(by = %admin.username, "backup storage settings updated");
    Ok(Json(updated))
}

#[derive(Deserialize)]
pub struct SecretUpload {
    pub content: String,
}

async fn upload_secret(
    State(state): State<Arc<AppState>>,
    AdminIdentity(_): AdminIdentity,
    Json(body): Json<SecretUpload>,
) -> ApiResult<Json<serde_json::Value>> {
    let storage = SecretStorage::new(state.data_dir.clone());
    // Expect base64 or raw JSON; for now accept raw JSON string
    storage
        .write_secret("google-drive", body.content.as_bytes())
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

async fn test_connection(
    State(state): State<Arc<AppState>>,
    AdminIdentity(_): AdminIdentity,
) -> ApiResult<Json<serde_json::Value>> {
    let data = state.store.read().await;
    let settings = data.backup_storage.clone();
    drop(data);
    // Create provider based on global settings (use first server or dummy)
    let dummy_record = crate::store::ServerRecord {
        id: "test".into(),
        name: "test".into(),
        config: guardian::ServerConfig::paper(state.server_dir("test"), "1.21.8"),
        policy: guardian::GuardianConfig::default(),
        playit: None,
        created_at: "2026-08-29T00:00:00Z".into(),
        backup_policy: None,
    };
    let provider = crate::backups::provider_for(&state, &dummy_record)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    let health = provider.health_check().await?;
    Ok(Json(
        serde_json::json!({ "ok": health.ok, "message": health.message }),
    ))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerBackupSettingsView {
    pub inherit_global: bool,
    pub provider: Option<BackupProviderKind>,
    pub retention: Option<BackupRetentionPolicy>,
    pub google_drive: Option<GoogleDriveView>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateServerBackupSettings {
    pub inherit_global: Option<bool>,
    pub provider: Option<BackupProviderKind>,
    pub retention: Option<BackupRetentionPolicy>,
    pub google_drive: Option<GoogleDriveUpdate>,
}

async fn get_server(
    State(state): State<Arc<AppState>>,
    AdminIdentity(_): AdminIdentity,
    Path(id): Path<String>,
) -> ApiResult<Json<ServerBackupSettingsView>> {
    let record = state
        .store
        .server(&id)
        .await
        .ok_or_else(|| ApiError::NotFound("server".into()))?;
    if let Some(policy) = record.backup_policy {
        Ok(Json(ServerBackupSettingsView {
            inherit_global: false,
            provider: Some(policy.provider),
            retention: Some(policy.retention),
            google_drive: policy.google_drive.map(|c| GoogleDriveView {
                folder_id: c.folder_id,
                credentials_present: false,
                configured: true,
            }),
        }))
    } else {
        Ok(Json(ServerBackupSettingsView {
            inherit_global: true,
            provider: None,
            retention: None,
            google_drive: None,
        }))
    }
}

async fn patch_server(
    State(state): State<Arc<AppState>>,
    AdminIdentity(admin): AdminIdentity,
    Path(id): Path<String>,
    Json(body): Json<UpdateServerBackupSettings>,
) -> ApiResult<Json<ServerBackupSettingsView>> {
    if let Some(ret) = &body.retention {
        ret.validate().map_err(ApiError::BadRequest)?;
    }
    let server_id = id.clone();
    let updated = state
        .store
        .try_update(move |data| -> ApiResult<ServerBackupSettingsView> {
            let Some(slot) = data.servers.iter_mut().find(|s| s.id == server_id) else {
                return Err(ApiError::NotFound("server".into()));
            };
            if body.inherit_global == Some(true) {
                slot.backup_policy = None;
            } else {
                let mut policy =
                    slot.backup_policy
                        .clone()
                        .unwrap_or(crate::store::ServerBackupPolicy {
                            provider: crate::store::BackupProviderKind::Local,
                            retention: crate::store::BackupRetentionPolicy::default(),
                            google_drive: None,
                        });
                if let Some(provider) = body.provider.clone() {
                    policy.provider = provider;
                }
                if let Some(retention) = body.retention.clone() {
                    retention.validate().map_err(ApiError::BadRequest)?;
                    policy.retention = retention;
                }
                if let Some(gd) = body.google_drive.clone() {
                    let mut cfg = policy.google_drive.clone().unwrap_or(GoogleDriveConfig {
                        folder_id: String::new(),
                        credential_ref: "google-drive".into(),
                    });
                    if let Some(fid) = gd.folder_id {
                        cfg.folder_id = fid;
                    }
                    if let Some(cref) = gd.credential_ref {
                        cfg.credential_ref = cref;
                    }
                    cfg.validate().map_err(ApiError::BadRequest)?;
                    policy.google_drive = Some(cfg);
                }
                slot.backup_policy = Some(policy);
            }
            let policy = slot.backup_policy.clone();
            Ok(ServerBackupSettingsView {
                inherit_global: policy.is_none(),
                provider: policy.as_ref().map(|p| p.provider.clone()),
                retention: policy.as_ref().map(|p| p.retention.clone()),
                google_drive: policy
                    .as_ref()
                    .and_then(|p| p.google_drive.clone())
                    .map(|c| GoogleDriveView {
                        folder_id: c.folder_id,
                        credentials_present: false,
                        configured: true,
                    }),
            })
        })
        .await
        .map_err(|e| ApiError::Internal(e.into()))?
        .map_err(|e| e)?;
    tracing::info!(server = %id, by = %admin.username, "server backup settings updated");
    Ok(Json(updated))
}

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/settings/backups", get(get_global).patch(patch_global))
        .route("/settings/backups/secret", post(upload_secret))
        .route("/settings/backups/test", post(test_connection))
}

pub fn server_router() -> Router<Arc<AppState>> {
    Router::new().route("/{id}/backup-settings", get(get_server).patch(patch_server))
}
