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

    // server.properties is only seeded, never rewritten: the operator may have
    // edited it through the file manager, and clobbering that would be rude.
    let props = dir.join("server.properties");
    if !props.exists() {
        let body = format!(
            "server-port={}\nmotd=A Minecraft Server\nmax-players=20\nonline-mode=true\n",
            config.port
        );
        tokio::fs::write(&props, body).await.map_err(|e| Error::io(&props, e))?;
    }

    Ok(())
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
    })
}
