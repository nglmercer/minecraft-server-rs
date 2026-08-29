//! Per-server configuration.
//!
//! Configuration is plain data: it is loaded from JSON, handed to
//! [`prepare`](crate::environment::prepare) and to [`Guardian`](crate::Guardian),
//! and never reached through a global.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Largest JVM heap the panel accepts from a single server configuration.
///
/// This is intentionally finite even on a host with a large amount of RAM:
/// configuration is untrusted input and `u32::MAX` would otherwise become a
/// denial of service at process launch.
pub const MAX_SERVER_MEMORY_MB: u32 = 1_048_576;
/// Smallest useful retained console buffer.
pub const MIN_CONSOLE_BUFFER: usize = 10;
/// Largest retained console buffer.
pub const MAX_CONSOLE_BUFFER: usize = 100_000;
/// Upper bound for automatic restart attempts.
pub const MAX_RETRIES: u32 = 100;
/// Upper bound for retry and stop delays.
pub const MAX_POLICY_DELAY_SECS: u64 = 60 * 60;
/// Upper bound for provisioning timeout.
pub const MAX_PREPARE_TIMEOUT_SECS: u64 = 24 * 60 * 60;
/// Maximum number of JVM/server arguments accepted in one configuration.
pub const MAX_ARGUMENTS: usize = 128;
/// Maximum bytes in one JVM/server argument.
pub const MAX_ARGUMENT_BYTES: usize = 16 * 1024;
/// Java feature versions above this are not meaningful configuration values.
pub const MAX_JAVA_MAJOR: u32 = 100;

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
        Memory {
            min_mb: 1024,
            max_mb: 2048,
        }
    }
}

impl Memory {
    /// The `-Xms`/`-Xmx` pair, in the order the JVM expects them.
    pub fn jvm_flags(&self) -> [String; 2] {
        [
            format!("-Xms{}M", self.min_mb),
            format!("-Xmx{}M", self.max_mb),
        ]
    }

    /// Validate heap sizing before it reaches a process launcher.
    pub fn validate(&self) -> Result<(), String> {
        if self.min_mb == 0 || self.max_mb == 0 {
            return Err("memory values must be greater than zero".into());
        }
        if self.min_mb > self.max_mb {
            return Err("memory min_mb must not exceed max_mb".into());
        }
        if self.max_mb > MAX_SERVER_MEMORY_MB {
            return Err(format!(
                "memory max_mb must not exceed {MAX_SERVER_MEMORY_MB}"
            ));
        }
        Ok(())
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
    /// The parts of the config that decide *which artifact* runs.
    ///
    /// Everything else — name, memory, port, flags, supervision policy — can
    /// change without re-downloading anything, so changing them must not cost
    /// the operator a reinstall.
    pub fn artifact_key(&self) -> (&str, &str, Option<&str>, u32) {
        (
            &self.core,
            &self.version,
            self.build.as_deref(),
            self.java_major,
        )
    }

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

    /// Validate values that can cause resource exhaustion or unsafe process
    /// argument handling.
    pub fn validate(&self) -> Result<(), String> {
        if self.core.trim().is_empty() || self.core.len() > 128 {
            return Err("server core is required and must be at most 128 bytes".into());
        }
        if self.version.trim().is_empty() || self.version.len() > 128 {
            return Err("server version is required and must be at most 128 bytes".into());
        }
        if self.core.contains(['\0', '\r', '\n']) || self.version.contains(['\0', '\r', '\n']) {
            return Err("server core and version may not contain control characters".into());
        }
        if self.java_major == 0 || self.java_major > MAX_JAVA_MAJOR {
            return Err(format!("java_major must be between 1 and {MAX_JAVA_MAJOR}"));
        }
        if let Some(build) = &self.build {
            if build.trim().is_empty() || build.len() > 128 || build.contains(['\0', '\r', '\n']) {
                return Err("build must be 1-128 bytes and contain no control characters".into());
            }
        }
        if self.directory.as_os_str().is_empty() {
            return Err("server directory is required".into());
        }
        self.memory.validate()?;
        validate_arguments(&self.jvm_args, "jvm_args")?;
        validate_arguments(&self.server_args, "server_args")?;
        if self.port == 0 {
            return Err("port must be greater than zero".into());
        }
        Ok(())
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

impl GuardianConfig {
    /// Validate supervision values before they are used to allocate memory or
    /// schedule long sleeps.
    pub fn validate(&self) -> Result<(), String> {
        if self.max_retries > MAX_RETRIES {
            return Err(format!("max_retries must not exceed {MAX_RETRIES}"));
        }
        if self.retry_delay_secs > MAX_POLICY_DELAY_SECS {
            return Err(format!(
                "retry_delay_secs must not exceed {MAX_POLICY_DELAY_SECS}"
            ));
        }
        if self.stop_timeout_secs > MAX_POLICY_DELAY_SECS {
            return Err(format!(
                "stop_timeout_secs must not exceed {MAX_POLICY_DELAY_SECS}"
            ));
        }
        if self.console_buffer < MIN_CONSOLE_BUFFER || self.console_buffer > MAX_CONSOLE_BUFFER {
            return Err(format!(
                "console_buffer must be between {MIN_CONSOLE_BUFFER} and {MAX_CONSOLE_BUFFER}"
            ));
        }
        if self.prepare_timeout_secs == 0 || self.prepare_timeout_secs > MAX_PREPARE_TIMEOUT_SECS {
            return Err(format!(
                "prepare_timeout_secs must be between 1 and {MAX_PREPARE_TIMEOUT_SECS}"
            ));
        }
        Ok(())
    }
}

fn validate_arguments(arguments: &[String], field: &str) -> Result<(), String> {
    if arguments.len() > MAX_ARGUMENTS {
        return Err(format!("{field} contains too many arguments"));
    }
    if arguments.iter().any(|argument| {
        argument.len() > MAX_ARGUMENT_BYTES || argument.contains(['\0', '\r', '\n'])
    }) {
        return Err(format!(
            "{field} contains an oversized argument or a control character"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_artifact_defining_fields_are_part_of_the_key() {
        let base = ServerConfig::paper("/srv/mc", "1.21.8");

        let mut cosmetic = base.clone();
        cosmetic.port = 25599;
        cosmetic.memory.max_mb = 8192;
        cosmetic.jvm_args = vec!["-XX:+UseG1GC".into()];
        assert_eq!(base.artifact_key(), cosmetic.artifact_key());

        let mut core = base.clone();
        core.core = "fabric".into();
        assert_ne!(base.artifact_key(), core.artifact_key());

        let mut java = base.clone();
        java.java_major = 25;
        assert_ne!(base.artifact_key(), java.artifact_key());
    }

    #[test]
    fn heap_flags_are_emitted_in_jvm_order() {
        let memory = Memory {
            min_mb: 512,
            max_mb: 4096,
        };
        assert_eq!(
            memory.jvm_flags(),
            ["-Xms512M".to_string(), "-Xmx4096M".to_string()]
        );
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
        assert!(
            !config.eula_accepted,
            "the EULA must never default to accepted"
        );
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
        assert!(
            policy.max_retries > 0,
            "an unbounded retry loop would hammer a broken server"
        );
        assert!(policy.console_buffer > 0);
        assert!(policy.validate().is_ok());
    }

    #[test]
    fn invalid_resources_are_rejected_before_launch() {
        let mut memory = Memory {
            min_mb: 2048,
            max_mb: 1024,
        };
        assert!(memory.validate().is_err());
        memory.max_mb = MAX_SERVER_MEMORY_MB + 1;
        assert!(memory.validate().is_err());

        let policy = GuardianConfig {
            console_buffer: MAX_CONSOLE_BUFFER + 1,
            ..Default::default()
        };
        assert!(policy.validate().is_err());
        let invalid_timeout = GuardianConfig {
            prepare_timeout_secs: 0,
            ..Default::default()
        };
        assert!(invalid_timeout.validate().is_err());

        let mut config = ServerConfig::paper("/srv/mc", "1.21.8");
        config.memory.min_mb = 0;
        assert!(config.validate().is_err());
        config.memory = Memory::default();
        config.jvm_args = vec!["-Xmx1G\nmalicious".into()];
        assert!(config.validate().is_err());
    }
}
