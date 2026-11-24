//! Build the pinned Horizon runtime when a panel is run from a source checkout.
use crate::{Error, Result, ScopedFs};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::OnceLock;
use tokio::io::{AsyncBufReadExt, BufReader};

static BUILD: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
const LOCK: &[u8] = include_bytes!("../../../vendor/horizon/research-lock.json");

/// Find the checkout containing the running panel, if it is a source build.
pub fn checkout_for(executable: &Path) -> Option<PathBuf> {
    executable
        .ancestors()
        .find(|path| {
            path.join(".git").exists()
                && path.join("vendor/horizon/Cargo.toml").is_file()
                && path.join("scripts/install-horizon.sh").is_file()
        })
        .map(Path::to_path_buf)
}

async fn git(root: &Path, arguments: &[&str]) -> Result<String> {
    let output = crate::hidden::hidden_tokio_command("git")
        .current_dir(root)
        .args(arguments)
        .stdin(Stdio::null())
        .kill_on_drop(true)
        .output()
        .await
        .map_err(|e| Error::io(root, e))?;
    if !output.status.success() {
        return Err(Error::InvalidConfiguration(
            "cannot verify the pinned Horizon source; initialize its Git submodule".into(),
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

async fn pinned_source(root: &Path) -> Result<PathBuf> {
    let source = root.join("vendor/horizon");
    let expected = git(root, &["rev-parse", "HEAD:vendor/horizon"]).await?;
    let actual = git(&source, &["rev-parse", "HEAD"]).await?;
    let changed = git(&source, &["status", "--porcelain", "--untracked-files=no"]).await?;
    if option_env!("MCPANEL_HORIZON_REVISION") != Some(expected.as_str())
        || expected != actual
        || !changed.is_empty()
        || std::fs::read(source.join("research-lock.json"))
            .ok()
            .as_deref()
            != Some(LOCK)
    {
        return Err(Error::InvalidConfiguration(
            "Horizon source must be clean and match the revision pinned by this panel".into(),
        ));
    }
    Ok(source)
}

async fn cargo(
    executable: &Path,
    source: &Path,
    target: &Path,
    args: &[&str],
    progress: &(impl Fn(String, Option<f32>) + Send + Sync),
) -> Result<()> {
    let mut command = crate::hidden::hidden_tokio_command(executable);
    if cfg!(target_os = "android") && std::env::var_os("CARGO_BUILD_JOBS").is_none() {
        command.env("CARGO_BUILD_JOBS", "2");
    }
    let mut child = command
        .current_dir(source)
        .env("CARGO_TARGET_DIR", target)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| Error::io(source, e))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| Error::Task("cargo has no output".into()))?;
    let mut lines = BufReader::new(stderr).lines();
    let mut last = String::new();
    while let Some(line) = lines.next_line().await.map_err(|e| Error::io(source, e))? {
        if !line.trim().is_empty() {
            last = line.clone();
            progress(format!("building Horizon: {line}"), None);
        }
    }
    let status = child.wait().await.map_err(|e| Error::io(source, e))?;
    if !status.success() {
        return Err(Error::InvalidConfiguration(format!(
            "building Horizon failed: {last}"
        )));
    }
    Ok(())
}

fn copy_tree(
    source: &ScopedFs,
    destination: &ScopedFs,
    path: &Path,
    budget: &mut (usize, u64),
) -> Result<()> {
    destination
        .create_dir_all(path)
        .map_err(|e| Error::io(path, e))?;
    for (name, metadata) in source.read_dir(path).map_err(|e| Error::io(path, e))? {
        let child = path.join(name);
        if metadata.is_dir {
            copy_tree(source, destination, &child, budget)?;
        } else if metadata.is_file {
            budget.0 += 1;
            budget.1 = budget.1.saturating_add(metadata.len);
            if budget.0 > 20_000 || budget.1 > 256 * 1024 * 1024 {
                return Err(Error::InvalidConfiguration(
                    "Horizon exporter source is too large".into(),
                ));
            }
            let mut input = source.open_file(&child).map_err(|e| Error::io(&child, e))?;
            let mut output = destination
                .create_new_file(&child)
                .map_err(|e| Error::io(&child, e))?;
            std::io::copy(&mut input, &mut output).map_err(|e| Error::io(&child, e))?;
        }
    }
    Ok(())
}

/// Build and install the clean Horizon submodule without replacing an existing installation.
pub async fn build_from_checkout(
    root: &Path,
    data_dir: &Path,
    progress: impl Fn(String, Option<f32>) + Send + Sync,
) -> Result<()> {
    build_with_cargo(root, data_dir, Path::new("cargo"), progress).await
}

async fn build_with_cargo(
    root: &Path,
    data_dir: &Path,
    cargo_executable: &Path,
    progress: impl Fn(String, Option<f32>) + Send + Sync,
) -> Result<()> {
    let _guard = BUILD
        .get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await;
    let root = std::fs::canonicalize(root).map_err(|e| Error::io(root, e))?;
    let data_dir = std::fs::canonicalize(data_dir).map_err(|e| Error::io(data_dir, e))?;
    let destination = data_dir.join("horizon");
    let runtime_present = crate::sandbox::horizon_programs_present(&destination);
    if runtime_present && destination.join("prepare").exists() {
        return Ok(());
    }
    if !runtime_present && destination.exists() {
        return Err(Error::InvalidConfiguration(
            "an incomplete Horizon installation exists; inspect it before retrying".into(),
        ));
    }
    let source = pinned_source(&root).await?;
    let build_dir = data_dir.join(".horizon-build");
    progress(
        "building the pinned Horizon runtime (this can take several minutes)".into(),
        None,
    );
    if !runtime_present {
        cargo(
            cargo_executable,
            &source,
            &build_dir,
            &[
                "build",
                "--locked",
                "--profile",
                "fast",
                "-p",
                "horizon-game",
                "--no-default-features",
                "--bin",
                "horizon-client",
            ],
            &progress,
        )
        .await?;
    }
    let server_args = [
        "build",
        "--locked",
        "--profile",
        "fast",
        "-p",
        "horizon-server",
        "--bin",
        "horizon-server",
        "-p",
        "horizon-prepare",
        "--bin",
        "horizon-prepare",
    ];
    let preparer_args = [
        "build",
        "--locked",
        "--profile",
        "fast",
        "-p",
        "horizon-prepare",
        "--bin",
        "horizon-prepare",
    ];
    cargo(
        cargo_executable,
        &source,
        &build_dir,
        if runtime_present {
            preparer_args.as_slice()
        } else {
            server_args.as_slice()
        },
        &progress,
    )
    .await?;
    // Check again after a long build, before copying or publishing anything.
    pinned_source(&root).await?;
    let stage = tempfile::Builder::new()
        .prefix(".horizon-build-install-")
        .tempdir_in(&data_dir)
        .map_err(|e| Error::io(&data_dir, e))?;
    let stage_fs = ScopedFs::open(stage.path()).map_err(|e| Error::io(stage.path(), e))?;
    stage_fs
        .set_private()
        .map_err(|e| Error::io(stage.path(), e))?;
    let source_fs = ScopedFs::open(&source).map_err(|e| Error::io(&source, e))?;
    let prepare = stage.path().join("prepare");
    std::fs::create_dir(&prepare).map_err(|e| Error::io(&prepare, e))?;
    let prepare_fs = ScopedFs::open(&prepare).map_err(|e| Error::io(&prepare, e))?;
    copy_tree(
        &source_fs,
        &prepare_fs,
        Path::new("mods/horizon/src/test/java"),
        &mut (0, 0),
    )?;
    std::fs::write(prepare.join("research-lock.json"), LOCK).map_err(|e| Error::io(&prepare, e))?;
    let suffix = std::env::consts::EXE_SUFFIX;
    for (from, to) in [
        (
            format!("horizon-client{suffix}"),
            format!("horizon-engine{suffix}"),
        ),
        (
            format!("horizon-server{suffix}"),
            format!("horizon-server{suffix}"),
        ),
        (
            format!("horizon-prepare{suffix}"),
            format!("prepare/horizon-prepare{suffix}"),
        ),
    ] {
        if runtime_present && !to.starts_with("prepare/") {
            continue;
        }
        let input = build_dir.join("fast").join(from);
        let output = stage.path().join(to);
        std::fs::copy(&input, &output).map_err(|e| Error::io(&input, e))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&output, std::fs::Permissions::from_mode(0o700))
                .map_err(|e| Error::io(&output, e))?;
        }
    }
    // Windows does not allow a directory to be renamed while its handle is open.
    drop(prepare_fs);
    if runtime_present {
        // Preserve the existing server and engine, adding only the missing
        // preparer. A newly claimed directory is never merged with user data.
        ScopedFs::open(&destination).map_err(|e| Error::io(&destination, e))?;
        let target = destination.join("prepare");
        std::fs::create_dir(&target).map_err(|e| Error::io(&target, e))?;
        struct Cleanup<'a>(&'a Path, bool);
        impl Drop for Cleanup<'_> {
            fn drop(&mut self) {
                if !self.1 {
                    let _ = std::fs::remove_dir_all(self.0);
                }
            }
        }
        let mut cleanup = Cleanup(&target, false);
        for entry in
            std::fs::read_dir(stage.path().join("prepare")).map_err(|e| Error::io(&target, e))?
        {
            let entry = entry.map_err(|e| Error::io(&target, e))?;
            std::fs::rename(entry.path(), target.join(entry.file_name()))
                .map_err(|e| Error::io(&target, e))?;
        }
        cleanup.1 = true;
        progress("Horizon resource preparer installed".into(), Some(1.0));
        return Ok(());
    }
    // Claim a fresh directory and publish the supervisor last.
    std::fs::create_dir(&destination).map_err(|e| Error::io(&destination, e))?;
    for name in [
        "prepare",
        &format!("horizon-engine{suffix}"),
        &format!("horizon-server{suffix}"),
    ] {
        std::fs::rename(stage.path().join(name), destination.join(name))
            .map_err(|e| Error::io(&destination, e))?;
    }
    progress("Horizon runtime installed".into(), Some(1.0));
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[tokio::test]
    async fn source_build_installs_pinned_runtime_once_without_replacing_it() {
        let temp = tempfile::tempdir().unwrap();
        let data = temp.path().join("data");
        std::fs::create_dir(&data).unwrap();
        let cargo = temp.path().join("fake-cargo");
        std::fs::write(&cargo, b"#!/bin/sh\nset -eu\nmkdir -p \"$CARGO_TARGET_DIR/fast\"\nfor name in horizon-client horizon-server horizon-prepare; do printf '#!/bin/sh\\nexit 0\\n' > \"$CARGO_TARGET_DIR/fast/$name\"; chmod 700 \"$CARGO_TARGET_DIR/fast/$name\"; done\nprintf 'compiling fixture\\n' >&2\n").unwrap();
        std::fs::set_permissions(&cargo, std::fs::Permissions::from_mode(0o700)).unwrap();
        let checkout = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        build_with_cargo(&checkout, &data, &cargo, |_, _| {})
            .await
            .unwrap();
        assert!(crate::sandbox::horizon_programs_present(
            &data.join("horizon")
        ));
        assert!(data.join("horizon/prepare/horizon-prepare").is_file());
        assert!(data.join("horizon/prepare/mods/horizon/src/test/java/dev/horizonlab/parity/PrepareWorker.java").is_file());
        std::fs::write(data.join("horizon/keep"), b"original").unwrap();
        build_with_cargo(&checkout, &data, Path::new("does-not-exist"), |_, _| {})
            .await
            .unwrap();
        assert_eq!(
            std::fs::read(data.join("horizon/keep")).unwrap(),
            b"original"
        );
        std::fs::remove_dir_all(data.join("horizon/prepare")).unwrap();
        let engine = std::fs::read(data.join("horizon/horizon-engine")).unwrap();
        build_with_cargo(&checkout, &data, &cargo, |_, _| {})
            .await
            .unwrap();
        assert!(data.join("horizon/prepare/horizon-prepare").is_file());
        assert_eq!(
            std::fs::read(data.join("horizon/horizon-engine")).unwrap(),
            engine
        );
    }
}

#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;

    #[tokio::test]
    async fn source_build_stages_windows_programs_and_preparer() {
        let temp = tempfile::tempdir().unwrap();
        let data = temp.path().join("data");
        std::fs::create_dir(&data).unwrap();
        let cargo = temp.path().join("fake-cargo.cmd");
        std::fs::write(&cargo, b"@echo off\r\nif not exist \"%CARGO_TARGET_DIR%\\fast\" mkdir \"%CARGO_TARGET_DIR%\\fast\"\r\necho client>\"%CARGO_TARGET_DIR%\\fast\\horizon-client.exe\"\r\necho server>\"%CARGO_TARGET_DIR%\\fast\\horizon-server.exe\"\r\necho prep>\"%CARGO_TARGET_DIR%\\fast\\horizon-prepare.exe\"\r\n").unwrap();
        let checkout = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        build_with_cargo(&checkout, &data, &cargo, |_, _| {})
            .await
            .unwrap();
        assert!(crate::sandbox::horizon_programs_present(
            &data.join("horizon")
        ));
        assert!(data.join("horizon/prepare/horizon-prepare.exe").is_file());
        assert!(data.join("horizon/prepare/mods/horizon/src/test/java/dev/horizonlab/parity/PrepareWorker.java").is_file());
    }
}
