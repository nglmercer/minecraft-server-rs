//! A small, mockable adapter around the Playit daemon's local IPC API.
//!
//! The panel talks to this crate rather than depending on Playit's wire models
//! directly. The daemon remains a separate process: this crate only opens a
//! short-lived IPC connection for each operation.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod client;
pub mod error;
pub mod manager;
pub mod model;

pub use client::{IpcPlayitService, PlayitService};
pub use error::PlayitError;
pub use manager::PlayitManager;
pub use model::{
    ClaimInfo, PlayitAccount, PlayitAccountStatus, PlayitConnectionState, PlayitProtocol,
    PlayitStatus, PlayitTunnel, TunnelCreateInfo,
};
