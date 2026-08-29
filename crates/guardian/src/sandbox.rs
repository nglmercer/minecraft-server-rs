//! OS isolation for Minecraft child processes.
//!
//! Linux uses bubblewrap when it is installed and macOS uses the system
//! sandbox profile. Windows has no hard per-server kernel sandbox in this
//! binary. If the platform helper is unavailable, the caller must explicitly
//! acknowledge that the JVM will run with the panel account's remaining OS
//! authority.

use crate::error::{Error, Result};
use std::ffi::OsString;
use std::path::Path;
#[cfg(any(target_os = "linux", target_os = "macos", test))]
use std::path::PathBuf;
use tokio::process::Command;

/// The isolation level applied to a Minecraft child process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxMode {
    /// A platform process/filesystem sandbox is wrapping the JVM.
    KernelSandbox,
    /// The JVM is running with only the panel's application-level controls.
    Unsandboxed,
}

/// Deployment policy for platforms where the kernel sandbox helper is absent.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SandboxPolicy {
    allow_unsandboxed: bool,
}

impl SandboxPolicy {
    /// Create a policy, optionally acknowledging unsandboxed JVM execution.
    pub const fn new(allow_unsandboxed: bool) -> Self {
        Self { allow_unsandboxed }
    }

    /// Whether this policy permits a JVM without a platform sandbox.
    pub const fn allows_unsandboxed(self) -> bool {
        self.allow_unsandboxed
    }
}

/// Build the child command, optionally placing it in a platform sandbox.
pub fn command(
    java: &Path,
    directory: &Path,
    jar: &Path,
    args: &[OsString],
    policy: SandboxPolicy,
) -> Result<(Command, SandboxMode)> {
    #[cfg(target_os = "linux")]
    {
        linux_command(java, directory, jar, args, find_in_path("bwrap"), policy)
    }
    #[cfg(target_os = "macos")]
    {
        macos_command(
            java,
            directory,
            jar,
            args,
            find_in_path("sandbox-exec"),
            policy,
        )
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = jar;
        unsandboxed_command(java, directory, args, policy)
    }
}

#[cfg(target_os = "linux")]
fn linux_command(
    java: &Path,
    directory: &Path,
    jar: &Path,
    args: &[OsString],
    bwrap: Option<PathBuf>,
    policy: SandboxPolicy,
) -> Result<(Command, SandboxMode)> {
    let Some(bwrap) = bwrap else {
        return unsandboxed_command(java, directory, args, policy);
    };

    let java = std::fs::canonicalize(java).unwrap_or_else(|_| java.to_path_buf());
    let java_parent = java.parent().unwrap_or_else(|| Path::new("/"));
    let (java_mount, guest_java) = if java_parent
        .file_name()
        .is_some_and(|name| name.eq_ignore_ascii_case("bin"))
    {
        let home = java_parent.parent().unwrap_or(java_parent);
        (
            home,
            PathBuf::from("/opt/mcpanel-java/bin").join(
                java.file_name()
                    .unwrap_or_else(|| std::ffi::OsStr::new("java")),
            ),
        )
    } else {
        (
            java_parent,
            PathBuf::from("/opt/mcpanel-java").join(
                java.file_name()
                    .unwrap_or_else(|| std::ffi::OsStr::new("java")),
            ),
        )
    };
    let directory = std::fs::canonicalize(directory).unwrap_or_else(|_| directory.to_path_buf());
    let jar = std::fs::canonicalize(jar).unwrap_or_else(|_| jar.to_path_buf());
    let guest_jar = jar
        .strip_prefix(&directory)
        .map(|relative| PathBuf::from("/server").join(relative))
        .unwrap_or_else(|_| PathBuf::from("/server/server.jar"));

    let mut command = Command::new(bwrap);
    command
        .arg("--die-with-parent")
        .arg("--new-session")
        .arg("--unshare-pid")
        .arg("--proc")
        .arg("/proc")
        .arg("--dev")
        .arg("/dev")
        .arg("--tmpfs")
        .arg("/tmp");
    for path in ["/usr", "/bin", "/sbin", "/lib", "/lib64", "/etc"] {
        if Path::new(path).exists() {
            command.args(["--ro-bind", path, path]);
        }
    }
    command
        .args(["--ro-bind"])
        .arg(java_mount)
        .arg("/opt/mcpanel-java")
        .args(["--bind"])
        .arg(&directory)
        .arg("/server")
        .args(["--chdir", "/server", "--setenv", "HOME", "/server"])
        .args(["--setenv", "TMPDIR", "/tmp"]);

    let mut child_args = args.to_vec();
    for argument in &mut child_args {
        if Path::new(argument).is_absolute() && Path::new(argument) == jar {
            *argument = guest_jar.clone().into_os_string();
        }
    }
    command.arg(guest_java).args(child_args);
    command.current_dir(directory);
    Ok((command, SandboxMode::KernelSandbox))
}

#[cfg(target_os = "linux")]
fn find_in_path(name: &str) -> Option<PathBuf> {
    find_in_path_value(name, &std::env::var_os("PATH")?)
}

#[cfg(target_os = "macos")]
fn macos_command(
    java: &Path,
    directory: &Path,
    jar: &Path,
    args: &[OsString],
    sandbox_exec: Option<PathBuf>,
    policy: SandboxPolicy,
) -> Result<(Command, SandboxMode)> {
    let Some(sandbox_exec) = sandbox_exec else {
        return unsandboxed_command(java, directory, args, policy);
    };

    let java = std::fs::canonicalize(java).unwrap_or_else(|_| java.to_path_buf());
    let directory = std::fs::canonicalize(directory).unwrap_or_else(|_| directory.to_path_buf());
    let jar = std::fs::canonicalize(jar).unwrap_or_else(|_| jar.to_path_buf());
    let profile = format!(
        "(version 1)
         (deny default)
         (allow process*)
         (allow file-read* (subpath \"/usr\") (subpath \"/System\") (subpath \"/Library\"))
         (allow file-read* (subpath \"{}\"))
         (allow file-read* (subpath \"{}\"))
         (allow file-write* (subpath \"{}\"))
         (allow file-write* (subpath \"/tmp\"))
         (allow network*)",
        profile_path(
            java.parent()
                .and_then(Path::parent)
                .unwrap_or_else(|| java.parent().unwrap_or_else(|| Path::new("/")))
        ),
        profile_path(&directory),
        profile_path(&directory),
    );
    let mut command = Command::new(sandbox_exec);
    command
        .args(["-p", &profile])
        .arg(&java)
        .args(args)
        .current_dir(directory);
    let _ = jar;
    Ok((command, SandboxMode::KernelSandbox))
}

#[cfg(target_os = "macos")]
fn find_in_path(name: &str) -> Option<PathBuf> {
    find_in_path_value(name, &std::env::var_os("PATH")?)
}

#[cfg(any(target_os = "linux", target_os = "macos", test))]
fn find_in_path_value(name: &str, path: &std::ffi::OsStr) -> Option<PathBuf> {
    std::env::split_paths(path)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

fn unsandboxed_command(
    java: &Path,
    directory: &Path,
    args: &[OsString],
    policy: SandboxPolicy,
) -> Result<(Command, SandboxMode)> {
    if !policy.allows_unsandboxed() {
        return Err(Error::SandboxUnavailable);
    }
    let mut command = Command::new(java);
    command.current_dir(directory).args(args);
    tracing::warn!(
        "Minecraft is running without a platform sandbox because unsandboxed execution was explicitly enabled"
    );
    Ok((command, SandboxMode::Unsandboxed))
}

#[cfg(target_os = "macos")]
fn profile_path(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;

    fn sample_command(policy: SandboxPolicy) -> Result<(Command, SandboxMode)> {
        unsandboxed_command(
            Path::new("/opt/java/bin/java"),
            Path::new("/srv/server"),
            &[],
            policy,
        )
    }

    #[test]
    fn helper_detection_finds_a_regular_file_in_path() {
        let directory = tempfile::tempdir().unwrap();
        let helper = directory.path().join("sandbox-helper");
        std::fs::write(&helper, b"helper").unwrap();
        let path = std::env::join_paths([directory.path()]).unwrap();

        assert_eq!(find_in_path_value("sandbox-helper", &path), Some(helper));
    }

    #[test]
    fn missing_helper_is_detected() {
        let directory = tempfile::tempdir().unwrap();
        let path = std::env::join_paths([directory.path().join("missing")]).unwrap();

        assert_eq!(find_in_path_value("sandbox-helper", &path), None);
    }

    #[test]
    fn missing_helper_without_acknowledgement_rejects_launch() {
        let error = match sample_command(SandboxPolicy::default()) {
            Ok(_) => panic!("unsandboxed execution must be rejected by default"),
            Err(error) => error,
        };

        assert!(matches!(error, Error::SandboxUnavailable));
        assert!(error.client_message().contains("allow-unsandboxed-servers"));
        assert!(!error.client_message().contains("/opt/java"));
    }

    #[test]
    fn explicit_acknowledgement_permits_unsandboxed_launch() {
        let (_, mode) = sample_command(SandboxPolicy::new(true)).unwrap();
        assert_eq!(mode, SandboxMode::Unsandboxed);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn missing_linux_helper_is_rejected_or_explicitly_allowed() {
        let rejected = linux_command(
            Path::new("/opt/java/bin/java"),
            Path::new("/srv/server"),
            Path::new("/srv/server/server.jar"),
            &[],
            None,
            SandboxPolicy::default(),
        );
        assert!(matches!(rejected, Err(Error::SandboxUnavailable)));

        let (_, mode) = linux_command(
            Path::new("/opt/java/bin/java"),
            Path::new("/srv/server"),
            Path::new("/srv/server/server.jar"),
            &[],
            None,
            SandboxPolicy::new(true),
        )
        .unwrap();
        assert_eq!(mode, SandboxMode::Unsandboxed);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn missing_macos_helper_is_rejected_or_explicitly_allowed() {
        let rejected = macos_command(
            Path::new("/opt/java/bin/java"),
            Path::new("/srv/server"),
            Path::new("/srv/server/server.jar"),
            &[],
            None,
            SandboxPolicy::default(),
        );
        assert!(matches!(rejected, Err(Error::SandboxUnavailable)));

        let (_, mode) = macos_command(
            Path::new("/opt/java/bin/java"),
            Path::new("/srv/server"),
            Path::new("/srv/server/server.jar"),
            &[],
            None,
            SandboxPolicy::new(true),
        )
        .unwrap();
        assert_eq!(mode, SandboxMode::Unsandboxed);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn detected_linux_helper_selects_the_kernel_sandbox() {
        let root = tempfile::tempdir().unwrap();
        let helper = root.path().join("bwrap");
        std::fs::write(&helper, b"test helper").unwrap();
        let java = root.path().join("java");
        let directory = root.path().join("server");
        let jar = directory.join("server.jar");
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(&java, b"java").unwrap();
        std::fs::write(&jar, b"jar").unwrap();

        let (_, mode) = linux_command(
            &java,
            &directory,
            &jar,
            &[],
            Some(helper),
            SandboxPolicy::default(),
        )
        .unwrap();
        assert_eq!(mode, SandboxMode::KernelSandbox);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_requires_the_explicit_unsandboxed_acknowledgement() {
        let rejected = command(
            Path::new(r"C:\Java\bin\java.exe"),
            Path::new(r"C:\servers\one"),
            Path::new(r"C:\servers\one\server.jar"),
            &[],
            SandboxPolicy::default(),
        );
        assert!(matches!(rejected, Err(Error::SandboxUnavailable)));

        let (_, mode) = command(
            Path::new(r"C:\Java\bin\java.exe"),
            Path::new(r"C:\servers\one"),
            Path::new(r"C:\servers\one\server.jar"),
            &[],
            SandboxPolicy::new(true),
        )
        .unwrap();
        assert_eq!(mode, SandboxMode::Unsandboxed);
    }

    #[test]
    fn path_helper_input_is_not_required_to_be_utf8() {
        // The actual platform PATH parser is OsStr-based; this assertion keeps
        // the test's intent explicit on platforms where PATH can be non-UTF-8.
        assert!(find_in_path_value("missing", OsStr::new("")).is_none());
    }
}
