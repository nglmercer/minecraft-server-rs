//! A server-scoped file manager backed by guardian's directory capability.

use axum::body::Body;
use axum::extract::{DefaultBodyLimit, Multipart, Path as AxumPath, Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::StreamExt;
use guardian::ScopedFs;
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio_util::io::ReaderStream;
use uuid::Uuid;

use crate::auth::Identity;
use crate::error::{ApiError, ApiResult};
use crate::filesystem::{display_path, filename, open, relative};
use crate::limits::{ResourceLimits, MAX_EDIT_BODY_BYTES};
use crate::state::AppState;
use crate::tickets::Resource;

/// One directory entry.
#[derive(Serialize)]
pub struct Entry {
    name: String,
    path: String,
    directory: bool,
    size: u64,
    modified: Option<u64>,
}

/// Query used by path-oriented file endpoints.
#[derive(Deserialize)]
pub struct PathQuery {
    #[serde(default)]
    path: String,
}

async fn root_for(state: &AppState, identity: &Identity, id: &str) -> ApiResult<PathBuf> {
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

fn task_error(error: tokio::task::JoinError) -> ApiError {
    ApiError::Internal(anyhow::anyhow!("filesystem task failed: {error}"))
}

fn io_error(error: std::io::Error) -> ApiError {
    if error.kind() == std::io::ErrorKind::NotFound {
        ApiError::NotFound("path".into())
    } else {
        ApiError::Internal(error.into())
    }
}

async fn list(
    State(state): State<Arc<AppState>>,
    identity: Identity,
    AxumPath(id): AxumPath<String>,
    Query(query): Query<PathQuery>,
) -> ApiResult<Json<Vec<Entry>>> {
    let root = root_for(&state, &identity, &id).await?;
    let path = relative(&query.path)?;
    let fs = open(root).await?;
    let lookup_path = path.clone();
    let entries = tokio::task::spawn_blocking(move || fs.read_dir(&lookup_path))
        .await
        .map_err(task_error)?
        .map_err(io_error)?;

    let mut entries = entries
        .into_iter()
        .map(|(name, metadata)| {
            let path = path.join(&name);
            Entry {
                name: name.to_string_lossy().into_owned(),
                path: display_path(&path),
                directory: metadata.is_dir,
                size: metadata.len,
                modified: metadata.modified_epoch_secs,
            }
        })
        .collect::<Vec<_>>();
    entries.sort_by(|a, b| b.directory.cmp(&a.directory).then(a.name.cmp(&b.name)));
    Ok(Json(entries))
}

/// The contents of a text file.
#[derive(Serialize)]
pub struct FileContents {
    path: String,
    content: String,
}

async fn read(
    State(state): State<Arc<AppState>>,
    identity: Identity,
    AxumPath(id): AxumPath<String>,
    Query(query): Query<PathQuery>,
) -> ApiResult<Json<FileContents>> {
    let root = root_for(&state, &identity, &id).await?;
    let path = relative(&query.path)?;
    if path.as_os_str().is_empty() {
        return Err(ApiError::BadRequest("a file path is required".into()));
    }

    let fs = open(root).await?;
    let lookup_path = path.clone();
    let bytes = tokio::task::spawn_blocking(move || -> ApiResult<Vec<u8>> {
        let file = fs.open_file(&lookup_path).map_err(io_error)?;
        let metadata = file.metadata().map_err(io_error)?;
        if metadata.is_dir() {
            return Err(ApiError::BadRequest("that is a directory".into()));
        }
        if metadata.len() > MAX_EDIT_BODY_BYTES as u64 {
            return Err(ApiError::BadRequest(format!(
                "file is too large; the editor handles up to {MAX_EDIT_BODY_BYTES} bytes"
            )));
        }
        let mut bytes = Vec::with_capacity(metadata.len() as usize);
        file.take(MAX_EDIT_BODY_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(io_error)?;
        if bytes.len() > MAX_EDIT_BODY_BYTES {
            return Err(ApiError::BadRequest(format!(
                "file is too large; the editor handles up to {MAX_EDIT_BODY_BYTES} bytes"
            )));
        }
        Ok(bytes)
    })
    .await
    .map_err(task_error)??;
    let content = String::from_utf8(bytes)
        .map_err(|_| ApiError::BadRequest("file is not valid UTF-8".into()))?;

    Ok(Json(FileContents {
        path: display_path(&path),
        content,
    }))
}

/// Body of the file editor endpoint.
#[derive(Deserialize)]
pub struct WriteRequest {
    path: String,
    content: String,
}

async fn write(
    State(state): State<Arc<AppState>>,
    identity: Identity,
    AxumPath(id): AxumPath<String>,
    Json(body): Json<WriteRequest>,
) -> ApiResult<Json<serde_json::Value>> {
    let root = root_for(&state, &identity, &id).await?;
    let path = relative(&body.path)?;
    if path.as_os_str().is_empty() {
        return Err(ApiError::BadRequest("a file path is required".into()));
    }
    let bytes = body.content.into_bytes();
    if bytes.len() > MAX_EDIT_BODY_BYTES {
        return Err(ApiError::BadRequest(format!(
            "file is too large; the editor accepts up to {MAX_EDIT_BODY_BYTES} bytes"
        )));
    }

    let _resource_lock = state.resource_lock.lock().await;
    let fs = open(root).await?;
    let quota_fs = fs.try_clone().map_err(io_error)?;
    let quota_path = path.clone();
    let (existing, replaced) = tokio::task::spawn_blocking(move || {
        let existing = quota_fs.directory_size(".")?;
        let replaced = quota_fs
            .metadata(&quota_path)
            .ok()
            .filter(|metadata| metadata.is_file)
            .map(|metadata| metadata.len)
            .unwrap_or(0);
        Ok::<_, std::io::Error>((existing, replaced))
    })
    .await
    .map_err(task_error)?
    .map_err(io_error)?;
    let new_usage = existing
        .checked_sub(replaced)
        .and_then(|base| base.checked_add(bytes.len() as u64))
        .ok_or_else(|| ApiError::Conflict("server storage quota exceeded".into()))?;
    if new_usage > state.limits.max_server_disk_bytes {
        return Err(ApiError::Conflict("server storage quota exceeded".into()));
    }

    tokio::task::spawn_blocking(move || -> ApiResult<()> {
        if let Some(parent) = path.parent() {
            fs.create_dir_all(parent).map_err(io_error)?;
        }
        fs.write_atomic(&path, &bytes).map_err(io_error)
    })
    .await
    .map_err(task_error)??;

    Ok(Json(serde_json::json!({ "ok": true })))
}

async fn delete(
    State(state): State<Arc<AppState>>,
    identity: Identity,
    AxumPath(id): AxumPath<String>,
    Query(query): Query<PathQuery>,
) -> ApiResult<Json<serde_json::Value>> {
    let root = root_for(&state, &identity, &id).await?;
    let path = relative(&query.path)?;
    if path.as_os_str().is_empty() {
        return Err(ApiError::BadRequest(
            "refusing to delete the server root".into(),
        ));
    }
    let fs = open(root).await?;
    tokio::task::spawn_blocking(move || fs.remove(&path))
        .await
        .map_err(task_error)?
        .map_err(io_error)?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

/// Body of the directory creation endpoint.
#[derive(Deserialize)]
pub struct MkdirRequest {
    path: String,
}

async fn mkdir(
    State(state): State<Arc<AppState>>,
    identity: Identity,
    AxumPath(id): AxumPath<String>,
    Json(body): Json<MkdirRequest>,
) -> ApiResult<Json<serde_json::Value>> {
    let root = root_for(&state, &identity, &id).await?;
    let path = relative(&body.path)?;
    let fs = open(root).await?;
    tokio::task::spawn_blocking(move || fs.create_dir_all(&path))
        .await
        .map_err(task_error)?
        .map_err(io_error)?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

/// The measured size of one subdirectory.
#[derive(Serialize)]
pub struct DirectorySize {
    path: String,
    bytes: u64,
}

async fn sizes(
    State(state): State<Arc<AppState>>,
    identity: Identity,
    AxumPath(id): AxumPath<String>,
    Query(query): Query<PathQuery>,
) -> ApiResult<Json<Vec<DirectorySize>>> {
    let _resource_lock = state.resource_lock.lock().await;
    let root = root_for(&state, &identity, &id).await?;
    let path = relative(&query.path)?;
    let fs = open(root).await?;
    let values = tokio::task::spawn_blocking(move || -> std::io::Result<Vec<DirectorySize>> {
        let entries = fs.read_dir(&path)?;
        let mut values = Vec::new();
        for (name, metadata) in entries {
            if metadata.is_dir {
                let child = path.join(&name);
                values.push(DirectorySize {
                    path: display_path(&child),
                    bytes: fs.directory_size(&child)?,
                });
            }
        }
        Ok(values)
    })
    .await
    .map_err(task_error)?
    .map_err(io_error)?;
    Ok(Json(values))
}

/// Issue a short-lived grant for one file, for the browser to redeem.
async fn download_ticket(
    State(state): State<Arc<AppState>>,
    identity: Identity,
    AxumPath(id): AxumPath<String>,
    Query(query): Query<PathQuery>,
) -> ApiResult<Json<serde_json::Value>> {
    let root = root_for(&state, &identity, &id).await?;
    let path = relative(&query.path)?;
    let fs = open(root).await?;
    let lookup_path = path.clone();
    let is_file = tokio::task::spawn_blocking(move || {
        fs.metadata(&lookup_path).map(|metadata| metadata.is_file)
    })
    .await
    .map_err(task_error)?
    .map_err(io_error)?;
    if !is_file {
        return Err(ApiError::NotFound("file".into()));
    }

    let ticket = state.tickets.issue(Resource::File {
        server: id,
        path: display_path(&path),
    });
    Ok(Json(serde_json::json!({ "ticket": ticket })))
}

/// Query used by the ticketed download route.
#[derive(Deserialize)]
pub struct TicketQuery {
    ticket: String,
}

async fn download(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
    Query(query): Query<TicketQuery>,
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
    let Resource::File { server, path } = granted else {
        return Err(ApiError::Unauthorized);
    };
    if server != id {
        return Err(ApiError::Unauthorized);
    }

    let root = state
        .store
        .server(&id)
        .await
        .map(|record| record.config.directory)
        .ok_or_else(|| ApiError::NotFound("server".into()))?;
    let path = relative(&path)?;
    let fs = open(root).await?;
    let (file, name) =
        tokio::task::spawn_blocking(move || -> ApiResult<(std::fs::File, String)> {
            let file = fs.open_file(&path).map_err(io_error)?;
            let metadata = file.metadata().map_err(io_error)?;
            if !metadata.is_file() {
                return Err(ApiError::NotFound("file".into()));
            }
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| ApiError::BadRequest("invalid file name".into()))?;
            filename(name)?;
            Ok((file, name.to_owned()))
        })
        .await
        .map_err(task_error)??;

    let file = tokio::fs::File::from_std(file);
    let stream = ReaderStream::new(file).map(move |chunk| {
        // Keep the permit alive for the complete response stream, not merely
        // until the handler returns after constructing the response.
        let _keep_slot = &download_slot;
        chunk
    });
    Ok((
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/octet-stream".to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{name}\""),
            ),
        ],
        Body::from_stream(stream),
    )
        .into_response())
}

/// Accept one or more uploaded files into a server directory.
async fn upload(
    State(state): State<Arc<AppState>>,
    identity: Identity,
    AxumPath(id): AxumPath<String>,
    Query(query): Query<PathQuery>,
    mut multipart: Multipart,
) -> ApiResult<Json<serde_json::Value>> {
    let root = root_for(&state, &identity, &id).await?;
    let directory = relative(&query.path)?;
    let _resource_lock = state.resource_lock.lock().await;
    let fs = open(root).await?;
    let quota_fs = fs.try_clone().map_err(io_error)?;
    let existing = tokio::task::spawn_blocking(move || quota_fs.directory_size("."))
        .await
        .map_err(task_error)?
        .map_err(io_error)?;
    if existing > state.limits.max_server_disk_bytes {
        return Err(ApiError::Conflict("server storage quota exceeded".into()));
    }

    fs.create_dir_all(&directory).map_err(io_error)?;
    let mut written = Vec::new();
    let mut total_written = 0_u64;
    let mut replaced_bytes = 0_u64;
    let mut replaced = std::collections::HashSet::new();

    while let Some(mut field) = multipart
        .next_field()
        .await
        .map_err(crate::filesystem::multipart_error)?
    {
        let Some(raw_name) = field.file_name().map(str::to_owned) else {
            continue;
        };
        let name = filename(&raw_name)?;
        let relative_path = directory.join(name);
        let old_size = if replaced.insert(relative_path.clone()) {
            fs.metadata(&relative_path)
                .ok()
                .filter(|metadata| metadata.is_file)
                .map(|metadata| metadata.len)
                .unwrap_or(0)
        } else {
            0
        };
        replaced_bytes = replaced_bytes.saturating_add(old_size);
        let temporary_path = directory.join(format!(".mcpanel-upload-{}", Uuid::new_v4().simple()));
        let output = fs.create_new_file(&temporary_path).map_err(io_error)?;
        let mut output = tokio::fs::File::from_std(output);
        let result: ApiResult<u64> = async {
            let mut field_bytes = 0_u64;
            while let Some(chunk) = field
                .chunk()
                .await
                .map_err(crate::filesystem::multipart_error)?
            {
                let next_field = field_bytes
                    .checked_add(chunk.len() as u64)
                    .ok_or_else(|| ApiError::Conflict("upload exceeds its size limit".into()))?;
                let next_total = total_written
                    .checked_add(next_field)
                    .ok_or_else(|| ApiError::Conflict("upload exceeds its size limit".into()))?;
                let next_usage = existing
                    .checked_add(next_total)
                    .ok_or_else(|| ApiError::Conflict("server storage quota exceeded".into()))?;
                if next_field > state.limits.max_upload_bytes
                    || next_total > state.limits.max_upload_bytes
                    || next_usage.saturating_sub(replaced_bytes)
                        > state.limits.max_server_disk_bytes
                {
                    return Err(ApiError::Conflict("server storage quota exceeded".into()));
                }
                output
                    .write_all(&chunk)
                    .await
                    .map_err(|error| ApiError::Internal(error.into()))?;
                field_bytes = next_field;
            }
            output
                .flush()
                .await
                .map_err(|error| ApiError::Internal(error.into()))?;
            output
                .sync_data()
                .await
                .map_err(|error| ApiError::Internal(error.into()))?;
            Ok(field_bytes)
        }
        .await;
        drop(output);

        let field_bytes = match result {
            Ok(bytes) => bytes,
            Err(error) => {
                let _ = fs.remove(&temporary_path);
                return Err(error);
            }
        };
        if let Err(error) = fs.replace_file(&temporary_path, &relative_path) {
            let _ = fs.remove(&temporary_path);
            return Err(io_error(error));
        }
        total_written = total_written.saturating_add(field_bytes);
        written.push(display_path(&relative_path));
    }

    if written.is_empty() {
        return Err(ApiError::BadRequest("no files in the upload".into()));
    }
    Ok(Json(serde_json::json!({ "ok": true, "written": written })))
}

/// Body of the archive extraction endpoint.
#[derive(Deserialize)]
pub struct ExtractRequest {
    path: String,
    #[serde(default)]
    into: Option<String>,
}

fn safe_member(path: &Path) -> bool {
    let raw = path.to_string_lossy();
    let windows_drive =
        raw.len() >= 2 && raw.as_bytes()[0].is_ascii_alphabetic() && raw.as_bytes()[1] == b':';
    !raw.starts_with('/')
        && !raw.starts_with('\\')
        && !raw.contains('\\')
        && !windows_drive
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_) | Component::CurDir))
}

fn normalise_member(path: &Path) -> Option<PathBuf> {
    let mut output = PathBuf::new();
    for component in path.components() {
        if let Component::Normal(name) = component {
            output.push(name);
        }
    }
    (!output.as_os_str().is_empty()).then_some(output)
}

struct Extraction<'a> {
    fs: &'a ScopedFs,
    destination: &'a Path,
    limits: ResourceLimits,
    base_usage: u64,
    total: u64,
    entries: usize,
}

impl Extraction<'_> {
    fn count(&mut self) -> ApiResult<()> {
        self.entries = self.entries.saturating_add(1);
        if self.entries > self.limits.max_archive_entries {
            return Err(ApiError::Conflict(
                "archive contains too many entries".into(),
            ));
        }
        Ok(())
    }

    fn copy_file<R: Read>(&mut self, relative: &Path, reader: &mut R) -> ApiResult<()> {
        let parent = relative.parent().unwrap_or_else(|| Path::new("."));
        let parent_path = self.destination.join(parent);
        self.fs.create_dir_all(&parent_path).map_err(io_error)?;
        let temporary = parent_path.join(format!(".mcpanel-extract-{}", Uuid::new_v4().simple()));
        let mut output = self.fs.create_new_file(&temporary).map_err(io_error)?;
        let mut file_bytes = 0_u64;
        let result = (|| -> ApiResult<()> {
            let mut buffer = [0_u8; 64 * 1024];
            loop {
                let read = reader.read(&mut buffer).map_err(|error| {
                    ApiError::BadRequest(format!("archive read failed: {error}"))
                })?;
                if read == 0 {
                    break;
                }
                file_bytes = file_bytes.checked_add(read as u64).ok_or_else(|| {
                    ApiError::Conflict("archive expands beyond allowed size".into())
                })?;
                let total = self.total.checked_add(file_bytes).ok_or_else(|| {
                    ApiError::Conflict("archive expands beyond allowed size".into())
                })?;
                if file_bytes > self.limits.max_extracted_file_bytes {
                    return Err(ApiError::Conflict(
                        "archive file exceeds the per-file limit".into(),
                    ));
                }
                if total > self.limits.max_extracted_bytes
                    || self.base_usage.saturating_add(total) > self.limits.max_server_disk_bytes
                {
                    return Err(ApiError::Conflict("server storage quota exceeded".into()));
                }
                output
                    .write_all(&buffer[..read])
                    .map_err(|error| ApiError::Internal(error.into()))?;
            }
            output
                .sync_all()
                .map_err(|error| ApiError::Internal(error.into()))?;
            Ok(())
        })();
        drop(output);
        if let Err(error) = result {
            let _ = self.fs.remove(&temporary);
            return Err(error);
        }
        self.fs
            .rename(&temporary, self.destination.join(relative))
            .map_err(|error| {
                let _ = self.fs.remove(&temporary);
                io_error(error)
            })?;
        self.total = self.total.saturating_add(file_bytes);
        Ok(())
    }
}

fn extract_zip(
    fs: &ScopedFs,
    archive: &Path,
    destination: &Path,
    limits: ResourceLimits,
    base_usage: u64,
) -> ApiResult<usize> {
    let file = fs.open_file(archive).map_err(io_error)?;
    let mut zip = zip::ZipArchive::new(file)
        .map_err(|error| ApiError::BadRequest(format!("not a readable zip: {error}")))?;
    let mut extraction = Extraction {
        fs,
        destination,
        limits,
        base_usage,
        total: 0,
        entries: 0,
    };
    for index in 0..zip.len() {
        let mut entry = zip
            .by_index(index)
            .map_err(|error| ApiError::BadRequest(format!("corrupt zip entry: {error}")))?;
        extraction.count()?;
        let Some(relative) = entry.enclosed_name() else {
            return Err(ApiError::BadRequest(format!(
                "archive entry {} would escape the destination",
                entry.name()
            )));
        };
        if !safe_member(&relative) || entry.is_symlink() {
            return Err(ApiError::BadRequest(format!(
                "archive entry {} is unsafe",
                entry.name()
            )));
        }
        let Some(relative) = normalise_member(&relative) else {
            continue;
        };
        let target = destination.join(&relative);
        if entry.is_dir() {
            fs.create_dir_all(&target).map_err(io_error)?;
        } else {
            extraction.copy_file(&relative, &mut entry)?;
        }
    }
    Ok(extraction.entries)
}

fn extract_tar_gz(
    fs: &ScopedFs,
    archive: &Path,
    destination: &Path,
    limits: ResourceLimits,
    base_usage: u64,
) -> ApiResult<usize> {
    let file = fs.open_file(archive).map_err(io_error)?;
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(file));
    let mut extraction = Extraction {
        fs,
        destination,
        limits,
        base_usage,
        total: 0,
        entries: 0,
    };
    for entry in tar
        .entries()
        .map_err(|error| ApiError::BadRequest(format!("not a readable tarball: {error}")))?
    {
        let mut entry =
            entry.map_err(|error| ApiError::BadRequest(format!("corrupt entry: {error}")))?;
        extraction.count()?;
        let relative = entry
            .path()
            .map_err(|error| ApiError::BadRequest(format!("bad entry path: {error}")))?
            .into_owned();
        if !safe_member(&relative) {
            return Err(ApiError::BadRequest(format!(
                "archive entry {} would escape the destination",
                relative.display()
            )));
        }
        let entry_type = entry.header().entry_type();
        if entry_type.is_symlink()
            || entry_type.is_hard_link()
            || (!entry_type.is_file() && !entry_type.is_dir())
        {
            return Err(ApiError::BadRequest(format!(
                "archive entry {} is an unsupported link or special file",
                relative.display()
            )));
        }
        let Some(relative) = normalise_member(&relative) else {
            continue;
        };
        let target = destination.join(&relative);
        if entry_type.is_dir() {
            fs.create_dir_all(&target).map_err(io_error)?;
        } else {
            if entry.size() > limits.max_extracted_file_bytes {
                return Err(ApiError::Conflict(
                    "archive file exceeds the per-file limit".into(),
                ));
            }
            extraction.copy_file(&relative, &mut entry)?;
        }
    }
    Ok(extraction.entries)
}

async fn extract(
    State(state): State<Arc<AppState>>,
    identity: Identity,
    AxumPath(id): AxumPath<String>,
    Json(body): Json<ExtractRequest>,
) -> ApiResult<Json<serde_json::Value>> {
    let root = root_for(&state, &identity, &id).await?;
    let archive = relative(&body.path)?;
    if archive.as_os_str().is_empty() {
        return Err(ApiError::BadRequest("an archive path is required".into()));
    }
    let destination = match body.into {
        Some(into) => relative(&into)?,
        None => archive
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf(),
    };
    let _resource_lock = state.resource_lock.lock().await;
    let fs = open(root).await?;
    let archive_metadata = fs.metadata(&archive).map_err(io_error)?;
    if !archive_metadata.is_file {
        return Err(ApiError::NotFound("archive".into()));
    }
    fs.create_dir_all(&destination).map_err(io_error)?;
    let quota_fs = fs.try_clone().map_err(io_error)?;
    let base_usage = tokio::task::spawn_blocking(move || quota_fs.directory_size("."))
        .await
        .map_err(task_error)?
        .map_err(io_error)?;
    let limits = state.limits;
    let lowered = body.path.to_ascii_lowercase();
    let count = tokio::task::spawn_blocking(move || {
        if lowered.ends_with(".zip") || lowered.ends_with(".jar") {
            extract_zip(&fs, &archive, &destination, limits, base_usage)
        } else if lowered.ends_with(".tar.gz") || lowered.ends_with(".tgz") {
            extract_tar_gz(&fs, &archive, &destination, limits, base_usage)
        } else {
            Err(ApiError::BadRequest(
                "only .zip, .jar, .tar.gz and .tgz archives can be extracted".into(),
            ))
        }
    })
    .await
    .map_err(task_error)??;
    Ok(Json(serde_json::json!({ "ok": true, "entries": count })))
}

/// Body of the rename endpoint.
#[derive(Deserialize)]
pub struct RenameRequest {
    from: String,
    to: String,
}

async fn rename(
    State(state): State<Arc<AppState>>,
    identity: Identity,
    AxumPath(id): AxumPath<String>,
    Json(body): Json<RenameRequest>,
) -> ApiResult<Json<serde_json::Value>> {
    let root = root_for(&state, &identity, &id).await?;
    let from = relative(&body.from)?;
    let to = relative(&body.to)?;
    if from.as_os_str().is_empty() || to.as_os_str().is_empty() {
        return Err(ApiError::BadRequest(
            "refusing to rename the server root".into(),
        ));
    }
    let fs = open(root).await?;
    if let Some(parent) = to.parent() {
        let parent = parent.to_path_buf();
        let parent_fs = fs.try_clone().map_err(io_error)?;
        tokio::task::spawn_blocking(move || parent_fs.create_dir_all(&parent))
            .await
            .map_err(task_error)?
            .map_err(io_error)?;
    }
    tokio::task::spawn_blocking(move || fs.rename(&from, &to))
        .await
        .map_err(task_error)?
        .map_err(io_error)?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

/// Routes under the server file manager.
#[allow(dead_code)]
pub fn router() -> Router<Arc<AppState>> {
    router_with_limits(ResourceLimits::default())
}

/// Build the file routes with explicit installation limits.
pub fn router_with_limits(limits: ResourceLimits) -> Router<Arc<AppState>> {
    let upload_limit = limits.max_upload_bytes.min(usize::MAX as u64) as usize;
    Router::new()
        .route(
            "/{id}/files",
            get(list)
                .put(write)
                .delete(delete)
                .layer(DefaultBodyLimit::max(MAX_EDIT_BODY_BYTES + 64 * 1024)),
        )
        .route("/{id}/files/read", get(read))
        .route("/{id}/files/sizes", get(sizes))
        .route("/{id}/files/download", get(download))
        .route("/{id}/files/ticket", post(download_ticket))
        .route(
            "/{id}/files/upload",
            post(upload).layer(DefaultBodyLimit::max(upload_limit)),
        )
        .route("/{id}/files/extract", post(extract))
        .route("/{id}/files/rename", post(rename))
        .route("/{id}/files/mkdir", post(mkdir))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_paths_are_strictly_relative() {
        assert_eq!(
            display_path(&relative("world/level.dat").unwrap()),
            "world/level.dat"
        );
        for attempt in ["../secrets", "/etc/passwd", "C:/Windows/win.ini", "a\\b"] {
            assert!(relative(attempt).is_err(), "{attempt} should be rejected");
        }
    }

    #[test]
    fn archive_members_reject_traversal_links_and_drive_prefixes() {
        assert!(safe_member(Path::new("./plugins/Thing.jar")));
        assert!(!safe_member(Path::new("../evil")));
        assert!(!safe_member(Path::new("/etc/passwd")));
        assert!(!safe_member(Path::new("C:/Windows/win.ini")));
        assert!(!safe_member(Path::new("plugins\\escape")));
    }
}
