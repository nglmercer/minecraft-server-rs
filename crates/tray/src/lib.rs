//! The MCP Panel desktop tray integration.
//!
//! The tray is deliberately kept outside of `guardian`: it only observes the
//! panel lifecycle and sends a shutdown request. Server operations will be
//! connected to the panel API in a later milestone.
//!
//! Each platform gets its own backend, because the two supported ones do not
//! share an event loop: Windows runs winit, Linux runs GTK. Everything else
//! falls back to a no-op backend.

#![forbid(unsafe_code)]

#[cfg(windows)]
#[path = "windows.rs"]
mod backend;

#[cfg(all(target_os = "linux", not(target_env = "musl"), feature = "linux-tray"))]
#[path = "linux.rs"]
mod backend;

#[cfg(not(any(
    windows,
    all(target_os = "linux", not(target_env = "musl"), feature = "linux-tray")
)))]
#[path = "unsupported.rs"]
mod backend;

use std::io;

use tokio::sync::watch;

/// Configuration used to create the MCP Panel tray icon.
#[derive(Clone, Debug)]
pub struct TrayConfig {
    /// The URL opened by the tray's Open Panel action.
    pub panel_url: String,
}

impl TrayConfig {
    /// Create tray configuration for a panel URL.
    pub fn new(panel_url: impl Into<String>) -> Self {
        Self {
            panel_url: panel_url.into(),
        }
    }
}

/// A running tray integration.
pub struct TrayHandle {
    exit_tx: watch::Sender<bool>,
    exit_rx: watch::Receiver<bool>,
    backend: backend::Backend,
}

/// Start the tray integration.
///
/// Windows and Linux desktop builds own a native event loop. Every other
/// target, and any session that has no tray to attach to, gets a no-op handle
/// so the panel remains usable; the error explains which it was.
pub fn start(config: TrayConfig) -> io::Result<TrayHandle> {
    if std::env::var_os("MCPANEL_NO_TRAY").is_some() {
        return Err(io::Error::other("the tray is disabled by MCPANEL_NO_TRAY"));
    }

    let (exit_tx, exit_rx) = watch::channel(false);
    let backend = backend::Backend::start(config, exit_tx.clone())?;

    Ok(TrayHandle {
        exit_tx,
        exit_rx,
        backend,
    })
}

impl TrayHandle {
    /// Get a receiver that becomes `true` when the user chooses Exit.
    pub fn exit_signal(&self) -> watch::Receiver<bool> {
        self.exit_rx.clone()
    }

    /// Stop the native tray event loop, if this platform has one.
    pub fn shutdown(self) {
        let _ = self.exit_tx.send(true);
        self.backend.shutdown();
    }
}
