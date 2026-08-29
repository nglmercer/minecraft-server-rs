//! Capability-based access to one server directory.
//!
//! The panel accepts paths from an authenticated but potentially hostile server
//! operator.  A lexical `PathBuf::join` followed by `starts_with` is not a
//! filesystem boundary: an attacker can replace a component with a symlink
//! between the check and the operation.  [`ScopedFs`] keeps an open directory
//! capability and resolves every component relative to directory handles.  All
//! directory components are opened without following symlinks and final files
//! are opened with the same rule.

use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
use cap_std::ambient_authority;
use cap_std::fs::{Dir, OpenOptions};
use std::ffi::{OsStr, OsString};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Metadata returned for an entry in a scoped directory.
#[derive(Debug, Clone)]
pub struct EntryMetadata {
    /// Whether the entry is a directory.
    pub is_dir: bool,
    /// Whether the entry is a regular file.
    pub is_file: bool,
    /// Whether the entry is a symbolic link.
    pub is_symlink: bool,
    /// File length when the platform reports one.
    pub len: u64,
    /// Modification time when the platform reports one.
    pub modified_epoch_secs: Option<u64>,
}

/// A directory capability rooted at one server directory.
pub struct ScopedFs {
    root: PathBuf,
    dir: Dir,
}

impl std::fmt::Debug for ScopedFs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScopedFs")
            .field("root", &self.root)
            .finish()
    }
}

impl ScopedFs {
    /// Open `root` as a directory capability.
    ///
    /// The root is opened one component at a time without following the final
    /// component. Operations after this point use the open handle rather than
    /// resolving the root path again.
    pub fn open(root: impl AsRef<Path>) -> io::Result<Self> {
        let requested = root.as_ref();
        let root = if requested.is_absolute() {
            requested.to_path_buf()
        } else {
            std::env::current_dir()?.join(requested)
        };
        let (root, dir) = open_absolute_nofollow(&root)?;
        Ok(Self { root, dir })
    }

    /// The canonical path used for diagnostics and display only.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Duplicate the directory capability for another blocking operation.
    pub fn try_clone(&self) -> io::Result<Self> {
        Ok(Self {
            root: self.root.clone(),
            dir: self.dir.try_clone()?,
        })
    }

    /// Restrict the capability's root directory to its owning Unix user.
    /// Windows has no portable mode-bit equivalent, so ACLs remain the
    /// administrator's responsibility there.
    pub fn set_private(&self) -> io::Result<()> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            self.dir.set_permissions(
                ".",
                cap_std::fs::Permissions::from_std(std::fs::Permissions::from_mode(0o700)),
            )?;
        }
        Ok(())
    }

    /// Read a regular file without following a final symlink.
    pub fn read_file(&self, path: impl AsRef<Path>) -> io::Result<Vec<u8>> {
        let mut file = self.open_file(path)?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        Ok(bytes)
    }

    /// Open a regular file for reading without following symlinks.
    pub fn open_file(&self, path: impl AsRef<Path>) -> io::Result<std::fs::File> {
        let (parent, name) = self.parent_and_name(path.as_ref(), false)?;
        let mut options = OpenOptions::new();
        options.read(true).follow(FollowSymlinks::No);
        let file = parent.open_with(name, &options)?;
        Ok(file.into_std())
    }

    /// Open a file and restrict its Unix mode to owner read/write access.
    pub fn set_file_private(&self, path: impl AsRef<Path>) -> io::Result<()> {
        let file = self.open_file(path)?;
        restrict_file_permissions(&file)
    }

    /// Open a file for truncating writes without following symlinks.
    pub fn create_file(&self, path: impl AsRef<Path>) -> io::Result<std::fs::File> {
        let (parent, name) = self.parent_and_name(path.as_ref(), true)?;
        let mut options = OpenOptions::new();
        options
            .write(true)
            .create(true)
            .truncate(true)
            .follow(FollowSymlinks::No);
        let file = parent.open_with(name, &options)?;
        let file = file.into_std();
        restrict_file_permissions(&file)?;
        Ok(file)
    }

    /// Create a new file, failing if the final name already exists.
    pub fn create_new_file(&self, path: impl AsRef<Path>) -> io::Result<std::fs::File> {
        let (parent, name) = self.parent_and_name(path.as_ref(), true)?;
        let mut options = OpenOptions::new();
        options
            .write(true)
            .create_new(true)
            .follow(FollowSymlinks::No);
        let file = parent.open_with(name, &options)?;
        let file = file.into_std();
        restrict_file_permissions(&file)?;
        Ok(file)
    }

    /// Write a file through a sibling temporary file and an atomic rename.
    pub fn write_atomic(&self, path: impl AsRef<Path>, contents: &[u8]) -> io::Result<()> {
        let path = checked_relative(path.as_ref())?;
        let (parent, name) = self.parent_and_name(&path, true)?;

        let pid = std::process::id();
        let (temporary, file) = loop {
            let temporary: OsString = format!(
                ".mcpanel-tmp-{pid}-{}",
                TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
            )
            .into();

            let mut options = OpenOptions::new();
            options
                .write(true)
                .create_new(true)
                .follow(FollowSymlinks::No);
            match parent.open_with(&temporary, &options) {
                Ok(file) => break (temporary, file),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        };

        let mut file = file.into_std();
        if let Err(error) = restrict_file_permissions(&file) {
            drop(file);
            let _ = parent.remove_file_or_symlink(&temporary);
            return Err(error);
        }
        if let Err(error) = file.write_all(contents).and_then(|_| file.sync_all()) {
            drop(file);
            let _ = parent.remove_file_or_symlink(&temporary);
            return Err(error);
        }
        drop(file);

        if let Err(error) = replace_in_parent(&parent, &temporary, &parent, &name) {
            let _ = parent.remove_file_or_symlink(&temporary);
            return Err(error);
        }
        sync_directory(&parent);
        Ok(())
    }

    /// Create a directory and all missing parents without following symlinks.
    pub fn create_dir_all(&self, path: impl AsRef<Path>) -> io::Result<()> {
        let relative = checked_relative(path.as_ref())?;
        let mut current = self.dir.try_clone()?;
        for component in relative.components() {
            let name = component.as_os_str();
            match current.create_dir(name) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
            current = current.open_dir_nofollow(name)?;
        }
        Ok(())
    }

    /// List a directory, omitting symlinks rather than presenting misleading
    /// entries that cannot safely be opened through the capability.
    pub fn read_dir(&self, path: impl AsRef<Path>) -> io::Result<Vec<(OsString, EntryMetadata)>> {
        let dir = self.open_dir(path)?;
        let mut entries = Vec::new();
        for entry in dir.entries()? {
            let entry = entry?;
            let name = entry.file_name();
            let metadata = metadata_for(&dir, &name)?;
            if metadata.is_symlink {
                continue;
            }
            entries.push((name, metadata));
        }
        Ok(entries)
    }

    /// Read metadata without following the final symlink.
    pub fn metadata(&self, path: impl AsRef<Path>) -> io::Result<EntryMetadata> {
        let path = checked_relative(path.as_ref())?;
        let (parent, name) = self.parent_and_name(&path, false)?;
        metadata_for(&parent, &name)
    }

    /// Remove a file, directory, or symlink itself, never its target.
    pub fn remove(&self, path: impl AsRef<Path>) -> io::Result<()> {
        let path = checked_relative(path.as_ref())?;
        if path.as_os_str().is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "refusing to remove the scoped root",
            ));
        }
        let (parent, name) = self.parent_and_name(&path, false)?;
        let metadata = metadata_for(&parent, &name)?;
        if metadata.is_dir && !metadata.is_symlink {
            parent.remove_dir_all(&name)
        } else {
            parent.remove_file_or_symlink(&name)
        }
    }

    /// Rename within the scoped root.  Both parent paths are opened without
    /// following symlinks, and the final destination is replaced as a name in
    /// its parent directory rather than dereferenced.
    pub fn rename(&self, from: impl AsRef<Path>, to: impl AsRef<Path>) -> io::Result<()> {
        let from = checked_relative(from.as_ref())?;
        let to = checked_relative(to.as_ref())?;
        if from.as_os_str().is_empty() || to.as_os_str().is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "refusing to rename the scoped root",
            ));
        }
        let (from_parent, from_name) = self.parent_and_name(&from, false)?;
        let (to_parent, to_name) = self.parent_and_name(&to, true)?;
        replace_in_parent(&from_parent, &from_name, &to_parent, &to_name)
    }

    /// Replace a destination file with another file in this capability.
    ///
    /// Unix `rename` atomically replaces the destination name.  Windows does
    /// not provide that behavior through the portable capability API, so the
    /// existing destination is removed as a name (never followed) before the
    /// rename.  Callers use this only after the replacement file has been
    /// completely written and synced.
    pub fn replace_file(&self, from: impl AsRef<Path>, to: impl AsRef<Path>) -> io::Result<()> {
        let from = checked_relative(from.as_ref())?;
        let to = checked_relative(to.as_ref())?;
        if from.as_os_str().is_empty() || to.as_os_str().is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "refusing to replace the scoped root",
            ));
        }
        let (from_parent, from_name) = self.parent_and_name(&from, false)?;
        let (to_parent, to_name) = self.parent_and_name(&to, true)?;

        replace_in_parent(&from_parent, &from_name, &to_parent, &to_name)?;
        sync_directory(&to_parent);
        Ok(())
    }

    /// Sum regular files below a directory without following links.
    pub fn directory_size(&self, path: impl AsRef<Path>) -> io::Result<u64> {
        let dir = self.open_dir(path)?;
        directory_size_from(&dir)
    }

    /// Open a subdirectory without following any component symlink.
    pub fn open_dir(&self, path: impl AsRef<Path>) -> io::Result<Dir> {
        let relative = checked_relative(path.as_ref())?;
        let mut current = self.dir.try_clone()?;
        for component in relative.components() {
            current = current.open_dir_nofollow(component.as_os_str())?;
        }
        Ok(current)
    }

    fn parent_and_name(&self, path: &Path, create_parents: bool) -> io::Result<(Dir, OsString)> {
        let relative = checked_relative(path)?;
        let mut components = relative.components();
        let Some(last) = components.next_back() else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "a file name is required",
            ));
        };

        let mut parent = self.dir.try_clone()?;
        for component in components {
            let name = component.as_os_str();
            if create_parents {
                match parent.create_dir(name) {
                    Ok(()) => {}
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                    Err(error) => return Err(error),
                }
            }
            parent = parent.open_dir_nofollow(name)?;
        }
        Ok((parent, last.as_os_str().to_os_string()))
    }
}

fn checked_relative(path: &Path) -> io::Result<PathBuf> {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(name) => out.push(name),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "path must be relative and must not contain parent components",
                ));
            }
        }
    }
    Ok(out)
}

/// Open an absolute path one directory handle at a time.  In particular, the
/// final component is not opened through an ambient path operation, so a
/// server directory replaced with a symlink cannot turn into a capability for
/// another server or the host filesystem.
#[cfg(unix)]
fn open_absolute_nofollow(path: &Path) -> io::Result<(PathBuf, Dir)> {
    if !path.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "root path must be absolute",
        ));
    }
    let mut current = Dir::open_ambient_dir(Path::new("/"), ambient_authority())?;
    for component in path.components() {
        match component {
            Component::RootDir | Component::CurDir => {}
            Component::Normal(name) => current = current.open_dir_nofollow(name)?,
            Component::ParentDir | Component::Prefix(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "root path must not contain parent components",
                ));
            }
        }
    }
    Ok((path.to_path_buf(), current))
}

#[cfg(windows)]
fn open_absolute_nofollow(path: &Path) -> io::Result<(PathBuf, Dir)> {
    if !path.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "root path must be absolute",
        ));
    }
    let mut components = path.components();
    let mut anchor = PathBuf::new();
    if let Some(Component::Prefix(prefix)) = components.next() {
        anchor.push(prefix.as_os_str());
    }
    match components.next() {
        Some(Component::RootDir) => anchor.push(Path::new("\\")),
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "root path must have a filesystem root",
            ));
        }
    }
    let mut current = Dir::open_ambient_dir(&anchor, ambient_authority())?;
    for component in components {
        match component {
            Component::CurDir => {}
            Component::Normal(name) => current = current.open_dir_nofollow(name)?,
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "root path must not contain parent components",
                ));
            }
        }
    }
    Ok((path.to_path_buf(), current))
}

#[cfg(not(any(unix, windows)))]
fn open_absolute_nofollow(path: &Path) -> io::Result<(PathBuf, Dir)> {
    // The project currently targets Unix and Windows.  Keep a conservative
    // fallback for other Rust targets rather than making the crate fail to
    // compile, while the server-scoped operations themselves remain
    // descriptor-relative after this initial acquisition.
    let dir = Dir::open_ambient_dir(path, ambient_authority())?;
    Ok((path.to_path_buf(), dir))
}

fn metadata_for(dir: &Dir, name: &OsStr) -> io::Result<EntryMetadata> {
    let metadata = dir.symlink_metadata(name)?;
    let file_type = metadata.file_type();
    Ok(EntryMetadata {
        is_dir: file_type.is_dir(),
        is_file: file_type.is_file(),
        is_symlink: file_type.is_symlink(),
        len: metadata.len(),
        modified_epoch_secs: metadata
            .modified()
            .ok()
            .and_then(|time| time.into_std().duration_since(SystemTime::UNIX_EPOCH).ok())
            .map(|duration| duration.as_secs()),
    })
}

fn restrict_file_permissions(file: &std::fs::File) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(not(unix))]
    let _ = file;
    Ok(())
}

fn replace_in_parent(
    from_parent: &Dir,
    from_name: &OsStr,
    to_parent: &Dir,
    to_name: &OsStr,
) -> io::Result<()> {
    #[cfg(windows)]
    if let Ok(metadata) = metadata_for(to_parent, to_name) {
        if metadata.is_dir && !metadata.is_symlink {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "destination is a directory",
            ));
        }
        to_parent.remove_file_or_symlink(to_name)?;
    }

    from_parent.rename(from_name, to_parent, to_name)
}

fn directory_size_from(dir: &Dir) -> io::Result<u64> {
    let mut total = 0_u64;
    for entry in dir.entries()? {
        let entry = entry?;
        let name = entry.file_name();
        let metadata = metadata_for(dir, &name)?;
        if metadata.is_symlink {
            continue;
        }
        if metadata.is_dir {
            total = total.saturating_add(directory_size_from(&dir.open_dir_nofollow(&name)?)?);
        } else if metadata.is_file {
            total = total.saturating_add(metadata.len);
        }
    }
    Ok(total)
}

fn sync_directory(dir: &Dir) {
    // Directory fsync is not supported on every platform.  The file was
    // already synced, and the rename remains atomic on all supported hosts.
    #[cfg(unix)]
    {
        let _ = dir
            .try_clone()
            .and_then(|dir| dir.into_std_file().sync_all());
    }
    #[cfg(not(unix))]
    let _ = dir;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_paths_are_checked_before_touching_the_filesystem() {
        let tmp = tempfile::tempdir().unwrap();
        let fs = ScopedFs::open(tmp.path()).unwrap();

        assert!(fs.metadata(Path::new("../outside")).is_err());
        assert!(fs.metadata(Path::new("/etc/passwd")).is_err());
        assert!(fs.metadata(Path::new("C:\\Windows\\win.ini")).is_err());
    }

    #[test]
    fn ordinary_files_and_directories_work() {
        let tmp = tempfile::tempdir().unwrap();
        let fs = ScopedFs::open(tmp.path()).unwrap();

        fs.create_dir_all("plugins").unwrap();
        fs.write_atomic("plugins/test.txt", b"hello").unwrap();
        assert_eq!(fs.read_file("plugins/test.txt").unwrap(), b"hello");
        assert_eq!(fs.directory_size(".").unwrap(), 5);
        fs.rename("plugins/test.txt", "plugins/renamed.txt")
            .unwrap();
        fs.remove("plugins/renamed.txt").unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn private_permissions_can_be_set_on_the_scoped_root() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::tempdir().unwrap();
        let fs = ScopedFs::open(tmp.path()).unwrap();

        fs.set_private().unwrap();

        assert_eq!(
            std::fs::metadata(tmp.path()).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_file_and_directory_components_are_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret"), b"secret").unwrap();
        std::os::unix::fs::symlink(outside.path().join("secret"), tmp.path().join("file")).unwrap();
        std::os::unix::fs::symlink(outside.path(), tmp.path().join("dir")).unwrap();
        let fs = ScopedFs::open(tmp.path()).unwrap();

        assert!(fs.open_file("file").is_err());
        assert!(fs.open_file("dir/secret").is_err());
        assert!(fs.create_file("file").is_err());
        assert!(fs.create_file("dir/new").is_err());

        // Removing a link removes only the link, never its target.
        fs.remove("file").unwrap();
        assert!(outside.path().join("secret").exists());
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_cannot_become_the_scoped_root() {
        let tmp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), tmp.path().join("server")).unwrap();

        assert!(ScopedFs::open(tmp.path().join("server")).is_err());
    }
}
