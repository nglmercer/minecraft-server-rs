//! Server backups: gzipped tarballs of a server directory.
//!
//! A backup captures what cannot be re-downloaded — worlds, configuration,
//! plugins, player data — and deliberately skips what can. Re-fetching a
//! 300 MB `libraries/` tree is free; losing a world is not, and a backup that
//! is mostly redownloadable bytes is a backup people stop taking.

use crate::error::{Error, Result};
use crate::fs::{EntryMetadata, ScopedFs};
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use uuid::Uuid;

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

/// Bounds applied while reading an untrusted backup archive.
#[derive(Debug, Clone, Copy)]
pub struct ArchiveLimits {
    /// Maximum number of archive entries, including directories.
    pub max_entries: usize,
    /// Maximum number of uncompressed bytes emitted by the archive.
    pub max_total_bytes: u64,
    /// Maximum size of one extracted regular file.
    pub max_file_bytes: u64,
}

impl Default for ArchiveLimits {
    fn default() -> Self {
        Self {
            max_entries: 100_000,
            max_total_bytes: 8 * 1024 * 1024 * 1024,
            max_file_bytes: 2 * 1024 * 1024 * 1024,
        }
    }
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
    let raw = path.to_string_lossy();
    let windows_drive =
        raw.len() >= 2 && raw.as_bytes()[0].is_ascii_alphabetic() && raw.as_bytes()[1] == b':';
    !raw.starts_with('/')
        && !raw.starts_with('\\')
        && !raw.contains('\\')
        && !windows_drive
        && path
            .components()
            .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
}

/// `20260819-160422` — sortable, readable, and safe as a filename.
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
    create_with_limit(server_dir, backup_dir, note, u64::MAX).await
}

/// Archive `server_dir`, refusing to emit more than `max_archive_bytes` of
/// compressed output. The limit is enforced by the writer rather than by an
/// estimate of the source tree, so compression ratio cannot bypass it.
pub async fn create_with_limit(
    server_dir: &Path,
    backup_dir: &Path,
    note: String,
    max_archive_bytes: u64,
) -> Result<Backup> {
    tokio::fs::create_dir_all(backup_dir)
        .await
        .map_err(|e| Error::io(backup_dir, e))?;

    // UUIDs avoid the old second-resolution collision when two backup jobs are
    // started concurrently. The archive is still created with create_new in
    // the blocking section below, so even an improbable collision is harmless.
    let id = Uuid::new_v4().simple().to_string();
    let archive = archive_path(backup_dir, &id);
    let source = server_dir.to_path_buf();
    let target_name = format!("{id}.tar.gz");
    let temporary_name = format!(".{id}.tar.gz.tmp-{}", std::process::id());
    let backup_root = backup_dir.to_path_buf();
    let archive_for_task = archive.clone();

    tokio::task::spawn_blocking(move || -> Result<()> {
        let source_fs = ScopedFs::open(&source).map_err(|e| Error::io(&source, e))?;
        let backup_fs = ScopedFs::open(&backup_root).map_err(|e| Error::io(&backup_root, e))?;
        backup_fs
            .set_private()
            .map_err(|e| Error::io(&backup_root, e))?;
        let file = backup_fs
            .create_new_file(&temporary_name)
            .map_err(|e| Error::io(backup_root.join(&temporary_name), e))?;
        let encoder = GzEncoder::new(
            LimitedWriter::new(file, max_archive_bytes),
            Compression::default(),
        );
        let mut builder = tar::Builder::new(encoder);

        let result = (|| -> Result<()> {
            for (relative, metadata) in walk_scoped(&source_fs, Path::new("."))? {
                if !included(&relative) || metadata.is_dir {
                    continue;
                }
                let mut file = source_fs
                    .open_file(&relative)
                    .map_err(|e| Error::io(&relative, e))?;
                builder
                    .append_file(&relative, &mut file)
                    .map_err(|e| Error::io(&relative, e))?;
            }

            let writer = builder
                .into_inner()
                .and_then(|encoder| encoder.finish())
                .map_err(Error::PlainIo)?;
            let file = writer.into_inner();
            file.sync_all().map_err(Error::PlainIo)?;
            drop(file);
            backup_fs
                .rename(&temporary_name, &target_name)
                .map_err(|e| Error::io(&archive_for_task, e))?;
            Ok(())
        })();

        if result.is_err() {
            let _ = backup_fs.remove(&temporary_name);
        }
        result
    })
    .await
    .map_err(|e| Error::Task(e.to_string()))??;

    let backup_fs = ScopedFs::open(backup_dir).map_err(|e| Error::io(backup_dir, e))?;
    let size = backup_fs
        .metadata(format!("{id}.tar.gz"))
        .map_err(|e| Error::io(&archive, e))?
        .len;

    let backup = Backup {
        id: id.clone(),
        created_at: now_rfc3339(),
        size,
        note,
    };
    let meta = meta_path(backup_dir, &id);
    if let Err(error) =
        backup_fs.write_atomic(format!("{id}.json"), &serde_json::to_vec_pretty(&backup)?)
    {
        let _ = backup_fs.remove(format!("{id}.tar.gz"));
        return Err(Error::io(&meta, error));
    }

    Ok(backup)
}

struct LimitedWriter<W> {
    inner: W,
    written: u64,
    limit: u64,
}

impl<W> LimitedWriter<W> {
    fn new(inner: W, limit: u64) -> Self {
        Self {
            inner,
            written: 0,
            limit,
        }
    }

    fn into_inner(self) -> W {
        self.inner
    }
}

impl<W: Write> Write for LimitedWriter<W> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let remaining = self.limit.saturating_sub(self.written);
        if bytes.len() as u64 > remaining {
            return Err(std::io::Error::other(
                "backup archive exceeds its size limit",
            ));
        }
        let written = self.inner.write(bytes)?;
        self.written = self.written.saturating_add(written as u64);
        Ok(written)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

/// Every backup in `backup_dir`, newest first.
pub async fn list(backup_dir: &Path) -> Result<Vec<Backup>> {
    let root = backup_dir.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let fs = match ScopedFs::open(&root) {
            Ok(fs) => fs,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(Error::io(&root, error)),
        };
        let mut out = Vec::new();
        for (name, metadata) in fs.read_dir(".").map_err(|e| Error::io(&root, e))? {
            if metadata.is_dir
                || Path::new(&name).extension().and_then(|e| e.to_str()) != Some("json")
            {
                continue;
            }
            let bytes = match fs.read_file(&name) {
                Ok(bytes) => bytes,
                Err(_) => continue,
            };
            let Ok(backup) = serde_json::from_slice::<Backup>(&bytes) else {
                continue;
            };
            if fs.metadata(format!("{}.tar.gz", backup.id)).is_ok() {
                out.push(backup);
            }
        }
        out.sort_by(|a, b| b.id.cmp(&a.id));
        Ok(out)
    })
    .await
    .map_err(|e| Error::Task(e.to_string()))?
}

/// Extract a backup over `server_dir`.
///
/// Files in the archive overwrite their counterparts; files added since the
/// backup are left alone. The caller must ensure the server is stopped —
/// unpacking a world under a running JVM corrupts it.
pub async fn restore(backup_dir: &Path, id: &str, server_dir: &Path) -> Result<()> {
    restore_with_limits(backup_dir, id, server_dir, ArchiveLimits::default()).await
}

/// Extract a backup using explicit archive expansion limits.
pub async fn restore_with_limits(
    backup_dir: &Path,
    id: &str,
    server_dir: &Path,
    limits: ArchiveLimits,
) -> Result<()> {
    restore_with_limits_and_quota(backup_dir, id, server_dir, limits, u64::MAX).await
}

/// Extract a backup using explicit archive and server-disk limits.
///
/// The restore is all-or-nothing. Every archive entry is first expanded into a
/// staging directory inside the server folder; only once the whole archive has
/// been read, validated and quota-checked are the staged files moved into
/// place. A failure during the move phase rolls the already-moved files back
/// from the originals it displaced, so a failed restore leaves the server tree
/// as it was rather than a mixture of old and restored files.
///
/// The quota is evaluated against the resulting server tree. Replacing an
/// existing regular file subtracts its current size before the restored bytes
/// are added, so a smaller replacement can free space while a larger one
/// consumes it. Staging means the transient peak usage is higher than that
/// projection, because the old and new copies of a file briefly coexist.
pub async fn restore_with_limits_and_quota(
    backup_dir: &Path,
    id: &str,
    server_dir: &Path,
    limits: ArchiveLimits,
    max_server_disk_bytes: u64,
) -> Result<()> {
    validate_id(id)?;
    let id = id.to_owned();

    tokio::fs::create_dir_all(server_dir)
        .await
        .map_err(|e| Error::io(server_dir, e))?;

    let archive_root = backup_dir.to_path_buf();
    let target = server_dir.to_path_buf();
    tokio::task::spawn_blocking(move || -> Result<()> {
        let archive_fs = match ScopedFs::open(&archive_root) {
            Ok(fs) => fs,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(Error::BackupNotFound(id.to_string()))
            }
            Err(error) => return Err(Error::io(&archive_root, error)),
        };
        let archive_name = format!("{id}.tar.gz");
        let file = match archive_fs.open_file(&archive_name) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(Error::BackupNotFound(id.to_string()))
            }
            Err(error) => return Err(Error::io(archive_root.join(&archive_name), error)),
        };
        let target_fs = ScopedFs::open(&target).map_err(|e| Error::io(&target, e))?;

        // Staging lives inside the server directory so that moving a staged
        // file into place is a same-filesystem rename, which cannot fail
        // half-way through a file's contents.
        let staging: PathBuf = PathBuf::from(format!(
            "{}{}",
            RESTORE_STAGING_PREFIX,
            Uuid::new_v4().simple()
        ));
        let staged_tree = staging.join("new");
        let displaced_tree = staging.join("old");
        target_fs
            .create_dir_all(&staged_tree)
            .map_err(|e| Error::io(&staged_tree, e))?;
        target_fs
            .create_dir_all(&displaced_tree)
            .map_err(|e| Error::io(&displaced_tree, e))?;

        let outcome = expand_and_commit(
            &target_fs,
            &target,
            file,
            limits,
            max_server_disk_bytes,
            &staged_tree,
            &displaced_tree,
        );

        // Whatever happened, the staging tree is scratch space and never part
        // of the server the operator sees.
        let _ = target_fs.remove(&staging);
        outcome
    })
    .await
    .map_err(|e| Error::Task(e.to_string()))??;

    Ok(())
}

/// Prefix shared by every transient file and directory a restore creates.
const RESTORE_STAGING_PREFIX: &str = ".mcpanel-restore-";

/// Expand an archive into `staged_tree`, then move it over the server tree.
///
/// Split out of [`restore_with_limits_and_quota`] so the caller can remove the
/// staging tree on every exit path with a single statement.
fn expand_and_commit(
    target_fs: &ScopedFs,
    target: &Path,
    archive: std::fs::File,
    limits: ArchiveLimits,
    max_server_disk_bytes: u64,
    staged_tree: &Path,
    displaced_tree: &Path,
) -> Result<()> {
    let mut server_usage = target_fs
        .directory_size(".")
        .map_err(|e| Error::io(target, e))?;
    let mut tar = tar::Archive::new(GzDecoder::new(archive));
    let mut entries = 0_usize;
    let mut total_bytes = 0_u64;
    // Relative paths, in archive order, that the commit phase must publish.
    let mut directories: Vec<PathBuf> = Vec::new();
    let mut files: Vec<PathBuf> = Vec::new();

    for entry in tar.entries().map_err(Error::PlainIo)? {
        let mut entry = entry.map_err(Error::PlainIo)?;
        let path = entry.path().map_err(Error::PlainIo)?.into_owned();

        // An archive is untrusted input: a crafted member path must not be
        // able to write outside the server directory. `./` is harmless and
        // is what `tar -C dir .` emits for every entry.
        if !safe_member(&path) {
            return Err(Error::UnsafeArchiveEntry(path.display().to_string()));
        }

        entries = entries.saturating_add(1);
        if entries > limits.max_entries {
            return Err(Error::ArchiveLimit("too many entries".into()));
        }

        let entry_type = entry.header().entry_type();
        if entry_type.is_symlink() || entry_type.is_hard_link() {
            // Links are not needed in a panel backup and are rejected
            // completely. Never let tar::unpack resolve a link target.
            return Err(Error::UnsafeArchiveEntry(path.display().to_string()));
        }
        let relative = normalise_member(&path)?;
        if entry_type.is_dir() {
            if !relative.as_os_str().is_empty() {
                target_fs
                    .create_dir_all(staged_tree.join(&relative))
                    .map_err(|e| Error::io(&path, e))?;
                directories.push(relative);
            }
            continue;
        }
        if !entry_type.is_file() {
            return Err(Error::UnsafeArchiveEntry(path.display().to_string()));
        }
        let declared = entry.size();
        if declared > limits.max_file_bytes {
            return Err(Error::ArchiveLimit("file is too large".into()));
        }
        if relative.as_os_str().is_empty() {
            return Err(Error::UnsafeArchiveEntry(path.display().to_string()));
        }
        let staged = staged_tree.join(&relative);
        if let Some(parent) = staged.parent() {
            target_fs
                .create_dir_all(parent)
                .map_err(|e| Error::io(parent, e))?;
        }
        // The quota is measured against the tree the restore will produce, so
        // a file that replaces an existing one only costs its size difference.
        let replaced_size = target_fs
            .metadata(&relative)
            .ok()
            .filter(|metadata| metadata.is_file)
            .map(|metadata| metadata.len)
            .unwrap_or(0);
        let mut output = target_fs
            .create_new_file(&staged)
            .map_err(|e| Error::io(&staged, e))?;
        let copied = (|| -> Result<u64> {
            let copied = copy_limited(
                &mut entry,
                &mut output,
                total_bytes,
                limits.max_total_bytes,
                limits.max_file_bytes,
                RestoreQuota {
                    current_usage: server_usage,
                    replaced_size,
                    max_server_disk_bytes,
                },
            )?;
            output.sync_all().map_err(Error::PlainIo)?;
            Ok(copied)
        })();
        drop(output);
        let copied = copied?;
        total_bytes = total_bytes
            .checked_add(copied)
            .ok_or_else(|| Error::ArchiveLimit("expanded size overflow".into()))?;
        server_usage = server_usage
            .checked_sub(replaced_size)
            .and_then(|usage| usage.checked_add(copied))
            .ok_or(Error::ServerDiskQuotaExceeded)?;
        if server_usage > max_server_disk_bytes {
            return Err(Error::ServerDiskQuotaExceeded);
        }
        files.push(relative);
    }

    commit_staged_tree(target_fs, staged_tree, displaced_tree, &directories, &files)
}

/// Move a fully expanded staging tree over the live server directory.
///
/// Directories are created first and are never rolled back: an empty directory
/// the archive would have created anyway is harmless, whereas removing one
/// risks deleting a directory that already held the operator's files.
fn commit_staged_tree(
    target_fs: &ScopedFs,
    staged_tree: &Path,
    displaced_tree: &Path,
    directories: &[PathBuf],
    files: &[PathBuf],
) -> Result<()> {
    for directory in directories {
        target_fs
            .create_dir_all(directory)
            .map_err(|e| Error::io(directory, e))?;
    }

    // Each published file records whether it displaced an original, so a
    // failure part-way through can put the server tree back as it was.
    let mut published: Vec<(&Path, bool)> = Vec::with_capacity(files.len());
    for relative in files {
        match publish_one(target_fs, staged_tree, displaced_tree, relative) {
            Ok(displaced) => published.push((relative.as_path(), displaced)),
            Err(error) => {
                rollback_published(target_fs, displaced_tree, &published);
                return Err(error);
            }
        }
    }
    Ok(())
}

/// Publish one staged file, reporting whether it displaced an existing entry.
fn publish_one(
    target_fs: &ScopedFs,
    staged_tree: &Path,
    displaced_tree: &Path,
    relative: &Path,
) -> Result<bool> {
    if let Some(parent) = relative.parent() {
        if !parent.as_os_str().is_empty() {
            target_fs
                .create_dir_all(parent)
                .map_err(|e| Error::io(parent, e))?;
        }
    }
    let saved = displaced_tree.join(relative);
    let existing = target_fs.metadata(relative).ok();
    if existing
        .as_ref()
        .is_some_and(|metadata| metadata.is_dir && !metadata.is_symlink)
    {
        // The archive holds a file where the server holds a directory. Moving
        // the directory aside would silently discard everything inside it, so
        // the conflict is reported and the restore rolled back instead.
        return Err(Error::io(
            relative,
            std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "a directory occupies the path of a restored file",
            ),
        ));
    }
    let displaced = if existing.is_some() {
        if let Some(parent) = saved.parent() {
            target_fs
                .create_dir_all(parent)
                .map_err(|e| Error::io(parent, e))?;
        }
        if target_fs.metadata(&saved).is_ok() {
            // A duplicate member for the same path: the entry already here was
            // published by this restore, so the journal must keep the true
            // original rather than be overwritten by it.
            target_fs
                .remove(relative)
                .map_err(|e| Error::io(relative, e))?;
        } else {
            target_fs
                .rename(relative, &saved)
                .map_err(|e| Error::io(relative, e))?;
        }
        true
    } else {
        false
    };
    if let Err(error) = target_fs.rename(staged_tree.join(relative), relative) {
        // Put this file's own original back before reporting, so the caller
        // only has to undo the entries it recorded as published.
        if displaced {
            let _ = target_fs.rename(&saved, relative);
        }
        return Err(Error::io(relative, error));
    }
    Ok(displaced)
}

/// Undo published files, newest first, restoring anything they displaced.
///
/// Best effort by nature: the filesystem has already accepted these renames
/// once, so the reverse renames are expected to succeed, and there is nothing
/// better to do than continue if one does not.
fn rollback_published(target_fs: &ScopedFs, displaced_tree: &Path, published: &[(&Path, bool)]) {
    for (relative, displaced) in published.iter().rev() {
        let _ = target_fs.remove(relative);
        if *displaced {
            let _ = target_fs.rename(displaced_tree.join(relative), relative);
        }
    }
}

/// Remove a backup and its metadata.
pub async fn delete(backup_dir: &Path, id: &str) -> Result<()> {
    validate_id(id)?;

    let fs = match ScopedFs::open(backup_dir) {
        Ok(fs) => fs,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(Error::BackupNotFound(id.to_string()))
        }
        Err(error) => return Err(Error::io(backup_dir, error)),
    };
    let archive = archive_path(backup_dir, id);
    fs.remove(format!("{id}.tar.gz")).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            Error::BackupNotFound(id.to_string())
        } else {
            Error::io(&archive, e)
        }
    })?;
    let _ = fs.remove(format!("{id}.json"));
    Ok(())
}

/// The archive for `id`, for legacy callers that need its display path.
///
/// The existence check is descriptor-relative and rejects a final symlink. A
/// caller that subsequently opens the returned path must still use
/// ScopedFs::open and ScopedFs::open_file for race-resistant access; the
/// panel's download endpoint does exactly that.
pub fn path_of(backup_dir: &Path, id: &str) -> Result<PathBuf> {
    validate_id(id)?;
    let fs = match ScopedFs::open(backup_dir) {
        Ok(fs) => fs,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(Error::BackupNotFound(id.to_string()))
        }
        Err(error) => return Err(Error::io(backup_dir, error)),
    };
    let name = format!("{}.tar.gz", id);
    let archive = archive_path(backup_dir, id);
    if fs
        .metadata(&name)
        .map(|metadata| metadata.is_file)
        .unwrap_or(false)
    {
        Ok(archive)
    } else {
        Err(Error::BackupNotFound(id.to_string()))
    }
}

/// Every file under `root`, recursively.
fn walk_scoped(fs: &ScopedFs, root: &Path) -> Result<Vec<(PathBuf, EntryMetadata)>> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];

    while let Some(dir) = stack.pop() {
        for (name, metadata) in fs.read_dir(&dir).map_err(|e| Error::io(&dir, e))? {
            let path = if dir == Path::new(".") {
                PathBuf::from(&name)
            } else {
                dir.join(&name)
            };
            if metadata.is_dir {
                stack.push(path);
            } else if metadata.is_file {
                out.push((path, metadata));
            }
        }
    }

    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

fn normalise_member(path: &Path) -> Result<PathBuf> {
    let mut out = PathBuf::new();
    for component in path.components() {
        if let Component::Normal(name) = component {
            out.push(name);
        }
    }
    Ok(out)
}

fn copy_limited<R: Read, W: Write>(
    reader: &mut R,
    writer: &mut W,
    already: u64,
    max_total: u64,
    max_file: u64,
    quota: RestoreQuota,
) -> Result<u64> {
    let mut buffer = [0_u8; 64 * 1024];
    let mut file_bytes = 0_u64;
    loop {
        let read = reader.read(&mut buffer).map_err(Error::PlainIo)?;
        if read == 0 {
            break;
        }
        let next_file = file_bytes
            .checked_add(read as u64)
            .ok_or_else(|| Error::ArchiveLimit("file size overflow".into()))?;
        let next_total = already
            .checked_add(next_file)
            .ok_or_else(|| Error::ArchiveLimit("expanded size overflow".into()))?;
        if next_file > max_file {
            return Err(Error::ArchiveLimit("file is too large".into()));
        }
        if next_total > max_total {
            return Err(Error::ArchiveLimit("expanded archive is too large".into()));
        }
        let resulting_usage = quota
            .current_usage
            .checked_sub(quota.replaced_size)
            .and_then(|usage| usage.checked_add(next_file))
            .ok_or(Error::ServerDiskQuotaExceeded)?;
        if resulting_usage > quota.max_server_disk_bytes {
            return Err(Error::ServerDiskQuotaExceeded);
        }
        writer.write_all(&buffer[..read]).map_err(Error::PlainIo)?;
        file_bytes = next_file;
    }
    Ok(file_bytes)
}

#[derive(Clone, Copy)]
struct RestoreQuota {
    current_usage: u64,
    replaced_size: u64,
    max_server_disk_bytes: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;

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

    #[cfg(unix)]
    #[test]
    fn legacy_archive_path_lookup_rejects_a_final_symlink() {
        let tmp = tempfile::tempdir().unwrap();
        let outside = tmp.path().join("outside.tar.gz");
        let backups = tmp.path().join("backups");
        std::fs::create_dir_all(&backups).unwrap();
        std::fs::write(&outside, b"outside").unwrap();
        std::os::unix::fs::symlink(&outside, backups.join("evil.tar.gz")).unwrap();

        assert!(matches!(
            path_of(&backups, "evil"),
            Err(Error::BackupNotFound(_)) | Err(Error::Io { .. })
        ));
        assert_eq!(std::fs::read(&outside).unwrap(), b"outside");
    }

    #[tokio::test]
    async fn restoring_an_unknown_backup_reports_not_found() {
        let tmp = tempfile::tempdir().unwrap();
        let err = restore(&tmp.path().join("b"), "20260101-000000", tmp.path())
            .await
            .unwrap_err();
        assert!(matches!(err, Error::BackupNotFound(_)));
    }

    #[tokio::test]
    async fn concurrent_backups_get_distinct_collision_resistant_ids() {
        let root = tempfile::tempdir().unwrap();
        let server = root.path().join("server");
        let backups = root.path().join("backups");
        std::fs::create_dir_all(&server).unwrap();
        std::fs::write(server.join("world.dat"), b"world").unwrap();

        let (first, second) = tokio::join!(
            create(&server, &backups, "first".into()),
            create(&server, &backups, "second".into()),
        );
        let first = first.unwrap();
        let second = second.unwrap();
        assert_ne!(first.id, second.id);
        assert_eq!(list(&backups).await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn backup_output_limit_removes_partial_archives() {
        let root = tempfile::tempdir().unwrap();
        let server = root.path().join("server");
        let backups = root.path().join("backups");
        std::fs::create_dir_all(&server).unwrap();
        std::fs::write(server.join("world.dat"), vec![b'x'; 4096]).unwrap();

        let error = create_with_limit(&server, &backups, "too small".into(), 1)
            .await
            .unwrap_err();
        assert!(matches!(error, Error::Io { .. } | Error::PlainIo(_)));
        assert!(list(&backups).await.unwrap().is_empty());
        assert_eq!(
            std::fs::read_dir(&backups)
                .unwrap()
                .filter_map(std::result::Result::ok)
                .count(),
            0
        );
    }

    #[tokio::test]
    async fn a_restore_that_fails_on_a_later_entry_leaves_the_server_untouched() {
        let tmp = tempfile::tempdir().unwrap();
        let backups = tmp.path().join("backups");
        let server = tmp.path().join("server");

        // Three files the restore would replace, and one the archive never
        // mentions, so the assertions cover both merge behaviours.
        write(&server.join("one.txt"), "original one");
        write(&server.join("nested/two.txt"), "original two");
        write(&server.join("three.txt"), "original three");
        write(&server.join("untouched.txt"), "kept");
        let before: Vec<(PathBuf, String)> =
            ["one.txt", "nested/two.txt", "three.txt", "untouched.txt"]
                .iter()
                .map(|name| {
                    let path = server.join(name);
                    let body = std::fs::read_to_string(&path).unwrap();
                    (path, body)
                })
                .collect();

        // The first two entries fit the quota; the third pushes the projected
        // tree over it, so the restore must fail after two files were staged.
        make_archive(&backups, "partial", |builder| {
            append_file(builder, "one.txt", b"new one");
            append_file(builder, "nested/two.txt", b"new two");
            append_file(builder, "three.txt", &vec![b'x'; 4096]);
        });

        let error = restore_with_limits_and_quota(
            &backups,
            "partial",
            &server,
            ArchiveLimits::default(),
            256,
        )
        .await
        .unwrap_err();
        assert!(matches!(error, Error::ServerDiskQuotaExceeded));

        for (path, body) in before {
            assert_eq!(std::fs::read_to_string(&path).unwrap(), body);
        }
        assert!(!leftover_staging(&server));
    }

    #[tokio::test]
    async fn a_restore_that_fails_while_publishing_rolls_the_server_back() {
        let tmp = tempfile::tempdir().unwrap();
        let backups = tmp.path().join("backups");
        let server = tmp.path().join("server");

        write(&server.join("first.txt"), "original first");
        // `blocked` already exists as a non-empty directory, so publishing a
        // regular file over it fails during the commit phase rather than while
        // the archive is being expanded.
        write(&server.join("blocked/occupant.txt"), "occupant");

        make_archive(&backups, "conflict", |builder| {
            append_file(builder, "first.txt", b"new first");
            append_file(builder, "blocked", b"new blocked");
        });

        let error = restore(&backups, "conflict", &server).await.unwrap_err();
        assert!(matches!(error, Error::Io { .. } | Error::PlainIo(_)));

        assert_eq!(
            std::fs::read_to_string(server.join("first.txt")).unwrap(),
            "original first"
        );
        assert_eq!(
            std::fs::read_to_string(server.join("blocked/occupant.txt")).unwrap(),
            "occupant"
        );
        assert!(!leftover_staging(&server));
    }

    /// Whether any transient restore artifact survived in a server directory.
    fn leftover_staging(server: &Path) -> bool {
        std::fs::read_dir(server)
            .unwrap()
            .filter_map(std::result::Result::ok)
            .any(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(RESTORE_STAGING_PREFIX)
            })
    }

    fn make_archive(
        backup_dir: &Path,
        id: &str,
        add: impl FnOnce(&mut tar::Builder<GzEncoder<File>>),
    ) {
        std::fs::create_dir_all(backup_dir).unwrap();
        let file = File::create(backup_dir.join(format!("{id}.tar.gz"))).unwrap();
        let mut builder = tar::Builder::new(GzEncoder::new(file, Compression::default()));
        add(&mut builder);
        let encoder = builder.into_inner().unwrap();
        encoder.finish().unwrap();
    }

    fn append_file(builder: &mut tar::Builder<GzEncoder<File>>, name: &str, body: &[u8]) {
        let mut header = tar::Header::new_gnu();
        header.set_path("placeholder").unwrap();
        let path = name.as_bytes();
        assert!(path.len() <= 100);
        let bytes = header.as_mut_bytes();
        bytes[..100].fill(0);
        bytes[..path.len()].copy_from_slice(path);
        header.set_size(body.len() as u64);
        header.set_cksum();
        builder.append(&header, body).unwrap();
    }

    fn append_link(
        builder: &mut tar::Builder<GzEncoder<File>>,
        name: &str,
        target: &str,
        hard: bool,
    ) {
        let mut header = tar::Header::new_gnu();
        header.set_path(name).unwrap();
        header.set_entry_type(if hard {
            tar::EntryType::hard_link()
        } else {
            tar::EntryType::symlink()
        });
        header.set_link_name(target).unwrap();
        header.set_size(0);
        header.set_cksum();
        builder.append(&header, std::io::empty()).unwrap();
    }

    #[tokio::test]
    async fn traversal_archive_member_cannot_write_outside_the_server() {
        let tmp = tempfile::tempdir().unwrap();
        let backups = tmp.path().join("backups");
        let server = tmp.path().join("server");
        make_archive(&backups, "evil", |builder| {
            append_file(builder, "../../outside.txt", b"owned");
        });

        let error = restore_with_limits(&backups, "evil", &server, ArchiveLimits::default())
            .await
            .unwrap_err();

        assert!(matches!(error, Error::UnsafeArchiveEntry(_)));
        assert!(!tmp.path().join("outside.txt").exists());
    }

    #[tokio::test]
    async fn links_and_preexisting_symlink_parents_are_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let backups = tmp.path().join("backups");
        let server = tmp.path().join("server");

        make_archive(&backups, "symlink", |builder| {
            append_link(builder, "escape", "../../outside", false);
        });
        assert!(matches!(
            restore_with_limits(&backups, "symlink", &server, ArchiveLimits::default())
                .await
                .unwrap_err(),
            Error::UnsafeArchiveEntry(_)
        ));

        make_archive(&backups, "hardlink", |builder| {
            append_link(builder, "escape", "../../outside", true);
        });
        assert!(matches!(
            restore_with_limits(&backups, "hardlink", &server, ArchiveLimits::default())
                .await
                .unwrap_err(),
            Error::UnsafeArchiveEntry(_)
        ));

        #[cfg(unix)]
        {
            let outside = tmp.path().join("outside");
            std::fs::create_dir_all(&outside).unwrap();
            std::fs::create_dir_all(&server).unwrap();
            std::os::unix::fs::symlink(&outside, server.join("out")).unwrap();
            make_archive(&backups, "nested", |builder| {
                append_file(builder, "out/owned.txt", b"owned");
            });
            assert!(
                restore_with_limits(&backups, "nested", &server, ArchiveLimits::default())
                    .await
                    .is_err()
            );
            assert!(!outside.join("owned.txt").exists());
        }
    }

    #[tokio::test]
    async fn expansion_and_entry_count_limits_apply_to_actual_archive_output() {
        let tmp = tempfile::tempdir().unwrap();
        let backups = tmp.path().join("backups");
        let server = tmp.path().join("server");
        make_archive(&backups, "bomb", |builder| {
            append_file(builder, "expanded.txt", &vec![b'x'; 1024 * 1024]);
        });

        let error = restore_with_limits(
            &backups,
            "bomb",
            &server,
            ArchiveLimits {
                max_entries: 10,
                max_total_bytes: 1024,
                max_file_bytes: 1024 * 1024,
            },
        )
        .await
        .unwrap_err();
        assert!(matches!(error, Error::ArchiveLimit(_)));
        assert!(!server.join("expanded.txt").exists());

        make_archive(&backups, "many", |builder| {
            append_file(builder, "one", b"1");
            append_file(builder, "two", b"2");
        });
        let error = restore_with_limits(
            &backups,
            "many",
            &server,
            ArchiveLimits {
                max_entries: 1,
                ..ArchiveLimits::default()
            },
        )
        .await
        .unwrap_err();
        assert!(matches!(error, Error::ArchiveLimit(_)));
    }

    #[tokio::test]
    async fn restore_respects_the_server_quota_below_at_and_above_the_limit() {
        let limits = ArchiveLimits::default();

        let below = tempfile::tempdir().unwrap();
        let below_backups = below.path().join("backups");
        let below_server = below.path().join("server");
        make_archive(&below_backups, "below", |builder| {
            append_file(builder, "new.txt", b"1234");
        });
        restore_with_limits_and_quota(&below_backups, "below", &below_server, limits, 5)
            .await
            .unwrap();
        assert_eq!(
            ScopedFs::open(&below_server)
                .unwrap()
                .directory_size(".")
                .unwrap(),
            4
        );

        let exact = tempfile::tempdir().unwrap();
        let exact_backups = exact.path().join("backups");
        let exact_server = exact.path().join("server");
        make_archive(&exact_backups, "exact", |builder| {
            append_file(builder, "new.txt", b"12345");
        });
        restore_with_limits_and_quota(&exact_backups, "exact", &exact_server, limits, 5)
            .await
            .unwrap();
        assert_eq!(
            ScopedFs::open(&exact_server)
                .unwrap()
                .directory_size(".")
                .unwrap(),
            5
        );

        let exceed = tempfile::tempdir().unwrap();
        let exceed_backups = exceed.path().join("backups");
        let exceed_server = exceed.path().join("server");
        make_archive(&exceed_backups, "exceed", |builder| {
            append_file(builder, "new.txt", b"123456");
        });
        let error =
            restore_with_limits_and_quota(&exceed_backups, "exceed", &exceed_server, limits, 5)
                .await
                .unwrap_err();
        assert!(matches!(error, Error::ServerDiskQuotaExceeded));
        assert!(!exceed_server.join("new.txt").exists());
    }

    #[tokio::test]
    async fn restore_quota_subtracts_replaced_bytes_and_adds_larger_replacements() {
        let limits = ArchiveLimits::default();

        let smaller = tempfile::tempdir().unwrap();
        let smaller_backups = smaller.path().join("backups");
        let smaller_server = smaller.path().join("server");
        write(&smaller_server.join("keep.txt"), "1");
        write(&smaller_server.join("replace.txt"), "12345678");
        make_archive(&smaller_backups, "smaller", |builder| {
            append_file(builder, "replace.txt", b"12");
        });
        restore_with_limits_and_quota(&smaller_backups, "smaller", &smaller_server, limits, 3)
            .await
            .unwrap();
        assert_eq!(
            ScopedFs::open(&smaller_server)
                .unwrap()
                .directory_size(".")
                .unwrap(),
            3
        );

        let larger = tempfile::tempdir().unwrap();
        let larger_backups = larger.path().join("backups");
        let larger_server = larger.path().join("server");
        write(&larger_server.join("replace.txt"), "12");
        make_archive(&larger_backups, "larger", |builder| {
            append_file(builder, "replace.txt", b"12345");
        });
        restore_with_limits_and_quota(&larger_backups, "larger", &larger_server, limits, 5)
            .await
            .unwrap();
        assert_eq!(
            ScopedFs::open(&larger_server)
                .unwrap()
                .directory_size(".")
                .unwrap(),
            5
        );
    }

    #[tokio::test]
    async fn a_quota_failed_restore_removes_its_temporary_file_and_keeps_the_old_file() {
        let tmp = tempfile::tempdir().unwrap();
        let backups = tmp.path().join("backups");
        let server = tmp.path().join("server");
        write(&server.join("replace.txt"), "old");
        make_archive(&backups, "failed", |builder| {
            append_file(builder, "replace.txt", b"this is too large");
        });

        let error =
            restore_with_limits_and_quota(&backups, "failed", &server, ArchiveLimits::default(), 3)
                .await
                .unwrap_err();
        assert!(matches!(error, Error::ServerDiskQuotaExceeded));
        assert_eq!(
            std::fs::read_to_string(server.join("replace.txt")).unwrap(),
            "old"
        );
        assert_eq!(
            std::fs::read_dir(&server)
                .unwrap()
                .filter_map(std::result::Result::ok)
                .filter(|entry| {
                    entry
                        .file_name()
                        .to_string_lossy()
                        .starts_with(".mcpanel-restore-")
                })
                .count(),
            0
        );
    }
}
