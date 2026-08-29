//! Best-effort OS isolation for Minecraft child processes.
//!
//! Linux uses bubblewrap when it is installed, macOS uses the system sandbox
//! profile, and Windows keeps the application-level environment/path
//! protections while documenting that an equivalent boundary is not available
//! in this binary. A deployment that cannot provide the helper should run the
//! panel under a dedicated low-privilege service account.

use std::ffi::OsString;
use std::path::Path;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::path::PathBuf;
use tokio::process::Command;

/// Build the child command, optionally placing it in a platform sandbox.
pub fn command(java: &Path, directory: &Path, jar: &Path, args: &[OsString]) -> Command {
    #[cfg(target_os = "linux")]
    {
        return linux_command(java, directory, jar, args);
    }
    #[cfg(target_os = "macos")]
    {
        return macos_command(java, directory, jar, args);
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = jar;
        let mut command = Command::new(java);
        command.current_dir(directory).args(args);
        command
    }
}

#[cfg(target_os = "linux")]
fn linux_command(java: &Path, directory: &Path, jar: &Path, args: &[OsString]) -> Command {
    let Some(bwrap) = find_in_path("bwrap") else {
        tracing::warn!(
            "bwrap is unavailable; Minecraft runs with application-level isolation only"
        );
        let mut command = Command::new(java);
        command.current_dir(directory).args(args);
        return command;
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
    command
}

#[cfg(target_os = "linux")]
fn find_in_path(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH")?
        .to_string_lossy()
        .split(':')
        .map(PathBuf::from)
        .map(|dir| dir.join(name))
        .find(|path| path.is_file())
}

#[cfg(target_os = "macos")]
fn macos_command(java: &Path, directory: &Path, jar: &Path, args: &[OsString]) -> Command {
    let Some(sandbox_exec) = find_in_path("sandbox-exec") else {
        tracing::warn!(
            "sandbox-exec is unavailable; Minecraft runs with application-level isolation only"
        );
        let mut command = Command::new(java);
        command.current_dir(directory).args(args);
        return command;
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
    command
}

#[cfg(target_os = "macos")]
fn find_in_path(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH")?
        .to_string_lossy()
        .split(':')
        .map(PathBuf::from)
        .map(|dir| dir.join(name))
        .find(|path| path.is_file())
}

#[cfg(target_os = "macos")]
fn profile_path(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
}
