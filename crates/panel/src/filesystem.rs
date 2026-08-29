//! Panel-facing helpers for the guardian capability filesystem.

use axum::extract::multipart::MultipartError;
use guardian::ScopedFs;
use std::path::{Component, Path, PathBuf};

use crate::error::{ApiError, ApiResult};

/// Keep path processing bounded before it reaches the filesystem layer.
pub const MAX_PATH_BYTES: usize = 4096;

/// Open a server root without resolving it again for each request.
pub async fn open(root: PathBuf) -> ApiResult<ScopedFs> {
    tokio::task::spawn_blocking(move || {
        ScopedFs::open(&root).map_err(|_| ApiError::NotFound("server directory".into()))
    })
    .await
    .map_err(|_| ApiError::Internal(anyhow::anyhow!("filesystem task failed")))?
}

/// Parse a user path into a strictly relative path.
///
/// Absolute paths and backslashes are rejected rather than normalised. This
/// avoids platform-dependent interpretations and keeps the API contract
/// unambiguous; the capability layer performs the race-resistant resolution.
pub fn relative(input: &str) -> ApiResult<PathBuf> {
    if input.len() > MAX_PATH_BYTES {
        return Err(ApiError::BadRequest("path is too long".into()));
    }
    if input.is_empty() {
        return Ok(PathBuf::new());
    }
    if input.contains('\0') || input.contains('\\') {
        return Err(ApiError::BadRequest(
            "path must be relative and use forward slashes".into(),
        ));
    }

    let path = Path::new(input);
    if path.is_absolute() {
        return Err(ApiError::BadRequest(
            "path escapes the server directory".into(),
        ));
    }

    let mut output = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(name) => output.push(name),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(ApiError::BadRequest(
                    "path escapes the server directory".into(),
                ));
            }
        }
    }
    Ok(output)
}

/// A single filename supplied in multipart metadata or a download header.
pub fn filename(input: &str) -> ApiResult<&str> {
    if input.is_empty()
        || input == "."
        || input == ".."
        || input.contains(['/', '\\', '\0', '\r', '\n'])
        || input.len() > 255
    {
        return Err(ApiError::BadRequest("invalid file name".into()));
    }
    Ok(input)
}

/// Convert a relative path to the stable display form used by the frontend.
pub fn display_path(path: &Path) -> String {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(name) => Some(name.to_string_lossy().into_owned()),
            Component::CurDir => None,
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// Translate multipart parser failures into a client-safe API error.
pub fn multipart_error(error: MultipartError) -> ApiError {
    ApiError::BadRequest(format!("malformed upload: {error}"))
}
