//! Backup creation, listing, download, restore and deletion via pluggable providers.

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::StreamExt;
use serde::Deserialize;
use std::sync::Arc;
use tokio_util::io::ReaderStream;
use uuid::Uuid;

use crate::auth::Identity;
use crate::backups::retention::plan_retention;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use crate::store::StoredBackup;
use crate::tickets::Resource;

/// Resolve a server the caller may act on.
async fn authorized(
    state: &AppState,
    identity: &Identity,
    id: &str,
) -> ApiResult<std::path::PathBuf> {
    if !identity.may_access(id) {
        return Err(ApiError::NotFound("server".into()));
    }
    state
        .store
        .server(id)
        .await
        .map(|record| record.config.directory)
        .ok_or_else(|| ApiError::NotFound("server".into()))
}

async fn list(
    State(state): State<Arc<AppState>>,
    identity: Identity,
    Path(id): Path<String>,
) -> ApiResult<Json<Vec<StoredBackup>>> {
    authorized(&state, &identity, &id).await?;
    let backups = state.store.read().await.backups.clone();
    let mut out: Vec<StoredBackup> = backups.into_iter().filter(|b| b.server_id == id).collect();
    out.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Ok(Json(out))
}

/// Body of `POST /api/servers/:id/backups`.
#[derive(Deserialize, Default)]
pub struct CreateBackup {
    #[serde(default)]
    note: String,
}

async fn create(
    State(state): State<Arc<AppState>>,
    identity: Identity,
    Path(id): Path<String>,
    body: Option<Json<CreateBackup>>,
) -> ApiResult<Json<StoredBackup>> {
    let _ = authorized(&state, &identity, &id).await?;
    let note = body.map(|Json(b)| b.note).unwrap_or_default();
    if note.len() > 1024 || note.contains(['\0', '\r', '\n']) {
        return Err(ApiError::BadRequest(
            "backup note is too long or contains control characters".into(),
        ));
    }
    let (server, _resource_lock) = state.server_resource_lock(&id).await?;
    let backup_lock = state.backup_lock(&id).await;
    let _backup_guard = backup_lock.lock().await;
    let directory = authorized(&state, &identity, &id).await?;
    let record = state
        .store
        .server(&id)
        .await
        .ok_or_else(|| ApiError::NotFound("server".into()))?;

    let backup_dir = state.backup_dir(&id);
    crate::state::secure_directory(&backup_dir)
        .await
        .map_err(ApiError::Internal)?;
    let staging_dir = state.staging_dir();
    crate::state::secure_directory(&staging_dir)
        .await
        .map_err(ApiError::Internal)?;

    let quota_fs = crate::filesystem::open(backup_dir.clone())
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    let backup_usage = tokio::task::spawn_blocking(move || quota_fs.directory_size("."))
        .await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("backup quota check failed: {e}")))?
        .map_err(|e| ApiError::Internal(e.into()))?;
    const METADATA_RESERVE: u64 = 64 * 1024;
    let available = state
        .limits
        .max_backup_disk_bytes
        .saturating_sub(backup_usage)
        .saturating_sub(METADATA_RESERVE);
    if available == 0 {
        return Err(ApiError::Conflict("backup storage quota exceeded".into()));
    }

    // Minecraft consistency protocol
    let mut needs_save_on = false;
    let save_result = if server.status().await.is_running() {
        let core = server.config().await.core.to_ascii_lowercase();
        let is_proxy = core == "velocity" || core == "waterfall" || core == "bungeecord";
        if is_proxy {
            Ok(())
        } else {
            let mut events = server.subscribe();
            if let Err(e) = server.command("save-off").await {
                Err(ApiError::Internal(anyhow::anyhow!(
                    "failed to disable saving: {e}"
                )))
            } else {
                needs_save_on = true;
                if let Err(e) = server.command("save-all flush").await {
                    let _ = server.command("save-on").await;
                    needs_save_on = false;
                    Err(ApiError::Internal(anyhow::anyhow!("failed to flush: {e}")))
                } else {
                    let ack = tokio::time::timeout(std::time::Duration::from_secs(10), async {
                        loop {
                            match events.recv().await {
                                Ok(guardian::ServerEvent::Console(line)) => {
                                    let text = &line.line;
                                    if text.contains("Saved the game")
                                        || text.contains("Saved the world")
                                        || text.contains("All dimensions are saved")
                                    {
                                        return Ok::<(), anyhow::Error>(());
                                    }
                                }
                                Ok(_) => continue,
                                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                                    continue
                                }
                                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                                    return Err(anyhow::anyhow!("event channel closed"));
                                }
                            }
                        }
                    })
                    .await;
                    match ack {
                        Ok(Ok(())) => Ok(()),
                        Ok(Err(e)) => {
                            let _ = server.command("save-on").await;
                            needs_save_on = false;
                            Err(ApiError::Internal(e))
                        }
                        Err(_) => {
                            let _ = server.command("save-on").await;
                            needs_save_on = false;
                            Err(ApiError::Internal(anyhow::anyhow!(
                                "timeout waiting for save ack"
                            )))
                        }
                    }
                }
            }
        }
    } else {
        Ok(())
    };
    if let Err(e) = save_result {
        return Err(e);
    }

    // Create staging archive while save-off (if applicable)
    let created_at = time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap();
    // Use a temporary backup dir as staging: create backup there, then use its file as artifact
    let tmp_id = Uuid::new_v4().simple().to_string();
    let tmp_backup_dir = staging_dir.clone();
    let dir_for_create = directory.clone();
    let note_for_create = note.clone();
    let backup_meta = guardian::backup::create_with_limit(
        &dir_for_create,
        &tmp_backup_dir,
        note_for_create,
        available,
    )
    .await
    .map_err(|e| ApiError::Internal(e.into()))?;
    // The file is at tmp_backup_dir/<backup_meta.id>.tar.gz
    let src_path = tmp_backup_dir.join(format!("{}.tar.gz", backup_meta.id));
    let staging_path = staging_dir.join(format!("{}.tar.gz", backup_meta.id));
    // Move to staging_path with correct id (already there, but ensure)
    if src_path != staging_path {
        let _ = tokio::fs::rename(&src_path, &staging_path).await;
    }
    let _ = tokio::fs::remove_file(tmp_backup_dir.join(format!("{}.json", backup_meta.id))).await;
    let size_bytes = backup_meta.size;
    // Compute checksum
    let checksum = {
        use sha2::{Digest, Sha256};
        use tokio::io::AsyncReadExt;
        let mut file = tokio::fs::File::open(&staging_path)
            .await
            .map_err(|e| ApiError::Internal(e.into()))?;
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; 8192];
        loop {
            let n = file
                .read(&mut buf)
                .await
                .map_err(|e| ApiError::Internal(e.into()))?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
        Some(format!("{:x}", hasher.finalize()))
    };
    if needs_save_on {
        let _ = server.command("save-on").await;
    }

    let artifact = crate::backups::provider::BackupArtifact {
        id: backup_meta.id.clone(),
        server_id: id.clone(),
        note: note.clone(),
        staging_path: staging_path.clone(),
        size_bytes,
        checksum_sha256: checksum.clone(),
        created_at: created_at.clone(),
    };

    let provider = crate::backups::provider_for(&state, &record)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    let effective_kind = if let Some(policy) = &record.backup_policy {
        policy.provider.clone()
    } else {
        state.store.read().await.backup_storage.provider.clone()
    };

    let remote = provider.upload(&id, &artifact).await.map_err(|e| {
        let _ = std::fs::remove_file(&staging_path);
        e
    })?;

    let stored = StoredBackup {
        id: artifact.id.clone(),
        server_id: id.clone(),
        provider: effective_kind.clone(),
        remote_id: remote.remote_id.clone(),
        created_at: remote.created_at.clone(),
        size_bytes: remote.size_bytes,
        checksum_sha256: checksum.clone(),
        note: note.clone(),
    };
    state
        .store
        .update(|data| {
            data.backups.push(stored.clone());
        })
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;

    // Retention
    let retention = crate::backups::retention_for(&state, &record).await;
    let all_for_server: Vec<StoredBackup> = {
        let data = state.store.read().await;
        data.backups
            .iter()
            .filter(|b| b.server_id == id)
            .cloned()
            .collect()
    };
    let to_delete = plan_retention(&all_for_server, &retention, &stored.id);
    for del_id in to_delete {
        if del_id == stored.id {
            continue;
        }
        let del_provider = crate::backups::provider_for(&state, &record)
            .await
            .unwrap_or_else(|_| provider.clone());
        if let Err(e) = del_provider.delete(&id, &del_id).await {
            tracing::warn!(server = %id, backup = %del_id, error = %e, "retention delete failed");
            continue;
        }
        let _ = state
            .store
            .update(|data| {
                data.backups.retain(|b| b.id != del_id);
            })
            .await;
    }

    if effective_kind != crate::store::BackupProviderKind::Local {
        let _ = tokio::fs::remove_file(&staging_path).await;
    } else {
        let _ = tokio::fs::remove_file(&staging_path).await;
    }

    tracing::info!(server = %id, backup = %stored.id, provider = %stored.provider, "backup created");
    Ok(Json(stored))
}

async fn restore(
    State(state): State<Arc<AppState>>,
    identity: Identity,
    Path((id, backup)): Path<(String, String)>,
) -> ApiResult<Json<serde_json::Value>> {
    let _ = authorized(&state, &identity, &id).await?;
    let (server, _resource_lock) = state.server_resource_lock(&id).await?;
    let backup_lock = state.backup_lock(&id).await;
    let _backup_guard = backup_lock.lock().await;
    let directory = authorized(&state, &identity, &id).await?;
    if server.status().await.is_running()
        || server.status().await == guardian::ServerStatus::Preparing
    {
        return Err(ApiError::BadRequest(
            "stop the server before restoring a backup".into(),
        ));
    }
    let record = state
        .store
        .server(&id)
        .await
        .ok_or_else(|| ApiError::NotFound("server".into()))?;
    let stored = {
        let data = state.store.read().await;
        data.backups
            .iter()
            .find(|b| b.server_id == id && b.id == backup)
            .cloned()
    };
    let Some(stored) = stored else {
        return Err(ApiError::NotFound("backup".into()));
    };
    let provider = crate::backups::provider_for(&state, &record)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    let tmp_path = state
        .staging_dir()
        .join(format!(".restore-{}-{}.tar.gz", id, backup));
    crate::state::secure_directory(&state.staging_dir())
        .await
        .map_err(ApiError::Internal)?;
    let mut stream = provider.download(&id, &backup).await?;
    let mut file = tokio::fs::File::create(&tmp_path)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    tokio::io::copy(&mut stream, &mut file)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    file.sync_all()
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    drop(file);
    if let Some(expected) = &stored.checksum_sha256 {
        use sha2::{Digest, Sha256};
        use tokio::io::AsyncReadExt;
        let mut f = tokio::fs::File::open(&tmp_path)
            .await
            .map_err(|e| ApiError::Internal(e.into()))?;
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; 8192];
        loop {
            let n = f
                .read(&mut buf)
                .await
                .map_err(|e| ApiError::Internal(e.into()))?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
        let actual = format!("{:x}", hasher.finalize());
        if &actual != expected {
            let _ = tokio::fs::remove_file(&tmp_path).await;
            return Err(ApiError::Internal(anyhow::anyhow!("checksum mismatch")))?;
        }
    }
    let tmp_backup_dir = state
        .staging_dir()
        .join(format!(".restore-dir-{}-{}", id, backup));
    tokio::fs::create_dir_all(&tmp_backup_dir)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    let dest_archive = tmp_backup_dir.join(format!("{backup}.tar.gz"));
    tokio::fs::copy(&tmp_path, &dest_archive)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    guardian::backup::restore_with_limits_and_quota(
        &tmp_backup_dir,
        &backup,
        &directory,
        state.limits.archive_limits(),
        state.limits.max_server_disk_bytes,
    )
    .await
    .map_err(|e| ApiError::Internal(e.into()))?;
    let _ = tokio::fs::remove_file(&tmp_path).await;
    let _ = tokio::fs::remove_dir_all(&tmp_backup_dir).await;
    tracing::info!(server = %id, backup = %backup, "backup restored");
    Ok(Json(serde_json::json!({ "ok": true })))
}

async fn delete(
    State(state): State<Arc<AppState>>,
    identity: Identity,
    Path((id, backup)): Path<(String, String)>,
) -> ApiResult<Json<serde_json::Value>> {
    let _ = authorized(&state, &identity, &id).await?;
    let (_server, _resource_lock) = state.server_resource_lock(&id).await?;
    let backup_lock = state.backup_lock(&id).await;
    let _backup_guard = backup_lock.lock().await;
    authorized(&state, &identity, &id).await?;
    let record = state
        .store
        .server(&id)
        .await
        .ok_or_else(|| ApiError::NotFound("server".into()))?;
    let provider = crate::backups::provider_for(&state, &record)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    provider.delete(&id, &backup).await?;
    state
        .store
        .update(|data| {
            data.backups
                .retain(|b| !(b.server_id == id && b.id == backup));
        })
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

/// Issue a short-lived grant for one backup archive.
async fn download_ticket(
    State(state): State<Arc<AppState>>,
    identity: Identity,
    Path((id, backup)): Path<(String, String)>,
) -> ApiResult<Json<serde_json::Value>> {
    authorized(&state, &identity, &id).await?;
    let data = state.store.read().await;
    let exists = data
        .backups
        .iter()
        .any(|b| b.server_id == id && b.id == backup);
    if !exists {
        let backup_dir = state.backup_dir(&id);
        let backup_clone = backup.clone();
        let exists_fs = tokio::task::spawn_blocking(move || {
            let fs = guardian::ScopedFs::open(backup_dir).ok()?;
            fs.metadata(format!("{backup_clone}.tar.gz"))
                .ok()
                .map(|m| m.is_file)
        })
        .await
        .ok()
        .flatten()
        .unwrap_or(false);
        if !exists_fs {
            return Err(ApiError::NotFound("backup".into()));
        }
    }
    let ticket = state.tickets.issue(Resource::Backup { server: id, backup });
    Ok(Json(serde_json::json!({ "ticket": ticket })))
}

/// `?ticket=` on the download route.
#[derive(Deserialize)]
pub struct TicketQuery {
    ticket: String,
}

/// Stream an archive out for a browser navigation, authorised by a ticket.
async fn download(
    State(state): State<Arc<AppState>>,
    Path((id, backup)): Path<(String, String)>,
    axum::extract::Query(query): axum::extract::Query<TicketQuery>,
) -> ApiResult<Response> {
    let download_slot = state
        .download_slots
        .clone()
        .try_acquire_owned()
        .map_err(|_| ApiError::TooManyRequests)?;
    let granted = state
        .tickets
        .redeem(&query.ticket)
        .ok_or(ApiError::Unauthorized)?;

    let Resource::Backup {
        server,
        backup: granted_backup,
    } = granted
    else {
        return Err(ApiError::Unauthorized);
    };
    if server != id || granted_backup != backup {
        return Err(ApiError::Unauthorized);
    }

    let stored = {
        let data = state.store.read().await;
        data.backups
            .iter()
            .find(|b| b.server_id == id && b.id == backup)
            .cloned()
    };
    let record = state
        .store
        .server(&id)
        .await
        .ok_or_else(|| ApiError::NotFound("server".into()))?;
    let provider = crate::backups::provider_for(&state, &record)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;

    if let Some(_stored) = stored {
        if let Ok(stream) = provider.download(&id, &backup).await {
            let stream = ReaderStream::new(stream).map(move |chunk| {
                let _keep_slot = &download_slot;
                chunk
            });
            return Ok((
                StatusCode::OK,
                [
                    (header::CONTENT_TYPE, "application/gzip".to_string()),
                    (
                        header::CONTENT_DISPOSITION,
                        format!("attachment; filename=\"{backup}.tar.gz\""),
                    ),
                ],
                Body::from_stream(stream),
            )
                .into_response());
        }
    }

    let fs = crate::filesystem::open(state.backup_dir(&id))
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    let archive_name = format!("{backup}.tar.gz");
    let file = tokio::task::spawn_blocking(move || fs.open_file(&archive_name))
        .await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("backup open failed: {e}")))?
        .map_err(|_| ApiError::NotFound("backup".into()))?;
    let file = tokio::fs::File::from_std(file);
    let stream = ReaderStream::new(file).map(move |chunk| {
        let _keep_slot = &download_slot;
        chunk
    });
    Ok((
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/gzip".to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{backup}.tar.gz\""),
            ),
        ],
        Body::from_stream(stream),
    )
        .into_response())
}

/// Routes nested under `/api/servers`.
pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/{id}/backups", get(list).post(create))
        .route("/{id}/backups/{backup}", axum::routing::delete(delete))
        .route("/{id}/backups/{backup}/restore", post(restore))
        .route("/{id}/backups/{backup}/download", get(download))
        .route("/{id}/backups/{backup}/ticket", post(download_ticket))
}
