//! A sandboxed file manager for a server's directory.
//!
//! Every path is resolved against the server root and rejected if it escapes,
//! so `../../etc/passwd` cannot be read through the panel.

use axum::body::Body;
use axum::extract::{Multipart, Path as AxumPath, Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio_util::io::ReaderStream;

use crate::auth::Identity;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use crate::tickets::Resource;

/// Refuse to load anything larger than this into the editor.
const MAX_EDIT_BYTES: u64 = 2 * 1024 * 1024;

/// One directory entry.
#[derive(Serialize)]
pub struct Entry {
    name: String,
    /// Path relative to the server root, using forward slashes.
    path: String,
    directory: bool,
    size: u64,
    /// Unix seconds, when the platform reports a modification time.
    modified: Option<u64>,
}

/// `?path=` on every files endpoint.
#[derive(Deserialize)]
pub struct PathQuery {
    #[serde(default)]
    path: String,
}

/// Join `relative` onto `root`, rejecting anything that escapes.
///
/// This works on the lexical path rather than the canonicalised one, because
/// the target may not exist yet (writes create files), and because refusing
/// `..` outright is easier to reason about than comparing canonical prefixes.
fn resolve(root: &Path, relative: &str) -> ApiResult<PathBuf> {
    let mut out = root.to_path_buf();
    for component in Path::new(relative.trim_start_matches(['/', '\\'])).components() {
        match component {
            Component::Normal(part) => out.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(ApiError::BadRequest(
                    "path escapes the server directory".into(),
                ));
            }
        }
    }
    Ok(out)
}

async fn root_for(state: &AppState, identity: &Identity, id: &str) -> ApiResult<PathBuf> {
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

fn relative_of(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

async fn list(
    State(state): State<Arc<AppState>>,
    identity: Identity,
    AxumPath(id): AxumPath<String>,
    Query(query): Query<PathQuery>,
) -> ApiResult<Json<Vec<Entry>>> {
    let root = root_for(&state, &identity, &id).await?;
    let target = resolve(&root, &query.path)?;

    let mut dir = tokio::fs::read_dir(&target)
        .await
        .map_err(|_| ApiError::NotFound(format!("directory {}", query.path)))?;

    let mut entries = Vec::new();
    while let Ok(Some(item)) = dir.next_entry().await {
        let Ok(meta) = item.metadata().await else {
            continue;
        };
        entries.push(Entry {
            name: item.file_name().to_string_lossy().into_owned(),
            path: relative_of(&root, &item.path()),
            directory: meta.is_dir(),
            size: meta.len(),
            modified: meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs()),
        });
    }

    // Directories first, then alphabetical: the ordering every file manager uses.
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
    let target = resolve(&root, &query.path)?;

    let meta = tokio::fs::metadata(&target)
        .await
        .map_err(|_| ApiError::NotFound(format!("file {}", query.path)))?;
    if meta.is_dir() {
        return Err(ApiError::BadRequest("that is a directory".into()));
    }
    if meta.len() > MAX_EDIT_BYTES {
        return Err(ApiError::BadRequest(format!(
            "file is {} bytes; the editor handles up to {MAX_EDIT_BYTES}",
            meta.len()
        )));
    }

    let bytes = tokio::fs::read(&target)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    let content = String::from_utf8(bytes)
        .map_err(|_| ApiError::BadRequest("file is not valid UTF-8".into()))?;

    Ok(Json(FileContents {
        path: query.path,
        content,
    }))
}

/// Body of `PUT /api/servers/:id/files`.
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
    let target = resolve(&root, &body.path)?;

    if let Some(parent) = target.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| ApiError::Internal(e.into()))?;
    }
    tokio::fs::write(&target, body.content)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;

    Ok(Json(serde_json::json!({ "ok": true })))
}

async fn delete(
    State(state): State<Arc<AppState>>,
    identity: Identity,
    AxumPath(id): AxumPath<String>,
    Query(query): Query<PathQuery>,
) -> ApiResult<Json<serde_json::Value>> {
    let root = root_for(&state, &identity, &id).await?;
    let target = resolve(&root, &query.path)?;

    if target == root {
        return Err(ApiError::BadRequest(
            "refusing to delete the server root".into(),
        ));
    }

    let meta = tokio::fs::metadata(&target)
        .await
        .map_err(|_| ApiError::NotFound(format!("path {}", query.path)))?;

    if meta.is_dir() {
        tokio::fs::remove_dir_all(&target).await
    } else {
        tokio::fs::remove_file(&target).await
    }
    .map_err(|e| ApiError::Internal(e.into()))?;

    Ok(Json(serde_json::json!({ "ok": true })))
}

/// Body of `POST /api/servers/:id/files/mkdir`.
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
    let target = resolve(&root, &body.path)?;
    tokio::fs::create_dir_all(&target)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

/// The measured size of one subdirectory.
#[derive(Serialize)]
pub struct DirectorySize {
    path: String,
    bytes: u64,
}

/// Measure every subdirectory of `?path=`.
///
/// Deliberately a second request rather than part of the listing: walking a
/// `world/` or `libraries/` tree costs real I/O, and the file list should appear
/// immediately rather than waiting on it. Results are cached, so revisiting a
/// folder is free.
async fn sizes(
    State(state): State<Arc<AppState>>,
    identity: Identity,
    AxumPath(id): AxumPath<String>,
    Query(query): Query<PathQuery>,
) -> ApiResult<Json<Vec<DirectorySize>>> {
    let root = root_for(&state, &identity, &id).await?;
    let target = resolve(&root, &query.path)?;

    let mut dir = tokio::fs::read_dir(&target)
        .await
        .map_err(|_| ApiError::NotFound(format!("directory {}", query.path)))?;

    let mut out = Vec::new();
    while let Ok(Some(item)) = dir.next_entry().await {
        let Ok(meta) = item.metadata().await else {
            continue;
        };
        if !meta.is_dir() {
            continue;
        }
        let path = item.path();
        out.push(DirectorySize {
            bytes: state.metrics.disk_usage(&path).await,
            path: relative_of(&root, &path),
        });
    }

    Ok(Json(out))
}

/// Issue a short-lived grant for one file, for the browser to redeem.
async fn download_ticket(
    State(state): State<Arc<AppState>>,
    identity: Identity,
    AxumPath(id): AxumPath<String>,
    Query(query): Query<PathQuery>,
) -> ApiResult<Json<serde_json::Value>> {
    // Access is decided here, while the caller is still authenticated.
    let root = root_for(&state, &identity, &id).await?;
    let target = resolve(&root, &query.path)?;

    if !target.is_file() {
        return Err(ApiError::NotFound(format!("file {}", query.path)));
    }

    let ticket = state.tickets.issue(Resource::File {
        server: id,
        path: query.path,
    });

    Ok(Json(serde_json::json!({ "ticket": ticket })))
}

/// `?ticket=` on the download route.
#[derive(Deserialize)]
pub struct TicketQuery {
    ticket: String,
}

/// Stream a file out for a browser navigation, authorised by a ticket.
///
/// No session credential is involved, so nothing sensitive is written to
/// browser history or to the request log.
async fn download(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
    Query(query): Query<TicketQuery>,
) -> ApiResult<Response> {
    let granted = state
        .tickets
        .redeem(&query.ticket)
        .ok_or(ApiError::Unauthorized)?;

    let Resource::File { server, path } = granted else {
        return Err(ApiError::Unauthorized);
    };
    // A ticket for one server must not open a file in another.
    if server != id {
        return Err(ApiError::Unauthorized);
    }

    let root = state
        .store
        .server(&id)
        .await
        .map(|record| record.config.directory)
        .ok_or_else(|| ApiError::NotFound(format!("server {id}")))?;
    let target = resolve(&root, &path)?;
    let query = PathQuery { path };

    let meta = tokio::fs::metadata(&target)
        .await
        .map_err(|_| ApiError::NotFound(format!("file {}", query.path)))?;
    if meta.is_dir() {
        return Err(ApiError::BadRequest("that is a directory".into()));
    }

    let name = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "download".into());

    let file = tokio::fs::File::open(&target)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;

    Ok((
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/octet-stream".to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{name}\""),
            ),
        ],
        Body::from_stream(ReaderStream::new(file)),
    )
        .into_response())
}

/// Accept one or more uploaded files into `?path=`.
async fn upload(
    State(state): State<Arc<AppState>>,
    identity: Identity,
    AxumPath(id): AxumPath<String>,
    Query(query): Query<PathQuery>,
    mut multipart: Multipart,
) -> ApiResult<Json<serde_json::Value>> {
    let root = root_for(&state, &identity, &id).await?;
    let directory = resolve(&root, &query.path)?;
    tokio::fs::create_dir_all(&directory)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;

    let mut written = Vec::new();

    while let Some(mut field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::BadRequest(format!("malformed upload: {e}")))?
    {
        let Some(filename) = field.file_name().map(str::to_string) else {
            continue;
        };

        // The client controls this name, so it goes through the same sandbox as
        // every other path rather than being trusted.
        let relative = if query.path.is_empty() {
            filename.clone()
        } else {
            format!("{}/{}", query.path, filename)
        };
        let target = resolve(&root, &relative)?;

        if let Some(parent) = target.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| ApiError::Internal(e.into()))?;
        }

        // Streamed chunk by chunk so a 200 MB modpack does not have to fit in RAM.
        let mut file = tokio::fs::File::create(&target)
            .await
            .map_err(|e| ApiError::Internal(e.into()))?;

        while let Some(chunk) = field
            .chunk()
            .await
            .map_err(|e| ApiError::BadRequest(format!("upload interrupted: {e}")))?
        {
            file.write_all(&chunk)
                .await
                .map_err(|e| ApiError::Internal(e.into()))?;
        }
        file.flush()
            .await
            .map_err(|e| ApiError::Internal(e.into()))?;

        written.push(relative);
    }

    if written.is_empty() {
        return Err(ApiError::BadRequest("no files in the upload".into()));
    }

    Ok(Json(serde_json::json!({ "ok": true, "written": written })))
}

/// Body of `POST /api/servers/:id/files/extract`.
#[derive(Deserialize)]
pub struct ExtractRequest {
    /// Archive to unpack, relative to the server root.
    path: String,
    /// Where to unpack it; defaults to the archive's own directory.
    #[serde(default)]
    into: Option<String>,
}

/// Reject archive members that would escape the destination.
///
/// `./` is allowed: `tar czf out.tar.gz -C dir .` writes every entry with that
/// prefix, and it does not move anywhere.
fn safe_member(path: &Path) -> bool {
    path.components()
        .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
}

async fn extract(
    State(state): State<Arc<AppState>>,
    identity: Identity,
    AxumPath(id): AxumPath<String>,
    Json(body): Json<ExtractRequest>,
) -> ApiResult<Json<serde_json::Value>> {
    let root = root_for(&state, &identity, &id).await?;
    let archive = resolve(&root, &body.path)?;

    let destination = match &body.into {
        Some(into) => resolve(&root, into)?,
        None => archive.parent().unwrap_or(&root).to_path_buf(),
    };

    if !archive.is_file() {
        return Err(ApiError::NotFound(format!("archive {}", body.path)));
    }
    tokio::fs::create_dir_all(&destination)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;

    let lowered = body.path.to_ascii_lowercase();
    let count = tokio::task::spawn_blocking(move || -> ApiResult<usize> {
        if lowered.ends_with(".zip") || lowered.ends_with(".jar") {
            extract_zip(&archive, &destination)
        } else if lowered.ends_with(".tar.gz") || lowered.ends_with(".tgz") {
            extract_tar_gz(&archive, &destination)
        } else {
            Err(ApiError::BadRequest(
                "only .zip, .jar, .tar.gz and .tgz archives can be extracted".into(),
            ))
        }
    })
    .await
    .map_err(|e| ApiError::Internal(anyhow::anyhow!("extraction task failed: {e}")))??;

    Ok(Json(serde_json::json!({ "ok": true, "entries": count })))
}

fn extract_zip(archive: &Path, destination: &Path) -> ApiResult<usize> {
    let file = std::fs::File::open(archive).map_err(|e| ApiError::Internal(e.into()))?;
    let mut zip = zip::ZipArchive::new(file)
        .map_err(|e| ApiError::BadRequest(format!("not a readable zip: {e}")))?;

    let mut count = 0;
    for index in 0..zip.len() {
        let mut entry = zip
            .by_index(index)
            .map_err(|e| ApiError::BadRequest(format!("corrupt zip entry: {e}")))?;

        // `enclosed_name` already rejects traversal, but the explicit check
        // keeps the guarantee visible next to the tar path below.
        let Some(relative) = entry.enclosed_name() else {
            return Err(ApiError::BadRequest(format!(
                "archive entry {} would escape the destination",
                entry.name()
            )));
        };
        if !safe_member(&relative) {
            return Err(ApiError::BadRequest(format!(
                "archive entry {} would escape the destination",
                relative.display()
            )));
        }

        let target = destination.join(&relative);
        if entry.is_dir() {
            std::fs::create_dir_all(&target).map_err(|e| ApiError::Internal(e.into()))?;
            continue;
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| ApiError::Internal(e.into()))?;
        }
        let mut out = std::fs::File::create(&target).map_err(|e| ApiError::Internal(e.into()))?;
        std::io::copy(&mut entry, &mut out).map_err(|e| ApiError::Internal(e.into()))?;
        count += 1;
    }
    Ok(count)
}

fn extract_tar_gz(archive: &Path, destination: &Path) -> ApiResult<usize> {
    let file = std::fs::File::open(archive).map_err(|e| ApiError::Internal(e.into()))?;
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(file));

    let mut count = 0;
    for entry in tar
        .entries()
        .map_err(|e| ApiError::BadRequest(format!("not a readable tarball: {e}")))?
    {
        let mut entry = entry.map_err(|e| ApiError::BadRequest(format!("corrupt entry: {e}")))?;
        let relative = entry
            .path()
            .map_err(|e| ApiError::BadRequest(format!("bad entry path: {e}")))?
            .into_owned();

        if !safe_member(&relative) {
            return Err(ApiError::BadRequest(format!(
                "archive entry {} would escape the destination",
                relative.display()
            )));
        }

        entry
            .unpack(destination.join(&relative))
            .map_err(|e| ApiError::Internal(e.into()))?;
        count += 1;
    }
    Ok(count)
}

/// Body of `POST /api/servers/:id/files/rename`.
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
    let from = resolve(&root, &body.from)?;
    let to = resolve(&root, &body.to)?;

    if from == root || to == root {
        return Err(ApiError::BadRequest(
            "refusing to rename the server root".into(),
        ));
    }
    if let Some(parent) = to.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| ApiError::Internal(e.into()))?;
    }

    tokio::fs::rename(&from, &to)
        .await
        .map_err(|_| ApiError::NotFound(format!("path {}", body.from)))?;

    Ok(Json(serde_json::json!({ "ok": true })))
}

/// Routes under `/api/servers/{id}/files`.
pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/{id}/files", get(list).put(write).delete(delete))
        .route("/{id}/files/read", get(read))
        .route("/{id}/files/sizes", get(sizes))
        .route("/{id}/files/download", get(download))
        .route("/{id}/files/ticket", post(download_ticket))
        .route("/{id}/files/upload", post(upload))
        .route("/{id}/files/extract", post(extract))
        .route("/{id}/files/rename", post(rename))
        .route("/{id}/files/mkdir", post(mkdir))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> PathBuf {
        PathBuf::from("/srv/servers/abc")
    }

    #[test]
    fn ordinary_relative_paths_resolve_under_the_root() {
        assert_eq!(
            resolve(&root(), "world/level.dat").unwrap(),
            root().join("world/level.dat")
        );
        assert_eq!(resolve(&root(), "").unwrap(), root());
        // A leading slash is a client habit, not an attempt to escape.
        assert_eq!(
            resolve(&root(), "/plugins").unwrap(),
            root().join("plugins")
        );
        assert_eq!(resolve(&root(), "./a/./b").unwrap(), root().join("a/b"));
    }

    #[test]
    fn traversal_is_refused_however_it_is_spelled() {
        for attempt in [
            "../secrets",
            "world/../../etc/passwd",
            "/../etc/passwd",
            "a/../../b",
            "..",
        ] {
            assert!(
                resolve(&root(), attempt).is_err(),
                "{attempt} should have been rejected"
            );
        }
    }

    #[test]
    fn absolute_paths_cannot_replace_the_root() {
        assert!(resolve(&root(), "//etc/passwd").is_err_or_under_root());
    }

    #[test]
    fn archive_members_must_stay_inside_the_destination() {
        assert!(safe_member(Path::new("plugins/Thing.jar")));
        assert!(!safe_member(Path::new("../evil")));
        assert!(!safe_member(Path::new("/etc/passwd")));
        assert!(!safe_member(Path::new("a/../../b")));
    }

    #[test]
    fn dot_slash_entries_are_accepted() {
        // `tar czf out.tar.gz -C dir .` produces exactly these.
        assert!(safe_member(Path::new("./")));
        assert!(safe_member(Path::new("./plugins/Thing.jar")));
    }

    /// Helper so the absolute-path case reads clearly: either it is refused, or
    /// it was flattened to something still inside the root.
    trait ResolveAssert {
        fn is_err_or_under_root(&self) -> bool;
    }

    impl ResolveAssert for ApiResult<PathBuf> {
        fn is_err_or_under_root(&self) -> bool {
            match self {
                Err(_) => true,
                Ok(path) => path.starts_with(root()),
            }
        }
    }
}
