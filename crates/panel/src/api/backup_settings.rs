//! Backup storage settings API.

use axum::extract::{Path, Query, State};
#[allow(unused_imports)]
use axum::routing::{delete, get, patch, post};
use axum::response::Html;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Instant;

use crate::auth::AdminIdentity;
use crate::backups::google_oauth;
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
    #[serde(default)]
    pub oauth_connected: bool,
    #[serde(default)]
    pub oauth_configured: bool,
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
    let oauth_connected = google_oauth::is_connected(&state.data_dir).await;
    let oauth_configured = google_oauth::client_config().is_some();
    let view = BackupSettingsView {
        provider: s.provider.clone(),
        retention: s.retention.clone(),
        google_drive: s.google_drive.as_ref().map(|c| GoogleDriveView {
            folder_id: c.folder_id.clone(),
            credentials_present: false,
            configured: true,
            oauth_connected,
            oauth_configured,
        }),
    };
    let mut view = view;
    if let Some(g) = &data.backup_storage.google_drive {
        let storage = SecretStorage::new(state.data_dir.clone());
        if let Some(gd) = view.google_drive.as_mut() {
            gd.credentials_present = storage.exists(&g.credential_ref).await;
            // oauth flags already set; if no google_drive config but oauth exists, still show
        }
    } else {
        // No google_drive config yet — still report oauth status for UI (create stub)
        if oauth_configured || oauth_connected {
            // Provide empty view so frontend can show OAuth button even before folder configured
            if view.google_drive.is_none() {
                view.google_drive = Some(GoogleDriveView {
                    folder_id: String::new(),
                    credentials_present: false,
                    configured: false,
                    oauth_connected,
                    oauth_configured,
                });
            }
        }
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
    let _updated = state
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
                    oauth_connected: false,
                    oauth_configured: false,
                }),
            })
        })
        .await
        .map_err(|e| ApiError::Internal(e.into()))?
        .map_err(|e| e)?;
    tracing::info!(by = %admin.username, "backup storage settings updated");
    // Re-fetch to populate oauth flags
    get_global(State(state.clone()), AdminIdentity(admin)).await
}

#[derive(Deserialize)]
pub struct SecretUpload {
    pub content: String,
    #[serde(default)]
    pub credential_ref: Option<String>,
}

async fn upload_secret(
    State(state): State<Arc<AppState>>,
    AdminIdentity(_): AdminIdentity,
    Json(body): Json<SecretUpload>,
) -> ApiResult<Json<serde_json::Value>> {
    let storage = SecretStorage::new(state.data_dir.clone());
    let cref = body.credential_ref.as_deref().unwrap_or("google-drive");
    if cref.trim().is_empty()
        || cref.len() > 64
        || !cref
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(ApiError::BadRequest("invalid credential_ref".into()));
    }
    storage
        .write_secret(cref, body.content.as_bytes())
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    Ok(Json(
        serde_json::json!({ "ok": true, "credential_ref": cref }),
    ))
}

async fn test_connection(
    State(state): State<Arc<AppState>>,
    AdminIdentity(_): AdminIdentity,
) -> ApiResult<Json<serde_json::Value>> {
    drop(state.store.read().await);
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

// --- OAuth ---

#[derive(Deserialize)]
pub struct OAuthStartRequest {
    pub redirect_uri: String,
}

#[derive(Serialize)]
pub struct OAuthStartResponse {
    pub url: String,
    pub state: String,
}

async fn oauth_start(
    State(state): State<Arc<AppState>>,
    AdminIdentity(admin): AdminIdentity,
    Json(body): Json<OAuthStartRequest>,
) -> ApiResult<Json<OAuthStartResponse>> {
    let (client_id, _) = google_oauth::client_config()
        .ok_or_else(|| ApiError::Internal(anyhow::anyhow!("Google OAuth not configured: set MCPANEL_GOOGLE_CLIENT_ID and MCPANEL_GOOGLE_CLIENT_SECRET").into()))?;
    if body.redirect_uri.trim().is_empty() || !body.redirect_uri.starts_with("http") {
        return Err(ApiError::BadRequest("invalid redirect_uri".into()));
    }
    let st = google_oauth::new_state();
    let pending = google_oauth::OAuthPending {
        redirect_uri: body.redirect_uri.clone(),
        created_at: Instant::now(),
        admin: admin.username.clone(),
    };
    state.google_oauth_states.write().await.insert(st.clone(), pending);
    let url = google_oauth::build_auth_url(&client_id, &body.redirect_uri, &st);
    Ok(Json(OAuthStartResponse { url, state: st }))
}

#[derive(Deserialize)]
pub struct OAuthCallbackQuery {
    pub code: Option<String>,
    pub state: Option<String>,
    pub error: Option<String>,
}

async fn oauth_callback(
    State(state): State<Arc<AppState>>,
    Query(q): Query<OAuthCallbackQuery>,
) -> Result<Html<String>, ApiError> {
    if let Some(err) = q.error {
        return Ok(Html(format!("<html><body><h2>Google auth failed: {}</h2><p>You can close this window.</p></body></html>", err)));
    }
    let code = q.code.ok_or_else(|| ApiError::BadRequest("missing code".into()))?;
    let st = q.state.ok_or_else(|| ApiError::BadRequest("missing state".into()))?;
    let pending = {
        let mut map = state.google_oauth_states.write().await;
        map.remove(&st)
    };
    let Some(pending) = pending else {
        return Err(ApiError::BadRequest("invalid or expired state".into()));
    };
    if pending.created_at.elapsed() > std::time::Duration::from_secs(600) {
        return Err(ApiError::BadRequest("state expired".into()));
    }
    let (client_id, client_secret) = google_oauth::client_config()
        .ok_or_else(|| ApiError::Internal(anyhow::anyhow!("oauth not configured").into()))?;
    let tokens = google_oauth::exchange_code(&code, &pending.redirect_uri, &client_id, &client_secret).await?;
    google_oauth::save_tokens(&state.data_dir, &tokens).await.map_err(|e| ApiError::Internal(e.into()))?;
    tracing::info!(by=%pending.admin, "google drive oauth connected");
    Ok(Html(
        r##"<!doctype html><html><head><meta charset="utf-8"><title>Connected</title><style>body{font-family:system-ui;background:#0b0f14;color:#e6f0f2;display:grid;place-items:center;height:100vh;margin:0}div{border:1px solid #1e2a33;padding:24px;border-radius:12px;background:#111a22;text-align:center}a{color:#3dd68c}</style></head><body><div><h2>✓ Google Drive connected</h2><p>You can close this window and return to the panel.</p><p><a href='#' onclick='window.close()'>Close</a></p><script>setTimeout(()=>window.close(),2000)</script></div></body></html>"##.to_string(),
    ))
}

async fn oauth_status(
    State(state): State<Arc<AppState>>,
    AdminIdentity(_): AdminIdentity,
) -> ApiResult<Json<serde_json::Value>> {
    let connected = google_oauth::is_connected(&state.data_dir).await;
    let configured = google_oauth::client_config().is_some();
    Ok(Json(serde_json::json!({ "connected": connected, "configured": configured })))
}

async fn oauth_disconnect(
    State(state): State<Arc<AppState>>,
    AdminIdentity(admin): AdminIdentity,
) -> ApiResult<Json<serde_json::Value>> {
    google_oauth::delete_tokens(&state.data_dir).await.map_err(|e| ApiError::Internal(e.into()))?;
    tracing::info!(by=%admin.username, "google drive oauth disconnected");
    Ok(Json(serde_json::json!({ "ok": true })))
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
                oauth_connected: false,
                oauth_configured: false,
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
                        oauth_connected: false,
                        oauth_configured: false,
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
        .route("/settings/backups/google/oauth/start", post(oauth_start))
        .route("/settings/backups/google/oauth/callback", get(oauth_callback))
        .route("/settings/backups/google/oauth/status", get(oauth_status))
        .route("/settings/backups/google/oauth/disconnect", delete(oauth_disconnect))
}

pub fn server_router() -> Router<Arc<AppState>> {
    Router::new().route("/{id}/backup-settings", get(get_server).patch(patch_server))
}
