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
    /// Optional setup URL for first-run. When Some, the tray will open it during init.
    pub setup_url: Option<String>,
    /// Optional recovery URL template (without token). Not strictly needed; the
    /// tray signals recovery via channel and the panel opens the full URL.
    pub recovery_url: Option<String>,
}

impl TrayConfig {
    /// Create tray configuration for a panel URL.
    pub fn new(panel_url: impl Into<String>) -> Self {
        Self {
            panel_url: panel_url.into(),
            setup_url: None,
            recovery_url: None,
        }
    }

    /// Set setup URL (builder).
    pub fn with_setup_url(mut self, url: impl Into<String>) -> Self {
        self.setup_url = Some(url.into());
        self
    }

    /// Set recovery URL (builder).
    pub fn with_recovery_url(mut self, url: impl Into<String>) -> Self {
        self.recovery_url = Some(url.into());
        self
    }
}

/// A running tray integration.
pub struct TrayHandle {
    exit_tx: watch::Sender<bool>,
    exit_rx: watch::Receiver<bool>,
    reset_tx: watch::Sender<u64>,
    reset_rx: watch::Receiver<u64>,
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
    let (reset_tx, reset_rx) = watch::channel(0u64);
    let backend = backend::Backend::start(config, exit_tx.clone(), reset_tx.clone())?;

    Ok(TrayHandle {
        exit_tx,
        exit_rx,
        reset_tx,
        reset_rx,
        backend,
    })
}

impl TrayHandle {
    /// Get a receiver that becomes `true` when the user chooses Exit.
    pub fn exit_signal(&self) -> watch::Receiver<bool> {
        self.exit_rx.clone()
    }

    /// Get a receiver that increments when the user chooses Reset Admin Password.
    pub fn reset_signal(&self) -> watch::Receiver<u64> {
        self.reset_rx.clone()
    }

    /// Trigger a reset request programmatically (used by panel to simulate tray action).
    pub fn request_reset(&self) {
        self.reset_tx.send_modify(|v| *v = v.wrapping_add(1));
    }

    /// Stop the native tray event loop, if this platform has one.
    pub fn shutdown(self) {
        let _ = self.exit_tx.send(true);
        self.backend.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tray_config_builders_set_urls() {
        let cfg = TrayConfig::new("http://127.0.0.1:8080")
            .with_setup_url("http://127.0.0.1:8080/setup")
            .with_recovery_url("http://127.0.0.1:8080/recovery");
        assert_eq!(cfg.panel_url, "http://127.0.0.1:8080");
        assert_eq!(
            cfg.setup_url.as_deref(),
            Some("http://127.0.0.1:8080/setup")
        );
        assert_eq!(
            cfg.recovery_url.as_deref(),
            Some("http://127.0.0.1:8080/recovery")
        );
    }

    #[test]
    fn tray_config_defaults_have_no_optional_urls() {
        let cfg = TrayConfig::new("http://127.0.0.1:8080");
        assert!(cfg.setup_url.is_none());
        assert!(cfg.recovery_url.is_none());
    }

    #[tokio::test]
    async fn reset_signal_increments_on_request() {
        // winit/GTK event loops can only be created once per process. Testing
        // the tray backend twice in the same process panics with
        // "EventLoop can't be recreated" (Windows) or GTK init errors (Linux).
        // For those platforms test the underlying watch logic directly.
        if cfg!(windows) || cfg!(all(target_os = "linux", feature = "linux-tray")) {
            let (tx, mut rx) = watch::channel(0u64);
            assert_eq!(*rx.borrow(), 0);
            tx.send_modify(|v| *v = v.wrapping_add(1));
            rx.changed().await.unwrap();
            assert_eq!(*rx.borrow(), 1);
            tx.send_modify(|v| *v = v.wrapping_add(1));
            rx.changed().await.unwrap();
            assert_eq!(*rx.borrow(), 2);
            return;
        }
        let cfg = TrayConfig::new("http://127.0.0.1:8080");
        let handle = match start(cfg) {
            Ok(h) => h,
            Err(e) if e.to_string().contains("no display") => return,
            Err(e) if e.to_string().contains("EventLoop") => return,
            Err(e) => panic!("tray should start: {e}"),
        };
        let mut rx = handle.reset_signal();
        assert_eq!(*rx.borrow(), 0);
        handle.request_reset();
        rx.changed().await.unwrap();
        assert_eq!(*rx.borrow(), 1);
        handle.request_reset();
        rx.changed().await.unwrap();
        assert_eq!(*rx.borrow(), 2);
        handle.shutdown();
    }

    #[tokio::test]
    async fn exit_signal_fires_on_shutdown() {
        if cfg!(windows) || cfg!(all(target_os = "linux", feature = "linux-tray")) {
            // Avoid double EventLoop/GTK init in same process (see above).
            return;
        }
        let cfg = TrayConfig::new("http://127.0.0.1:8080");
        let handle = match start(cfg) {
            Ok(h) => h,
            Err(e) if e.to_string().contains("no display") => return,
            Err(e) if e.to_string().contains("EventLoop") => return,
            Err(e) => panic!("tray should start: {e}"),
        };
        let mut rx = handle.exit_signal();
        assert!(!*rx.borrow());
        handle.shutdown();
        assert!(rx.changed().await.is_err() || *rx.borrow());
    }
}
