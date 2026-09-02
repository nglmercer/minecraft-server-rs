//! The fallback backend for platforms with no tray implementation yet: macOS,
//! statically linked (musl) Linux, and Linux builds without the `linux-tray`
//! feature. The panel stays fully usable, it simply has no tray icon.

use std::io;

use tokio::sync::watch;

use crate::TrayConfig;

pub(crate) struct Backend;

impl Backend {
    pub(crate) fn start(
        config: TrayConfig,
        exit_tx: watch::Sender<bool>,
        reset_tx: watch::Sender<u64>,
    ) -> io::Result<Self> {
        let _ = (config, exit_tx, reset_tx);
        Ok(Self)
    }

    pub(crate) fn shutdown(self) {}
}
