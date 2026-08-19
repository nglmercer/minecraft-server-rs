//! Per-server configuration.
//!
//! Configuration is plain data: it is loaded from JSON, handed to
//! [`prepare`](crate::environment::prepare) and to [`Guardian`](crate::Guardian),
//! and never reached through a global.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// How much heap the JVM gets.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Memory {
    /// `-Xms`, in mebibytes.
    pub min_mb: u32,
    /// `-Xmx`, in mebibytes.
    pub max_mb: u32,
}

impl Default for Memory {
    fn default() -> Self {
        Memory { min_mb: 1024, max_mb: 2048 }
    }
}

impl Memory {
    /// The `-Xms`/`-Xmx` pair, in the order the JVM expects them.
    pub fn jvm_flags(&self) -> [String; 2] {
        [format!("-Xms{}M", self.min_mb), format!("-Xmx{}M", self.max_mb)]
    }
}

/// Which server to run, and how.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    /// Provider id understood by `minecraft-core` (`paper`, `vanilla`, `fabric`, ...).
    pub core: String,
    /// Minecraft version, e.g. `1.21.8`.
    pub version: String,
    /// Specific build id; `None` means "latest build for this version".
    #[serde(default)]
    pub build: Option<String>,
    /// Major Java version to run under.
    pub java_major: u32,
    /// Heap sizing.
    #[serde(default)]
    pub memory: Memory,
    /// Extra JVM flags, inserted after the heap flags and before `-jar`.
    #[serde(default)]
    pub jvm_args: Vec<String>,
    /// Arguments passed to the server jar itself, after `-jar <jar>`.
    #[serde(default = "default_server_args")]
    pub server_args: Vec<String>,
    /// Working directory the server runs in. World data lives here.
    pub directory: PathBuf,
    /// Port advertised in `server.properties`.
    #[serde(default = "default_port")]
    pub port: u16,
    /// Whether the operator has accepted the Mojang EULA.
    #[serde(default)]
    pub eula_accepted: bool,
}

fn default_server_args() -> Vec<String> {
    vec!["nogui".into()]
}

fn default_port() -> u16 {
    25565
}

impl ServerConfig {
    /// A sensible Paper server rooted at `directory`.
    pub fn paper(directory: impl Into<PathBuf>, version: impl Into<String>) -> Self {
        ServerConfig {
            core: "paper".into(),
            version: version.into(),
            build: None,
            java_major: 21,
            memory: Memory::default(),
            jvm_args: Vec::new(),
            server_args: default_server_args(),
            directory: directory.into(),
            port: default_port(),
            eula_accepted: false,
        }
    }
}

/// Supervision policy: what happens when the process goes away.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuardianConfig {
    /// Restart automatically after a crash.
    pub auto_restart: bool,
    /// Give up after this many consecutive failed restarts.
    pub max_retries: u32,
    /// Wait this long between a crash and the restart attempt.
    pub retry_delay_secs: u64,
    /// How long a graceful `stop` may take before the process is killed.
    pub stop_timeout_secs: u64,
    /// How many console lines to retain for late-joining clients.
    pub console_buffer: usize,
    /// Give up on downloading Java and the server jar after this long.
    #[serde(default = "default_prepare_timeout")]
    pub prepare_timeout_secs: u64,
}

/// Generous enough for a JDK and a server jar on a slow line, finite enough
/// that a stalled download does not pin the server in `Preparing` forever.
fn default_prepare_timeout() -> u64 {
    900
}

impl Default for GuardianConfig {
    fn default() -> Self {
        GuardianConfig {
            auto_restart: true,
            max_retries: 3,
            retry_delay_secs: 5,
            stop_timeout_secs: 60,
            console_buffer: 500,
            prepare_timeout_secs: default_prepare_timeout(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heap_flags_are_emitted_in_jvm_order() {
        let memory = Memory { min_mb: 512, max_mb: 4096 };
        assert_eq!(memory.jvm_flags(), ["-Xms512M".to_string(), "-Xmx4096M".to_string()]);
    }

    #[test]
    fn a_minimal_config_document_fills_in_the_optional_fields() {
        // The panel writes full documents, but a hand-edited panel.json should
        // not have to spell out every field.
        let json = r#"{
            "core": "paper",
            "version": "1.21.8",
            "java_major": 21,
            "directory": "/srv/mc"
        }"#;

        let config: ServerConfig = serde_json::from_str(json).unwrap();

        assert_eq!(config.port, 25565);
        assert_eq!(config.server_args, vec!["nogui".to_string()]);
        assert_eq!(config.memory.max_mb, 2048);
        assert!(config.jvm_args.is_empty());
        assert!(config.build.is_none());
        assert!(!config.eula_accepted, "the EULA must never default to accepted");
    }

    #[test]
    fn configs_survive_a_serde_round_trip() {
        let original = ServerConfig::paper("/srv/mc", "1.21.8");
        let text = serde_json::to_string(&original).unwrap();
        let parsed: ServerConfig = serde_json::from_str(&text).unwrap();

        assert_eq!(parsed.core, original.core);
        assert_eq!(parsed.version, original.version);
        assert_eq!(parsed.directory, original.directory);
        assert_eq!(parsed.port, original.port);
    }

    #[test]
    fn the_default_policy_restarts_a_bounded_number_of_times() {
        let policy = GuardianConfig::default();
        assert!(policy.auto_restart);
        assert!(policy.max_retries > 0, "an unbounded retry loop would hammer a broken server");
        assert!(policy.console_buffer > 0);
    }
}
