//! Turning a [`ServerConfig`] into something that can actually be executed.
//!
//! This is the only module that talks to both `java-path` and `minecraft-core`.
//! It answers one question: *given this config, what is the java binary, what
//! is the jar, and is the directory ready?*

use crate::config::ServerConfig;
use crate::error::{Error, Result};
use java_path::{JavaInstallation, JavaInstaller, SelectExt};
use minecraft_core::MinecraftClient;
use std::path::{Path, PathBuf};

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
    mut on_progress: impl FnMut(String, Option<f32>) + Send + 'static,
) -> Result<JavaInstallation> {
    if let Ok(installs) = java_path::discover() {
        if let Ok(found) = installs.select().major(major).current_arch().best() {
            return Ok(found.clone());
        }
    }

    on_progress(format!("installing Java {major}"), None);

    let dir = jdk_root(data_dir);
    tokio::fs::create_dir_all(&dir).await.map_err(|e| Error::io(&dir, e))?;

    JavaInstaller::adoptium()
        .version(major)
        .install_dir(&dir)
        .cache_dir(dir.join(".cache"))
        .install()
        .await
        .map_err(|e| Error::JavaUnavailable(major, e.to_string()))
}

/// Download the server jar for `config`, reusing it when it is already present.
pub async fn resolve_jar(
    config: &ServerConfig,
    mut on_progress: impl FnMut(String, Option<f32>) + Send + 'static,
) -> Result<PathBuf> {
    let client = MinecraftClient::builder().build()?;

    let build = match &config.build {
        Some(id) => client.build(&config.core, &config.version, id).await?,
        None => client.latest_build(&config.core, &config.version).await?,
    };

    let jar = config.directory.join("server.jar");
    on_progress(format!("downloading {} {}", config.core, config.version), None);

    // `verify` makes the download checksum-checked; `force` is off so an
    // already-correct jar is reused rather than re-fetched.
    client.download(&build).to(&jar).verify(true).await?;

    Ok(jar)
}

/// Create the server directory and write the files the server refuses to start without.
pub async fn scaffold(config: &ServerConfig) -> Result<()> {
    let dir = &config.directory;
    tokio::fs::create_dir_all(dir).await.map_err(|e| Error::io(dir, e))?;

    if config.eula_accepted {
        let eula = dir.join("eula.txt");
        let body = "# Accepted through the panel.\neula=true\n";
        tokio::fs::write(&eula, body).await.map_err(|e| Error::io(&eula, e))?;
    }

    // server.properties is seeded once and then left to the operator, except for
    // the port: the panel presents that as a setting, so it has to be the one
    // that wins. Every other line is preserved exactly as edited.
    let props = dir.join("server.properties");
    let existing = tokio::fs::read_to_string(&props).await.ok();

    let body = match existing {
        Some(text) => set_property(&text, "server-port", &config.port.to_string()),
        None => format!(
            "server-port={}\nmotd=A Minecraft Server\nmax-players=20\nonline-mode=true\n",
            config.port
        ),
    };

    tokio::fs::write(&props, body).await.map_err(|e| Error::io(&props, e))?;

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

/// The whole provisioning flow: directory, EULA, Java, jar.
pub async fn prepare(
    config: &ServerConfig,
    data_dir: &Path,
    on_progress: impl Fn(String, Option<f32>) + Send + Sync + Clone + 'static,
) -> Result<ServerEnvironment> {
    if !config.eula_accepted {
        return Err(Error::EulaNotAccepted);
    }

    scaffold(config).await?;

    let p = on_progress.clone();
    let java = resolve_java(config.java_major, data_dir, p).await?;

    let p = on_progress.clone();
    let jar = resolve_jar(config, p).await?;

    Ok(ServerEnvironment {
        java: java.java.clone(),
        java_major: java.major(),
        jar,
        directory: config.directory.clone(),
    }
    .absolutize())
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

        assert!(updated.contains("#server-port=1234"), "the comment must survive");
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
        assert!(std::fs::read_to_string(&props).unwrap().contains("server-port=25565"));

        // An operator edits the file by hand...
        std::fs::write(&props, "server-port=25565\nmotd=My Server\ndifficulty=hard\n").unwrap();

        // ...then changes the port in the panel.
        config.port = 25599;
        scaffold(&config).await.unwrap();

        let text = std::fs::read_to_string(&props).unwrap();
        assert!(text.contains("server-port=25599"), "the panel's port must win");
        assert!(text.contains("motd=My Server"), "hand edits must survive");
        assert!(text.contains("difficulty=hard"));
    }

    #[test]
    fn absolute_paths_are_left_alone() {
        let path = PathBuf::from("/srv/mc/server.jar");
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
