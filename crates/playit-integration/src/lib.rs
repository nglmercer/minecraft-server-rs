//! A mockable Playit integration supporting the embedded runtime and optional
//! external daemon IPC.
//!
//! The panel talks to this crate rather than depending on Playit's wire models
//! or transport details directly.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod client;
pub mod embedded;
pub mod error;
pub mod manager;
pub mod model;

pub use client::{IpcPlayitService, PlayitService};
pub use embedded::EmbeddedPlayitService;
pub use error::PlayitError;
pub use manager::PlayitManager;
pub use model::{
    ClaimInfo, PlayitAccount, PlayitAccountStatus, PlayitConnectionState, PlayitProtocol,
    PlayitStatus, PlayitTunnel, TunnelCreateInfo,
};
pub use playit_ipc::model::ServiceErrorCode;
