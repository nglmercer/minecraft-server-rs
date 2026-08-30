//! The Windows tray backend: a winit event loop owning a `tray-icon` tray.

use std::io;
use std::process::Command;
use std::sync::mpsc::SyncSender;

use tokio::sync::watch;
use tray_icon::menu::MenuEvent;
use tray_icon::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use winit::application::ApplicationHandler;
use winit::event::{StartCause, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::platform::windows::EventLoopBuilderExtWindows;

#[path = "icon.rs"]
mod icon;
#[path = "menu.rs"]
mod menu;

use crate::TrayConfig;

/// A running Windows tray, owned by [`crate::TrayHandle`].
pub(crate) struct Backend {
    proxy: Option<EventLoopProxy<UserEvent>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Backend {
    pub(crate) fn start(config: TrayConfig, exit_tx: watch::Sender<bool>) -> io::Result<Self> {
        let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);
        let thread = std::thread::Builder::new()
            .name("mcpanel-tray".into())
            .spawn(move || run(config, exit_tx, ready_tx))?;

        match ready_rx.recv() {
            Ok(Ok(proxy)) => Ok(Self {
                proxy: Some(proxy),
                thread: Some(thread),
            }),
            Ok(Err(error)) => {
                let _ = thread.join();
                Err(io::Error::other(error))
            }
            Err(error) => {
                let _ = thread.join();
                Err(io::Error::other(format!(
                    "tray event loop exited before initialization: {error}"
                )))
            }
        }
    }

    pub(crate) fn shutdown(mut self) {
        if let Some(proxy) = self.proxy.take() {
            let _ = proxy.send_event(UserEvent::Shutdown);
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

enum UserEvent {
    TrayIcon(TrayIconEvent),
    Menu(MenuEvent),
    Shutdown,
}

fn run(
    config: TrayConfig,
    exit_tx: watch::Sender<bool>,
    ready_tx: SyncSender<Result<EventLoopProxy<UserEvent>, String>>,
) {
    let mut event_loop_builder = EventLoop::<UserEvent>::with_user_event();
    event_loop_builder.with_any_thread(true);
    let event_loop = match event_loop_builder.build() {
        Ok(event_loop) => event_loop,
        Err(error) => {
            let _ = ready_tx.send(Err(format!(
                "could not initialize the tray event loop: {error}"
            )));
            return;
        }
    };
    let proxy = event_loop.create_proxy();

    let tray_menu = match menu::build() {
        Ok(menu) => menu,
        Err(error) => {
            let _ = ready_tx.send(Err(error.clone()));
            tracing::error!(error = %error, "could not create the MCP Panel tray menu");
            return;
        }
    };

    let tray_proxy = proxy.clone();
    TrayIconEvent::set_event_handler(Some(move |event| {
        let _ = tray_proxy.send_event(UserEvent::TrayIcon(event));
    }));
    let menu_proxy = proxy.clone();
    MenuEvent::set_event_handler(Some(move |event| {
        let _ = menu_proxy.send_event(UserEvent::Menu(event));
    }));

    let open_panel_id = tray_menu.open_panel.id().clone();
    let exit_id = tray_menu.exit.id().clone();
    let mut application = TrayApplication {
        panel_url: config.panel_url,
        exit_tx,
        tray_menu: Some(tray_menu),
        tray_icon: None,
        open_panel_id,
        exit_id,
        proxy,
        ready_tx: Some(ready_tx),
    };

    let _ = event_loop.run_app(&mut application);

    TrayIconEvent::set_event_handler(None::<fn(TrayIconEvent)>);
    MenuEvent::set_event_handler(None::<fn(MenuEvent)>);
}

struct TrayApplication {
    panel_url: String,
    exit_tx: watch::Sender<bool>,
    tray_menu: Option<menu::TrayMenu>,
    tray_icon: Option<TrayIcon>,
    open_panel_id: tray_icon::menu::MenuId,
    exit_id: tray_icon::menu::MenuId,
    proxy: EventLoopProxy<UserEvent>,
    ready_tx: Option<SyncSender<Result<EventLoopProxy<UserEvent>, String>>>,
}

impl TrayApplication {
    fn create_icon(&mut self, event_loop: &ActiveEventLoop) {
        let Some(tray_menu) = self.tray_menu.take() else {
            return;
        };

        match icon::load().and_then(|icon| {
            TrayIconBuilder::new()
                .with_menu(Box::new(tray_menu.menu))
                .with_menu_on_left_click(false)
                .with_tooltip("MCP Panel")
                .with_icon(icon)
                .build()
                .map_err(|_| icon::IconError::backend_rejected())
        }) {
            Ok(icon) => {
                self.tray_icon = Some(icon);
                if let Some(ready_tx) = self.ready_tx.take() {
                    if ready_tx.send(Ok(self.proxy.clone())).is_err() {
                        event_loop.exit();
                    }
                }
            }
            Err(error) => {
                if let Some(ready_tx) = self.ready_tx.take() {
                    let _ = ready_tx.send(Err(error.to_string()));
                }
                tracing::error!(error = %error, "could not create the MCP Panel tray icon");
                event_loop.exit();
            }
        }
    }

    fn open_panel(&self) {
        // The URL is generated by the panel from its bound port. Use a direct
        // executable invocation rather than a shell so this action cannot turn a
        // URL into shell syntax.
        if let Err(error) = Command::new("rundll32.exe")
            .args(["url.dll,FileProtocolHandler", &self.panel_url])
            .spawn()
        {
            tracing::warn!(error = %error, "could not open the MCP Panel in a browser");
        }
    }
}

impl ApplicationHandler<UserEvent> for TrayApplication {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        event_loop.set_control_flow(ControlFlow::Wait);
        if self.tray_icon.is_none() {
            self.create_icon(event_loop);
        }
    }

    fn window_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _window_id: winit::window::WindowId,
        _event: WindowEvent,
    ) {
    }

    fn new_events(&mut self, event_loop: &ActiveEventLoop, cause: StartCause) {
        if matches!(cause, StartCause::Init) && self.tray_icon.is_none() {
            self.create_icon(event_loop);
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::TrayIcon(TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            }) => self.open_panel(),
            UserEvent::Menu(event) if event.id() == &self.open_panel_id => self.open_panel(),
            UserEvent::Menu(event) if event.id() == &self.exit_id => {
                let _ = self.exit_tx.send(true);
                event_loop.exit();
            }
            UserEvent::Shutdown => event_loop.exit(),
            _ => {}
        }
    }
}
