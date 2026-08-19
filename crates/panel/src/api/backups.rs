//! Backup creation, listing, download, restore and deletion.

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use std::sync::Arc;
use tokio_util::io::ReaderStream;

use crate::auth::Identity;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

/// Resolve a server the caller may act on.
async fn authorized(state: &AppState, identity: &Identity, id: &str) -> ApiResult<std::path::PathBuf> {
    if !identity.may_access(id) {
        return Err(ApiError::Forbidden);
    }
    state
        .store
        .server(id)
        .await
        .map(|record| record.config.directory)
        .ok_or_else(|| ApiError::NotFound(format!("server {id}")))
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

    // Taking a backup while chunks are being written produces a torn world, so
    // flush first and let the server keep running.
    let server = state.guardian(&id).await?;
    if server.status().await.is_running() {
        let _ = server.command("save-all flush").await;
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }

    let backup = guardian::backup::create(&directory, &state.backup_dir(&id), note).await?;
    tracing::info!(server = %id, backup = %backup.id, "backup created");
    Ok(Json(backup))
}

async fn restore(
    State(state): State<Arc<AppState>>,
    identity: Identity,
    Path((id, backup)): Path<(String, String)>,
) -> ApiResult<Json<serde_json::Value>> {
    let directory = authorized(&state, &identity, &id).await?;

    // Unpacking a world under a live JVM corrupts it, so this is a hard error
    // rather than an implicit stop the operator did not ask for.
    if state.guardian(&id).await?.status().await.is_running() {
        return Err(ApiError::BadRequest(
            "stop the server before restoring a backup".into(),
        ));
    }

    guardian::backup::restore(&state.backup_dir(&id), &backup, &directory).await?;
    tracing::info!(server = %id, backup = %backup, "backup restored");
    Ok(Json(serde_json::json!({ "ok": true })))
}

async fn delete(
    State(state): State<Arc<AppState>>,
    identity: Identity,
    Path((id, backup)): Path<(String, String)>,
) -> ApiResult<Json<serde_json::Value>> {
    authorized(&state, &identity, &id).await?;
    guardian::backup::delete(&state.backup_dir(&id), &backup).await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

async fn download(
    State(state): State<Arc<AppState>>,
    identity: Identity,
    Path((id, backup)): Path<(String, String)>,
) -> ApiResult<Response> {
    authorized(&state, &identity, &id).await?;

    let path = guardian::backup::path_of(&state.backup_dir(&id), &backup)?;
    let file = tokio::fs::File::open(&path)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;

    // Streamed rather than buffered: a world backup can be gigabytes.
    let stream = ReaderStream::new(file);

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
}
