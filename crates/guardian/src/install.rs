//! A record of what was actually provisioned into a server directory.
//!
//! Without this, every start has to ask the internet what to run: which build
//! is newest, and does the jar on disk match it. That makes a restart depend on
//! a network round-trip, and — worse — makes "latest" a moving target, so a
//! server can quietly change build underneath its operator.
//!
//! The record turns the question into a local one: *is what I need already
//! here?* Downloading only happens when the answer is no.

use crate::config::ServerConfig;
use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Written into the server directory alongside the jar.
const FILENAME: &str = ".mcpanel-install.json";

/// What is installed in a server directory right now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Installation {
    /// Provider the jar came from.
    pub core: String,
    /// Minecraft version.
    pub version: String,
    /// The build that was actually resolved and downloaded, never "latest".
    pub build: String,
    /// Major Java version it was provisioned for.
    pub java_major: u32,
    /// The `java` launcher that was selected.
    pub java: PathBuf,
    /// The downloaded server jar.
    pub jar: PathBuf,
    /// RFC 3339 timestamp of the install.
    pub installed_at: String,
}

impl Installation {
    /// Whether this installation can run `config` with no further downloading.
    ///
    /// A config with no pinned build accepts whatever is installed. That is the
    /// point: "latest" selects a build at install time and then stops moving,
    /// so restarting a server never silently upgrades it. Use
    /// [`crate::Guardian::reinstall`] to take a newer build deliberately.
    pub fn satisfies(&self, config: &ServerConfig) -> bool {
        self.mismatch(config).is_none()
    }

    /// Why this installation cannot run `config`, phrased for the console.
    pub fn mismatch(&self, config: &ServerConfig) -> Option<String> {
        if self.core != config.core {
            return Some(format!("server type changed from {} to {}", self.core, config.core));
        }
        if self.version != config.version {
            return Some(format!("version changed from {} to {}", self.version, config.version));
        }
        if self.java_major != config.java_major {
            return Some(format!(
                "Java version changed from {} to {}",
                self.java_major, config.java_major
            ));
        }
        if let Some(pinned) = &config.build {
            if &self.build != pinned {
                return Some(format!("build changed from {} to {pinned}", self.build));
            }
        }
        if !self.jar.is_file() {
            return Some(format!("{} is missing", self.jar.display()));
        }
        if !self.java.is_file() {
            return Some(format!("{} is missing", self.java.display()));
        }
        None
    }

    /// Read the record from a server directory, if one is there and readable.
    pub async fn load(server_dir: &Path) -> Option<Self> {
        let bytes = tokio::fs::read(server_dir.join(FILENAME)).await.ok()?;
        // A corrupt record is treated as no record: reinstalling is recoverable,
        // refusing to start is not.
        serde_json::from_slice(&bytes).ok()
    }

    /// Write the record into a server directory.
    pub async fn save(&self, server_dir: &Path) -> Result<()> {
        let path = server_dir.join(FILENAME);
        let body = serde_json::to_vec_pretty(self)?;
        tokio::fs::write(&path, body).await.map_err(|e| Error::io(&path, e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn installed(dir: &Path) -> Installation {
        std::fs::write(dir.join("server.jar"), b"jar").unwrap();
        std::fs::write(dir.join("java"), b"java").unwrap();

        Installation {
            core: "paper".into(),
            version: "1.21.8".into(),
            build: "112".into(),
            java_major: 21,
            java: dir.join("java"),
            jar: dir.join("server.jar"),
            installed_at: "2026-08-19T00:00:00Z".into(),
        }
    }

    fn config_for(dir: &Path) -> ServerConfig {
        let mut config = ServerConfig::paper(dir, "1.21.8");
        config.java_major = 21;
        config
    }

    #[test]
    fn an_unchanged_config_needs_no_reinstall() {
        let tmp = tempfile::tempdir().unwrap();
        let installation = installed(tmp.path());

        assert!(installation.satisfies(&config_for(tmp.path())));
    }

    #[test]
    fn an_unpinned_build_stays_where_it_was_installed() {
        let tmp = tempfile::tempdir().unwrap();
        let installation = installed(tmp.path());

        let mut config = config_for(tmp.path());
        config.build = None;

        // Re-resolving "latest" on every start would silently move the server
        // onto a new build, which is exactly what must not happen.
        assert!(installation.satisfies(&config));
    }

    #[test]
    fn changing_the_server_type_forces_a_reinstall() {
        let tmp = tempfile::tempdir().unwrap();
        let installation = installed(tmp.path());

        let mut config = config_for(tmp.path());
        config.core = "fabric".into();

        assert!(!installation.satisfies(&config));
        assert!(installation.mismatch(&config).unwrap().contains("fabric"));
    }

    #[test]
    fn changing_version_java_or_a_pinned_build_forces_a_reinstall() {
        let tmp = tempfile::tempdir().unwrap();
        let installation = installed(tmp.path());

        let mut version = config_for(tmp.path());
        version.version = "1.21.9".into();
        assert!(!installation.satisfies(&version));

        let mut java = config_for(tmp.path());
        java.java_major = 25;
        assert!(!installation.satisfies(&java));

        let mut build = config_for(tmp.path());
        build.build = Some("999".into());
        assert!(!installation.satisfies(&build));
    }

    #[test]
    fn cosmetic_changes_do_not_force_a_reinstall() {
        let tmp = tempfile::tempdir().unwrap();
        let installation = installed(tmp.path());

        let mut config = config_for(tmp.path());
        config.port = 25599;
        config.memory.max_mb = 8192;
        config.jvm_args = vec!["-XX:+UseG1GC".into()];
        config.server_args = vec!["nogui".into(), "--forceUpgrade".into()];

        // None of these change which artifact runs, so none should cost a download.
        assert!(installation.satisfies(&config));
    }

    #[test]
    fn a_deleted_jar_forces_a_reinstall() {
        let tmp = tempfile::tempdir().unwrap();
        let installation = installed(tmp.path());
        std::fs::remove_file(tmp.path().join("server.jar")).unwrap();

        assert!(!installation.satisfies(&config_for(tmp.path())));
        assert!(installation.mismatch(&config_for(tmp.path())).unwrap().contains("missing"));
    }

    #[tokio::test]
    async fn records_round_trip_through_the_server_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let installation = installed(tmp.path());

        installation.save(tmp.path()).await.unwrap();
        assert_eq!(Installation::load(tmp.path()).await.unwrap(), installation);
    }

    #[tokio::test]
    async fn a_missing_or_corrupt_record_reads_as_none() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(Installation::load(tmp.path()).await.is_none());

        std::fs::write(tmp.path().join(FILENAME), b"{ not json").unwrap();
        assert!(Installation::load(tmp.path()).await.is_none());
    }
}
