//! Turning a [`ServerConfig`] into something that can actually be executed.
//!
//! This is the only module that talks to both `java-path` and `minecraft-core`.
//! It answers one question: *given this config, what is the java binary, what
//! is the jar, and is the directory ready?*

use crate::config::ServerConfig;
use crate::error::{Error, Result};
use crate::fs::ScopedFs;
use crate::install::Installation;
use java_path::{JavaInstallation, JavaInstaller, SelectExt};
use minecraft_core::MinecraftClient;
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// A prepared, runnable server.
///
/// Every path here is absolute, and that is load-bearing rather than cosmetic:
/// the JVM is spawned with its working directory set to [`Self::directory`], so
/// a path relative to the panel's own cwd would be re-resolved against the
/// server directory and silently fail to launch.
#[derive(Debug, Clone)]
pub struct ServerEnvironment {
    /// Absolute path to the `java` launcher.
    pub java: PathBuf,
    /// Major version of the Java that was selected.
    pub java_major: u32,
    /// Absolute path to the server jar.
    pub jar: PathBuf,
    /// Working directory for the process.
    pub directory: PathBuf,
}

/// Resolve `path` against the current working directory when it is relative.
///
/// `canonicalize` is deliberately not used: it requires the path to exist and
/// resolves symlinks, neither of which is wanted for a jar that is about to be
/// downloaded or a JDK reached through a symlinked home.
pub(crate) fn absolute(path: PathBuf) -> PathBuf {
    if path.is_absolute() {
        return path;
    }
    match std::env::current_dir() {
        Ok(cwd) => cwd.join(path),
        // With no cwd there is nothing better to do than leave it alone.
        Err(_) => path,
    }
}

impl ServerEnvironment {
    /// Rewrite every relative path as an absolute one.
    pub fn absolutize(mut self) -> Self {
        self.java = absolute(self.java);
        self.jar = absolute(self.jar);
        self.directory = absolute(self.directory);
        self
    }
}

/// Where downloaded JDKs are cached, shared across every server on the host.
fn jdk_root(data_dir: &Path) -> PathBuf {
    data_dir.join("jdks")
}

/// Find a local Java matching `major`, installing a Temurin build if none exists.
///
/// Local installations are preferred: downloading a JDK is slow, and a machine
/// that already has the right major version does not need another copy.
pub async fn resolve_java(
    major: u32,
    data_dir: &Path,
    on_progress: impl Fn(String, Option<f32>) + Send + Sync + Clone + 'static,
) -> Result<JavaInstallation> {
    if let Ok(installs) = java_path::discover() {
        if let Ok(found) = installs.select().major(major).current_arch().best() {
            return Ok(found.clone());
        }
    }

    on_progress(format!("installing Java {major}"), None);

    let dir = jdk_root(data_dir);
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| Error::io(&dir, e))?;

    let progress = on_progress.clone();
    JavaInstaller::adoptium()
        .version(major)
        .install_dir(&dir)
        .cache_dir(dir.join(".cache"))
        .on_event(move |event| match event {
            java_path::InstallEvent::Resolving => progress(format!("resolving Java {major}"), None),
            java_path::InstallEvent::Downloading { downloaded, total } => {
                let fraction = total
                    .filter(|t| *t > 0)
                    .map(|t| downloaded as f32 / t as f32);
                progress(format!("downloading Java {major}"), fraction);
            }
            java_path::InstallEvent::Verifying => progress(format!("verifying Java {major}"), None),
            java_path::InstallEvent::Extracting => {
                progress(format!("extracting Java {major}"), None)
            }
            java_path::InstallEvent::Installed { .. } => {
                progress(format!("installed Java {major}"), Some(1.0))
            }
            _ => {}
        })
        .install()
        .await
        .map_err(|e| Error::JavaUnavailable(major, e.to_string()))
}

/// Download the server jar for `config`, returning it and the build it resolved to.
pub async fn resolve_jar(
    config: &ServerConfig,
    on_progress: impl Fn(String, Option<f32>) + Send + Sync + Clone + 'static,
) -> Result<(PathBuf, String)> {
    // Stream download progress back to the guardian's Progress event.
    let progress_for_download = on_progress.clone();
    let core_for_progress = config.core.clone();
    let version_for_progress = config.version.clone();
    let client = MinecraftClient::builder()
        .on_progress(std::sync::Arc::new(move |p: minecraft_core::Progress| {
            let fraction = p
                .total
                .filter(|t| *t > 0)
                .map(|t| p.downloaded as f32 / t as f32);
            progress_for_download(
                format!(
                    "downloading {} {} ({}%)",
                    core_for_progress,
                    version_for_progress,
                    fraction.map(|f| (f * 100.0).round() as u32).unwrap_or(0)
                ),
                fraction,
            );
        }))
        .build()?;

    on_progress(
        format!("resolving {} {}", config.core, config.version),
        None,
    );
    let build = match &config.build {
        Some(id) => client.build(&config.core, &config.version, id).await?,
        None => client.latest_build(&config.core, &config.version).await?,
    };

    let fs = ScopedFs::open(&config.directory).map_err(|e| Error::io(&config.directory, e))?;
    // The vendor downloader is streaming and checksum-verifying, but its
    // portable path API cannot promise no-follow semantics for an attacker
    // replacing the destination concurrently. Publish into the open
    // capability after validation instead.
    if let Ok(metadata) = fs.metadata("server.jar") {
        if metadata.is_symlink {
            fs.remove("server.jar")
                .map_err(|e| Error::io(config.directory.join("server.jar"), e))?;
        }
    }
    let temporary_name = format!(".mcpanel-server-{}.jar", Uuid::new_v4().simple());
    let temporary = config.directory.join(&temporary_name);
    on_progress(
        format!(
            "downloading {} {} build {}",
            config.core, config.version, build.build_id
        ),
        None,
    );

    // `verify` makes the download checksum-checked; `force` is off so an
    // already-correct jar is reused rather than re-fetched.
    let downloaded = client.download(&build).to(&temporary).verify(true).await;
    if let Err(error) = downloaded {
        let _ = fs.remove(&temporary_name);
        return Err(error.into());
    }
    if let Err(error) = fs.replace_file(&temporary_name, "server.jar") {
        let _ = fs.remove(&temporary_name);
        return Err(Error::io(config.directory.join("server.jar"), error));
    }

    let jar = config.directory.join("server.jar");

    Ok((jar, build.build_id.to_string()))
}

/// Create the server directory and write the files the server refuses to start without.
pub async fn scaffold(config: &ServerConfig) -> Result<()> {
    let dir = &config.directory;
    tokio::fs::create_dir_all(dir)
        .await
        .map_err(|e| Error::io(dir, e))?;
    let fs = ScopedFs::open(dir).map_err(|e| Error::io(dir, e))?;

    if config.eula_accepted {
        let body = "# Accepted through the panel.\neula=true\n";
        fs.write_atomic("eula.txt", body.as_bytes())
            .map_err(|e| Error::io(dir.join("eula.txt"), e))?;
    }

    // server.properties is seeded once and then left to the operator, except for
    // the port: the panel presents that as a setting, so it has to be the one
    // that wins. Every other line is preserved exactly as edited.
    let props = dir.join("server.properties");
    let existing = fs
        .read_file("server.properties")
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok());

    let body = match existing {
        Some(text) => set_property(&text, "server-port", &config.port.to_string()),
        None => format!(
            "server-port={}\nmotd=A Minecraft Server\nmax-players=20\nonline-mode=true\n",
            config.port
        ),
    };

    fs.write_atomic("server.properties", body.as_bytes())
        .map_err(|e| Error::io(&props, e))?;

    Ok(())
}

/// Set `key` to `value` in a `.properties` document, preserving everything else.
///
/// Comments, ordering and unrelated keys survive untouched, because the file is
/// also edited by hand through the panel's file manager.
pub(crate) fn set_property(text: &str, key: &str, value: &str) -> String {
    let mut out = String::with_capacity(text.len() + key.len() + value.len() + 2);
    let mut replaced = false;

    for line in text.lines() {
        let is_target = line
            .split_once('=')
            .is_some_and(|(name, _)| name.trim() == key)
            && !line.trim_start().starts_with('#');

        if is_target && !replaced {
            out.push_str(&format!("{key}={value}"));
            replaced = true;
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }

    if !replaced {
        out.push_str(&format!("{key}={value}\n"));
    }

    out
}

/// Whether an existing installation may be reused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provision {
    /// Reuse what is installed when it already satisfies the config.
    IfNeeded,
    /// Re-resolve and download regardless — how a deliberate update is done.
    Force,
}

/// The whole provisioning flow: directory, EULA, Java, jar.
///
/// With [`Provision::IfNeeded`] and a matching installation this touches no
/// network at all, so a restart is as fast as launching the JVM.
pub async fn prepare(
    config: &ServerConfig,
    data_dir: &Path,
    mode: Provision,
    on_progress: impl Fn(String, Option<f32>) + Send + Sync + Clone + 'static,
) -> Result<ServerEnvironment> {
    if !config.eula_accepted {
        return Err(Error::EulaNotAccepted);
    }

    // Cheap, and it keeps eula.txt and the port in step with the config.
    scaffold(config).await?;

    let installed = Installation::load(&config.directory).await;

    if mode == Provision::IfNeeded {
        if let Some(installation) = &installed {
            match installation.mismatch(config) {
                None if trusted_installation(installation, config, data_dir) => {
                    return Ok(ServerEnvironment {
                        java: installation.java.clone(),
                        java_major: installation.java_major,
                        jar: installation.jar.clone(),
                        directory: config.directory.clone(),
                    }
                    .absolutize())
                }
                None => on_progress(
                    "reinstalling: installation metadata is not trusted".into(),
                    None,
                ),
                // Say what changed, so a surprise download is never unexplained.
                Some(reason) => on_progress(format!("reinstalling: {reason}"), None),
            }
        }
    }

    let p = on_progress.clone();
    let java = resolve_java(config.java_major, data_dir, p).await?;

    let p = on_progress.clone();
    let (jar, build) = resolve_jar(config, p).await?;

    let environment = ServerEnvironment {
        java: java.java.clone(),
        java_major: java.major(),
        jar,
        directory: config.directory.clone(),
    }
    .absolutize();

    Installation {
        core: config.core.clone(),
        version: config.version.clone(),
        // Recorded so an unpinned "latest" stops moving after the first install.
        build,
        java_major: environment.java_major,
        java: environment.java.clone(),
        jar: environment.jar.clone(),
        installed_at: now_rfc3339(),
    }
    .save(&config.directory)
    .await?;

    Ok(environment)
}

/// Installation metadata lives inside a server directory an operator can
/// edit. Only reuse a jar at the exact managed destination and a Java launcher
/// that is either in the panel's downloaded JDK cache or discoverable as the
/// requested system Java. This prevents metadata from turning `start` into a
/// general executable launcher.
fn trusted_installation(
    installation: &Installation,
    config: &ServerConfig,
    data_dir: &Path,
) -> bool {
    let expected_jar = absolute(config.directory.clone()).join("server.jar");
    if absolute(installation.jar.clone()) != expected_jar {
        return false;
    }
    if !std::fs::symlink_metadata(&expected_jar)
        .map(|metadata| metadata.file_type().is_file())
        .unwrap_or(false)
    {
        return false;
    }

    let Some(java) = std::fs::canonicalize(&installation.java).ok() else {
        return false;
    };
    if !java.is_file() {
        return false;
    }

    let downloaded_root = std::fs::canonicalize(jdk_root(data_dir)).ok();
    if downloaded_root
        .as_ref()
        .is_some_and(|root| java.starts_with(root))
    {
        return true;
    }

    java_path::discover()
        .ok()
        .map(|installs| {
            installs.iter().any(|candidate| {
                candidate.major() == config.java_major
                    && std::fs::canonicalize(&candidate.java)
                        .ok()
                        .is_some_and(|path| path == java)
            })
        })
        .unwrap_or(false)
}

fn now_rfc3339() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setting_a_property_replaces_only_that_line() {
        let original = "#Minecraft server properties\nserver-port=25565\nmotd=Hello\n";
        let updated = set_property(original, "server-port", "25599");

        assert!(updated.contains("server-port=25599"));
        assert!(!updated.contains("25565"));
        // Everything the operator wrote has to survive.
        assert!(updated.contains("motd=Hello"));
        assert!(updated.contains("#Minecraft server properties"));
    }

    #[test]
    fn a_missing_property_is_appended() {
        let updated = set_property("motd=Hello\n", "server-port", "25565");

        assert!(updated.contains("motd=Hello"));
        assert!(updated.contains("server-port=25565"));
    }

    #[test]
    fn a_commented_out_property_is_not_mistaken_for_the_real_one() {
        let original = "#server-port=1234\nserver-port=25565\n";
        let updated = set_property(original, "server-port", "25599");

        assert!(
            updated.contains("#server-port=1234"),
            "the comment must survive"
        );
        assert!(updated.contains("server-port=25599"));
        assert!(!updated.contains("=25565"));
    }

    #[test]
    fn whitespace_around_a_key_is_tolerated() {
        let updated = set_property("server-port = 25565\n", "server-port", "25599");
        assert!(updated.contains("server-port=25599"));
        assert!(!updated.contains("25565"));
    }

    #[tokio::test]
    async fn scaffold_seeds_then_updates_the_port() {
        let tmp = tempfile::tempdir().unwrap();
        let mut config = ServerConfig::paper(tmp.path(), "1.21.8");
        config.eula_accepted = true;
        config.port = 25565;

        scaffold(&config).await.unwrap();
        let props = tmp.path().join("server.properties");
        assert!(std::fs::read_to_string(&props)
            .unwrap()
            .contains("server-port=25565"));

        // An operator edits the file by hand...
        std::fs::write(
            &props,
            "server-port=25565\nmotd=My Server\ndifficulty=hard\n",
        )
        .unwrap();

        // ...then changes the port in the panel.
        config.port = 25599;
        scaffold(&config).await.unwrap();

        let text = std::fs::read_to_string(&props).unwrap();
        assert!(
            text.contains("server-port=25599"),
            "the panel's port must win"
        );
        assert!(text.contains("motd=My Server"), "hand edits must survive");
        assert!(text.contains("difficulty=hard"));
    }

    #[test]
    fn absolute_paths_are_left_alone() {
        // Built from the cwd rather than written as a literal: "/srv/mc" is not
        // absolute on Windows, which has no drive letter to anchor it.
        let path = std::env::current_dir().unwrap().join("server.jar");
        assert!(path.is_absolute());

        assert_eq!(absolute(path.clone()), path);
    }

    #[test]
    fn relative_paths_are_resolved_against_the_cwd() {
        let resolved = absolute(PathBuf::from("./data/servers/abc/server.jar"));

        assert!(resolved.is_absolute());
        assert!(resolved.ends_with("data/servers/abc/server.jar"));
    }

    #[test]
    fn absolutizing_an_environment_fixes_every_path() {
        // A relative jar is the specific failure this guards: the JVM is spawned
        // with its cwd set to `directory`, so "./data/x/server.jar" would be
        // looked for at "<directory>/data/x/server.jar" and never found.
        let environment = ServerEnvironment {
            java: PathBuf::from("bin/java"),
            java_major: 21,
            jar: PathBuf::from("./data/servers/abc/server.jar"),
            directory: PathBuf::from("./data/servers/abc"),
        }
        .absolutize();

        assert!(environment.java.is_absolute());
        assert!(environment.jar.is_absolute());
        assert!(environment.directory.is_absolute());
    }
}
