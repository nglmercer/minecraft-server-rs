//! Backup creation, listing, download, restore and deletion.

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

use crate::auth::Identity;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
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
) -> ApiResult<Json<Vec<guardian::Backup>>> {
    authorized(&state, &identity, &id).await?;
    Ok(Json(guardian::backup::list(&state.backup_dir(&id)).await?))
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
) -> ApiResult<Json<guardian::Backup>> {
    let directory = authorized(&state, &identity, &id).await?;
    let note = body.map(|Json(b)| b.note).unwrap_or_default();
    if note.len() > 1024 || note.contains(['\0', '\r', '\n']) {
        return Err(ApiError::BadRequest(
            "backup note is too long or contains control characters".into(),
        ));
    }
    let _resource_lock = state.resource_lock.lock().await;

    let backup_dir = state.backup_dir(&id);
    crate::state::secure_directory(&backup_dir)
        .await
        .map_err(ApiError::Internal)?;
    let quota_fs = crate::filesystem::open(backup_dir.clone()).await?;
    let backup_usage = tokio::task::spawn_blocking(move || quota_fs.directory_size("."))
        .await
        .map_err(|error| ApiError::Internal(anyhow::anyhow!("backup quota check failed: {error}")))?
        .map_err(|error| ApiError::Internal(error.into()))?;
    const METADATA_RESERVE: u64 = 64 * 1024;
    let available = state
        .limits
        .max_backup_disk_bytes
        .saturating_sub(backup_usage)
        .saturating_sub(METADATA_RESERVE);
    if available == 0 {
        return Err(ApiError::Conflict("backup storage quota exceeded".into()));
    }

    // Taking a backup while chunks are being written produces a torn world, so
    // flush first and let the server keep running.
    let server = state.guardian(&id).await?;
    if server.status().await.is_running() {
        let _ = server.command("save-all flush").await;
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }

    let backup =
        guardian::backup::create_with_limit(&directory, &backup_dir, note, available).await?;
    let usage_fs = crate::filesystem::open(backup_dir.clone()).await?;
    let usage = tokio::task::spawn_blocking(move || usage_fs.directory_size("."))
        .await
        .map_err(|error| ApiError::Internal(anyhow::anyhow!("backup quota check failed: {error}")))?
        .map_err(|error| ApiError::Internal(error.into()))?;
    if usage > state.limits.max_backup_disk_bytes {
        let _ = guardian::backup::delete(&backup_dir, &backup.id).await;
        return Err(ApiError::Conflict("backup storage quota exceeded".into()));
    }
    tracing::info!(server = %id, backup = %backup.id, "backup created");
    Ok(Json(backup))
}

async fn restore(
    State(state): State<Arc<AppState>>,
    identity: Identity,
    Path((id, backup)): Path<(String, String)>,
) -> ApiResult<Json<serde_json::Value>> {
    let _server_lock = state.server_mutation_lock.lock().await;
    let directory = authorized(&state, &identity, &id).await?;

    // Unpacking a world under a live JVM corrupts it, so this is a hard error
    // rather than an implicit stop the operator did not ask for.
    if state.guardian(&id).await?.status().await.is_running() {
        return Err(ApiError::BadRequest(
            "stop the server before restoring a backup".into(),
        ));
    }

    let _resource_lock = state.resource_lock.lock().await;
    guardian::backup::restore_with_limits(
        &state.backup_dir(&id),
        &backup,
        &directory,
        state.limits.archive_limits(),
    )
    .await?;
    tracing::info!(server = %id, backup = %backup, "backup restored");
    Ok(Json(serde_json::json!({ "ok": true })))
}

async fn delete(
    State(state): State<Arc<AppState>>,
    identity: Identity,
    Path((id, backup)): Path<(String, String)>,
) -> ApiResult<Json<serde_json::Value>> {
    let _resource_lock = state.resource_lock.lock().await;
    authorized(&state, &identity, &id).await?;
    guardian::backup::delete(&state.backup_dir(&id), &backup).await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

/// Issue a short-lived grant for one backup archive.
async fn download_ticket(
    State(state): State<Arc<AppState>>,
    identity: Identity,
    Path((id, backup)): Path<(String, String)>,
) -> ApiResult<Json<serde_json::Value>> {
    authorized(&state, &identity, &id).await?;
    let backup_fs = crate::filesystem::open(state.backup_dir(&id)).await?;
    let archive_name = format!("{backup}.tar.gz");
    let exists = tokio::task::spawn_blocking(move || {
        backup_fs
            .metadata(&archive_name)
            .map(|metadata| metadata.is_file)
    })
    .await
    .map_err(|error| ApiError::Internal(anyhow::anyhow!("backup check failed: {error}")))?
    .map_err(|_| ApiError::NotFound("backup".into()))?;
    if !exists {
        return Err(ApiError::NotFound("backup".into()));
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

    let fs = crate::filesystem::open(state.backup_dir(&id)).await?;
    let archive_name = format!("{backup}.tar.gz");
    let file = tokio::task::spawn_blocking(move || fs.open_file(&archive_name))
        .await
        .map_err(|error| ApiError::Internal(anyhow::anyhow!("backup open failed: {error}")))?
        .map_err(|_| ApiError::NotFound("backup".into()))?;
    let file = tokio::fs::File::from_std(file);

    // Streamed rather than buffered: a world backup can be gigabytes.
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
