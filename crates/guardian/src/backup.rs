//! Server backups: gzipped tarballs of a server directory.
//!
//! A backup captures what cannot be re-downloaded — worlds, configuration,
//! plugins, player data — and deliberately skips what can. Re-fetching a
//! 300 MB `libraries/` tree is free; losing a world is not, and a backup that
//! is mostly redownloadable bytes is a backup people stop taking.

use crate::error::{Error, Result};
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::path::{Component, Path, PathBuf};

/// Directory names skipped by [`create`], because the panel can fetch them again.
const EXCLUDED: &[&str] = &["cache", "libraries", "versions", "logs", ".paper"];

/// Files skipped by [`create`], for the same reason.
const EXCLUDED_FILES: &[&str] = &["server.jar"];

/// A stored backup.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Backup {
    /// Identifier, also the archive's file stem.
    pub id: String,
    /// RFC 3339 creation time.
    pub created_at: String,
    /// Size of the archive on disk, in bytes.
    pub size: u64,
    /// Operator-supplied label.
    #[serde(default)]
    pub note: String,
}

fn archive_path(backup_dir: &Path, id: &str) -> PathBuf {
    backup_dir.join(format!("{id}.tar.gz"))
}

fn meta_path(backup_dir: &Path, id: &str) -> PathBuf {
    backup_dir.join(format!("{id}.json"))
}

/// Reject anything that is not a plain file-stem, so an id from a request body
/// cannot address a path outside `backup_dir`.
fn validate_id(id: &str) -> Result<()> {
    let sane = !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if sane {
        Ok(())
    } else {
        Err(Error::InvalidBackupId(id.to_string()))
    }
}

/// Whether an archive member stays inside the destination directory.
fn safe_member(path: &Path) -> bool {
    path.components()
        .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
}

/// `20260819-160422` — sortable, readable, and safe as a filename.
fn timestamp_id() -> String {
    let now = time::OffsetDateTime::now_utc();
    format!(
        "{:04}{:02}{:02}-{:02}{:02}{:02}",
        now.year(),
        u8::from(now.month()),
        now.day(),
        now.hour(),
        now.minute(),
        now.second()
    )
}

fn now_rfc3339() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".into())
}

/// Whether a path relative to the server root should go into the archive.
fn included(relative: &Path) -> bool {
    let mut components = relative.components();
    match components.next() {
        Some(Component::Normal(first)) => {
            let name = first.to_string_lossy();
            !EXCLUDED.contains(&name.as_ref()) && !EXCLUDED_FILES.contains(&name.as_ref())
        }
        _ => true,
    }
}

/// Archive `server_dir` into `backup_dir`.
///
/// Runs on the blocking pool: tar and gzip are synchronous, and a multi-gigabyte
/// world would otherwise stall the runtime for the whole compression.
pub async fn create(server_dir: &Path, backup_dir: &Path, note: String) -> Result<Backup> {
    tokio::fs::create_dir_all(backup_dir)
        .await
        .map_err(|e| Error::io(backup_dir, e))?;

    let id = timestamp_id();
    let archive = archive_path(backup_dir, &id);
    let source = server_dir.to_path_buf();
    let target = archive.clone();

    tokio::task::spawn_blocking(move || -> Result<()> {
        let file = File::create(&target).map_err(|e| Error::io(&target, e))?;
        let encoder = GzEncoder::new(file, Compression::default());
        let mut builder = tar::Builder::new(encoder);

        for entry in walk(&source)? {
            let relative = entry.strip_prefix(&source).unwrap_or(&entry);
            if !included(relative) {
                continue;
            }
            let meta = std::fs::metadata(&entry).map_err(|e| Error::io(&entry, e))?;
            if meta.is_dir() {
                continue;
            }
            let mut file = File::open(&entry).map_err(|e| Error::io(&entry, e))?;
            builder
                .append_file(relative, &mut file)
                .map_err(|e| Error::io(&entry, e))?;
        }

        builder
            .into_inner()
            .and_then(|e| e.finish())
            .map_err(Error::PlainIo)?;
        Ok(())
    })
    .await
    .map_err(|e| Error::Task(e.to_string()))??;

    let size = tokio::fs::metadata(&archive)
        .await
        .map(|m| m.len())
        .unwrap_or(0);

    let backup = Backup {
        id: id.clone(),
        created_at: now_rfc3339(),
        size,
        note,
    };
    let meta = meta_path(backup_dir, &id);
    tokio::fs::write(&meta, serde_json::to_vec_pretty(&backup)?)
        .await
        .map_err(|e| Error::io(&meta, e))?;

    Ok(backup)
}

/// Every backup in `backup_dir`, newest first.
pub async fn list(backup_dir: &Path) -> Result<Vec<Backup>> {
    let mut out = Vec::new();

    let mut dir = match tokio::fs::read_dir(backup_dir).await {
        Ok(dir) => dir,
        // No directory simply means no backups have been taken yet.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
        Err(e) => return Err(Error::io(backup_dir, e)),
    };

    while let Ok(Some(entry)) = dir.next_entry().await {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Ok(bytes) = tokio::fs::read(&path).await else {
            continue;
        };
        let Ok(backup) = serde_json::from_slice::<Backup>(&bytes) else {
            continue;
        };
        // A metadata file whose archive is gone would only produce a failing
        // restore, so it is not worth listing.
        if archive_path(backup_dir, &backup.id).exists() {
            out.push(backup);
        }
    }

    out.sort_by(|a, b| b.id.cmp(&a.id));
    Ok(out)
}

/// Extract a backup over `server_dir`.
///
/// Files in the archive overwrite their counterparts; files added since the
/// backup are left alone. The caller must ensure the server is stopped —
/// unpacking a world under a running JVM corrupts it.
pub async fn restore(backup_dir: &Path, id: &str, server_dir: &Path) -> Result<()> {
    validate_id(id)?;

    let archive = archive_path(backup_dir, id);
    if !archive.exists() {
        return Err(Error::BackupNotFound(id.to_string()));
    }

    tokio::fs::create_dir_all(server_dir)
        .await
        .map_err(|e| Error::io(server_dir, e))?;

    let target = server_dir.to_path_buf();
    tokio::task::spawn_blocking(move || -> Result<()> {
        let file = File::open(&archive).map_err(|e| Error::io(&archive, e))?;
        let mut tar = tar::Archive::new(GzDecoder::new(file));

        for entry in tar.entries().map_err(Error::PlainIo)? {
            let mut entry = entry.map_err(Error::PlainIo)?;
            let path = entry.path().map_err(Error::PlainIo)?.into_owned();

            // An archive is untrusted input: a crafted member path must not be
            // able to write outside the server directory. `./` is harmless and
            // is what `tar -C dir .` emits for every entry.
            if !safe_member(&path) {
                return Err(Error::UnsafeArchiveEntry(path.display().to_string()));
            }

            entry
                .unpack(target.join(&path))
                .map_err(|e| Error::io(&path, e))?;
        }
        Ok(())
    })
    .await
    .map_err(|e| Error::Task(e.to_string()))??;

    Ok(())
}

/// Remove a backup and its metadata.
pub async fn delete(backup_dir: &Path, id: &str) -> Result<()> {
    validate_id(id)?;

    let archive = archive_path(backup_dir, id);
    if !archive.exists() {
        return Err(Error::BackupNotFound(id.to_string()));
    }

    tokio::fs::remove_file(&archive)
        .await
        .map_err(|e| Error::io(&archive, e))?;
    let _ = tokio::fs::remove_file(meta_path(backup_dir, id)).await;
    Ok(())
}

/// The archive for `id`, for streaming a download.
pub fn path_of(backup_dir: &Path, id: &str) -> Result<PathBuf> {
    validate_id(id)?;
    let archive = archive_path(backup_dir, id);
    if archive.exists() {
        Ok(archive)
    } else {
        Err(Error::BackupNotFound(id.to_string()))
    }
}

/// Every file under `root`, recursively.
fn walk(root: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];

    while let Some(dir) = stack.pop() {
        let entries = std::fs::read_dir(&dir).map_err(|e| Error::io(&dir, e))?;
        for entry in entries.flatten() {
            let path = entry.path();
            // Symlinks are not followed: a link into /etc would otherwise be
            // silently archived, and restoring it would write outside the server.
            let Ok(meta) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if meta.is_symlink() {
                continue;
            }
            if meta.is_dir() {
                stack.push(path);
            } else {
                out.push(path);
            }
        }
    }

    out.sort();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, body: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    }

    #[test]
    fn excluded_directories_are_not_archived() {
        assert!(included(Path::new("world/level.dat")));
        assert!(included(Path::new("server.properties")));
        assert!(!included(Path::new("libraries/some.jar")));
        assert!(!included(Path::new("cache/x")));
        assert!(!included(Path::new("server.jar")));
    }

    #[test]
    fn archive_members_are_checked_before_being_unpacked() {
        assert!(safe_member(Path::new("world/level.dat")));
        // tar -C dir . writes every entry with this prefix.
        assert!(safe_member(Path::new("./world/level.dat")));

        assert!(!safe_member(Path::new("../../etc/passwd")));
        assert!(!safe_member(Path::new("/etc/passwd")));
    }

    #[test]
    fn ids_from_requests_cannot_escape_the_backup_directory() {
        assert!(validate_id("20260819-160422").is_ok());
        assert!(validate_id("../../etc/passwd").is_err());
        assert!(validate_id("a/b").is_err());
        assert!(validate_id("").is_err());
        assert!(validate_id(&"x".repeat(65)).is_err());
    }

    #[tokio::test]
    async fn round_trip_restores_the_world_and_skips_redownloadables() {
        let tmp = tempfile::tempdir().unwrap();
        let server = tmp.path().join("server");
        let backups = tmp.path().join("backups");

        write(&server.join("world/level.dat"), "world-v1");
        write(&server.join("server.properties"), "port=25565");
        write(&server.join("libraries/big.jar"), "redownloadable");
        write(&server.join("server.jar"), "redownloadable");

        let backup = create(&server, &backups, "before upgrade".into())
            .await
            .unwrap();
        assert!(backup.size > 0);
        assert_eq!(backup.note, "before upgrade");

        let listed = list(&backups).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, backup.id);

        // Simulate the world being ruined after the backup was taken.
        write(&server.join("world/level.dat"), "world-v2-broken");

        restore(&backups, &backup.id, &server).await.unwrap();

        assert_eq!(
            std::fs::read_to_string(server.join("world/level.dat")).unwrap(),
            "world-v1"
        );
        assert_eq!(
            std::fs::read_to_string(server.join("server.properties")).unwrap(),
            "port=25565"
        );

        delete(&backups, &backup.id).await.unwrap();
        assert!(list(&backups).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn listing_an_absent_directory_is_empty_not_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(list(&tmp.path().join("nope")).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn restoring_an_unknown_backup_reports_not_found() {
        let tmp = tempfile::tempdir().unwrap();
        let err = restore(&tmp.path().join("b"), "20260101-000000", tmp.path())
            .await
            .unwrap_err();
        assert!(matches!(err, Error::BackupNotFound(_)));
    }
}
