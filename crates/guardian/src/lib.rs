//! Minecraft server lifecycle management.
//!
//! `guardian` sits on top of [`java-path`](java_path) (which Java, and where)
//! and [`minecraft-core`](minecraft_core) (which server build, and where), and
//! owns the part neither of them does: running the thing.
//!
//! ```no_run
//! use guardian::{Guardian, GuardianConfig, ServerConfig};
//!
//! # async fn run() -> guardian::Result<()> {
//! let mut config = ServerConfig::paper("./data/servers/lobby", "1.21.8");
//! config.eula_accepted = true;
//!
//! let guardian = Guardian::new(config, GuardianConfig::default(), "./data");
//! let mut events = guardian.subscribe();
//!
//! guardian.start().await?;
//! while let Ok(event) = events.recv().await {
//!     println!("{event:?}");
//! }
//! # Ok(()) }
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod backup;
pub mod config;
pub mod environment;
pub mod error;
pub mod events;
pub mod fs;
pub mod install;
pub mod process;
pub mod sandbox;

pub use backup::Backup;
pub use config::{GuardianConfig, Memory, ServerConfig, MAX_SERVER_MEMORY_MB};
pub use environment::{prepare, resolve_jar, resolve_java, Provision, ServerEnvironment};
pub use error::{Error, Result};
pub use events::{ConsoleLine, ServerEvent, ServerStatus, Stream};
pub use fs::ScopedFs;
pub use install::Installation;
pub use process::{Guardian, Snapshot};
