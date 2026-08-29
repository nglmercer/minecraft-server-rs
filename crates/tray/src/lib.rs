//! The MCP Panel desktop tray integration.
//!
//! The tray is deliberately kept outside of `guardian`: it only observes the
//! panel lifecycle and sends a shutdown request. Server operations will be
//! connected to the panel API in a later milestone.

#![forbid(unsafe_code)]

#[cfg(windows)]
mod events;
#[cfg(windows)]
mod icon;
#[cfg(windows)]
mod menu;

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
    #[cfg(windows)]
    proxy: Option<winit::event_loop::EventLoopProxy<events::UserEvent>>,
    #[cfg(windows)]
    thread: Option<std::thread::JoinHandle<()>>,
}

/// Start the tray integration.
///
/// Windows owns the native event loop for the first desktop milestone. Other
/// platforms receive a no-op handle so the panel remains usable while their
/// native tray implementation is added in a later milestone.
pub fn start(config: TrayConfig) -> io::Result<TrayHandle> {
    let (exit_tx, exit_rx) = watch::channel(false);

    #[cfg(windows)]
    {
        let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);
        let thread_exit_tx = exit_tx.clone();
        let thread = std::thread::Builder::new()
            .name("mcpanel-tray".into())
            .spawn(move || events::run(config, thread_exit_tx, ready_tx))?;

        let proxy = match ready_rx.recv() {
            Ok(Ok(proxy)) => proxy,
            Ok(Err(error)) => {
                let _ = thread.join();
                return Err(io::Error::other(error));
            }
            Err(error) => {
                let _ = thread.join();
                return Err(io::Error::other(format!(
                    "tray event loop exited before initialization: {error}"
                )));
            }
        };

        Ok(TrayHandle {
            exit_tx,
            exit_rx,
            proxy: Some(proxy),
            thread: Some(thread),
        })
    }

    #[cfg(not(windows))]
    {
        let _ = config;
        Ok(TrayHandle { exit_tx, exit_rx })
    }
}

impl TrayHandle {
    /// Get a receiver that becomes `true` when the user chooses Exit.
    pub fn exit_signal(&self) -> watch::Receiver<bool> {
        self.exit_rx.clone()
    }

    /// Stop the native tray event loop, if this platform has one.
    pub fn shutdown(self) {
        let _ = self.exit_tx.send(true);

        #[cfg(windows)]
        {
            let mut handle = self;
            if let Some(proxy) = handle.proxy.take() {
                let _ = proxy.send_event(events::UserEvent::Shutdown);
            }
            if let Some(thread) = handle.thread.take() {
                let _ = thread.join();
            }
        }
    }
}
