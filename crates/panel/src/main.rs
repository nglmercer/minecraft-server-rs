//! A single-binary Minecraft server control panel.
//!
//! ```text
//! mcpanel --data-dir ./data --bind 127.0.0.1:8080
//! ```
//!
//! On first run an `admin` account is created and its generated password is
//! printed once. Everything else — servers, users, sessions — is managed
//! through the web UI or the REST API under `/api`.

#![forbid(unsafe_code)]
#![allow(clippy::all, clippy::pedantic, clippy::nursery, clippy::restriction)]
// Packaged Windows builds are GUI executables so launching mcpanel.exe does
// not open a console window. Debug builds keep the console for `cargo run`
// logs; `--features console` opts a release build back in.
#![cfg_attr(
    all(windows, not(debug_assertions), not(feature = "console")),
    windows_subsystem = "windows"
)]

mod api;
mod auth;
mod backups;
mod bind;
mod error;
mod filesystem;
mod limits;
mod metrics;
mod recovery;
mod state;
mod store;
mod tickets;
mod web;

use anyhow::{Context, Result};
use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{header, HeaderMap, HeaderValue, Request};
use axum::middleware::Next;
use axum::response::Response;
use axum::Router;
use clap::Parser;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::watch;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

use crate::bind::{
    bind_with_fallback, panel_url, remove_instance_lock, resolve_effective_bind,
    write_instance_lock,
};
use crate::limits::ResourceLimits;
use crate::state::AppState;
use crate::state::PlayitMode;
use tray::{TrayConfig, TrayHandle};

/// Command line options.
#[derive(Parser, Debug)]
#[command(
    name = "mcpanel",
    version,
    about = "A fast, simple Minecraft server panel"
)]
struct Args {
    /// Where Playit state, servers, JDKs and panel state are stored.
    #[arg(long, default_value = "./data", env = "MCPANEL_DATA")]
    data_dir: PathBuf,

    /// Address to listen on. Default is 127.0.0.1:8080 when neither the flag nor an
    /// optional port file is present. A file at `<data-dir>/port` (or `<data-dir>/bind`)
    /// may override the default without needing a flag.
    #[arg(long, env = "MCPANEL_BIND", value_name = "ADDR")]
    bind: Option<SocketAddr>,

    /// Allow browser requests from any origin. Needed only for `npm run dev`.
    #[arg(long, env = "MCPANEL_DEV_CORS")]
    dev_cors: bool,

    /// Permit plaintext HTTP on a non-loopback bind. Prefer a TLS reverse
    /// proxy; this flag explicitly acknowledges the risk.
    #[arg(long, env = "MCPANEL_ALLOW_INSECURE_HTTP")]
    allow_insecure_http: bool,

    /// Explicitly permit Minecraft to run without a platform OS sandbox when
    /// the host helper is unavailable. This weakens tenant isolation.
    #[arg(long, env = "MCPANEL_ALLOW_UNSANDBOXED_SERVERS")]
    allow_unsandboxed_servers: bool,

    /// Trust forwarded client-IP headers from this proxy peer. Repeat the
    /// option or separate values with commas; never include untrusted peers.
    #[arg(
        long,
        value_name = "IP",
        value_delimiter = ',',
        env = "MCPANEL_TRUSTED_PROXIES"
    )]
    trusted_proxies: Vec<IpAddr>,

    /// Maximum multipart upload request size.
    #[arg(
        long,
        default_value_t = limits::DEFAULT_MAX_UPLOAD_BYTES,
        env = "MCPANEL_MAX_UPLOAD_BYTES"
    )]
    max_upload_bytes: u64,

    /// Maximum Modrinth plugin/mod download size.
    #[arg(
        long,
        default_value_t = limits::DEFAULT_MAX_DOWNLOAD_BYTES,
        env = "MCPANEL_MAX_DOWNLOAD_BYTES"
    )]
    max_download_bytes: u64,

    /// Maximum expanded bytes emitted by one archive extraction.
    #[arg(
        long,
        default_value_t = limits::DEFAULT_MAX_EXTRACTED_BYTES,
        env = "MCPANEL_MAX_EXTRACTED_BYTES"
    )]
    max_extracted_bytes: u64,

    /// Maximum entries accepted in one archive extraction.
    #[arg(
        long,
        default_value_t = limits::DEFAULT_MAX_ARCHIVE_ENTRIES,
        env = "MCPANEL_MAX_ARCHIVE_ENTRIES"
    )]
    max_archive_entries: usize,

    /// Maximum size of an individual extracted file.
    #[arg(
        long,
        default_value_t = limits::DEFAULT_MAX_EXTRACTED_FILE_BYTES,
        env = "MCPANEL_MAX_EXTRACTED_FILE_BYTES"
    )]
    max_extracted_file_bytes: u64,

    /// Maximum total on-disk size of one server directory.
    #[arg(
        long,
        default_value_t = limits::DEFAULT_MAX_SERVER_DISK_BYTES,
        env = "MCPANEL_MAX_SERVER_DISK_BYTES"
    )]
    max_server_disk_bytes: u64,

    /// Maximum total compressed backup size retained for one server.
    #[arg(
        long,
        default_value_t = limits::DEFAULT_MAX_BACKUP_DISK_BYTES,
        env = "MCPANEL_MAX_BACKUP_DISK_BYTES"
    )]
    max_backup_disk_bytes: u64,

    /// Maximum compressed bytes accepted from one backup download during a
    /// restore, before the archive is extracted.
    #[arg(
        long,
        default_value_t = limits::DEFAULT_MAX_BACKUP_ARCHIVE_BYTES,
        env = "MCPANEL_MAX_BACKUP_ARCHIVE_BYTES"
    )]
    max_backup_archive_bytes: u64,

    /// Maximum aggregate configured Java heap, in MiB. Defaults to 75% of
    /// detected host memory and must not exceed that detected budget.
    #[arg(long, env = "MCPANEL_MAX_SERVER_MEMORY_MB")]
    max_server_memory_mb: Option<u32>,

    /// Select the embedded Playit runtime or a separately managed external daemon.
    #[arg(
        long,
        value_enum,
        default_value = "embedded",
        env = "MCPANEL_PLAYIT_MODE"
    )]
    playit_mode: PlayitMode,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| {
                    "mcpanel=info,panel=info,playit_integration=info,playit_runtime=info,playit_agent_core=info,tower_http=warn".into()
                }),
        )
        .init();

    let args = Args::parse();
    let effective_bind = resolve_effective_bind(args.bind, &args.data_dir);
    if !effective_bind.ip().is_loopback() && !args.allow_insecure_http {
        anyhow::bail!(
            "non-loopback HTTP is disabled; terminate TLS in a reverse proxy or pass --allow-insecure-http only for a deliberately isolated network"
        );
    }
    let detected_memory_budget = limits::host_memory_budget_mb();
    let max_server_memory_mb = args.max_server_memory_mb.unwrap_or(detected_memory_budget);
    if max_server_memory_mb > detected_memory_budget {
        anyhow::bail!(
            "max configured server memory ({max_server_memory_mb} MiB) exceeds the detected host budget ({detected_memory_budget} MiB)"
        );
    }
    let limits = ResourceLimits {
        max_upload_bytes: args.max_upload_bytes,
        max_download_bytes: args.max_download_bytes,
        max_extracted_bytes: args.max_extracted_bytes,
        max_archive_entries: args.max_archive_entries,
        max_extracted_file_bytes: args.max_extracted_file_bytes,
        max_server_disk_bytes: args.max_server_disk_bytes,
        max_backup_disk_bytes: args.max_backup_disk_bytes,
        max_backup_archive_bytes: args.max_backup_archive_bytes,
        max_server_memory_mb,
    };
    limits.validate()?;
    let state = AppState::bootstrap_with_limits_and_sandbox_and_trusted_proxies(
        &args.data_dir,
        args.playit_mode,
        limits,
        args.allow_unsandboxed_servers,
        args.trusted_proxies,
    )
    .await?;

    let server_result = async {
        // No automatic admin creation. First-run setup is via browser /setup.
        if state.store.read().await.users.is_empty() {
            tracing::info!("no users found; panel is in setup mode at /setup");
        }

        let mut app = Router::new()
            .nest("/api", api::router_with_limits(state.limits))
            .fallback(web::serve)
            .layer(
                TraceLayer::new_for_http().make_span_with(|request: &Request<Body>| {
                    // Log only the path. The default tower-http span includes
                    // the complete URI and headers, which would put tickets,
                    // cookies, or Authorization values into logs.
                    tracing::info_span!(
                        "http_request",
                        method = %request.method(),
                        path = %request.uri().path()
                    )
                }),
            )
            .layer(axum::middleware::from_fn_with_state(
                state.clone(),
                security_headers,
            ));

        if args.dev_cors {
            app = app.layer(CorsLayer::very_permissive());
            tracing::warn!("permissive CORS is enabled; do not use this in production");
        }

        let app = app.with_state(state.clone());

        let (listener, local_addr) = bind_with_fallback(effective_bind, &state.data_dir).await?;
        // Persist instance lock for single-instance detection and tray discovery.
        if let Err(e) = write_instance_lock(&state.data_dir, local_addr).await {
            tracing::warn!(error=%e, "could not write instance lock");
        }
        let panel_url_str = panel_url(local_addr);
        let tray = match tray::start(TrayConfig::new(panel_url_str.clone())) {
            Ok(tray) => Some(tray),
            Err(error) => {
                tracing::warn!(error = %error, "system tray is unavailable; the panel will continue without it");
                None
            }
        };
        let tray_exit = tray.as_ref().map(TrayHandle::exit_signal);
        let tray_reset = tray.as_ref().map(TrayHandle::reset_signal);

        tracing::info!(
            "panel listening on http://{} (use a TLS reverse proxy for remote access)",
            local_addr
        );
        if local_addr != effective_bind {
            tracing::info!(
                "requested {} but bound to {} (port file: {}/port or --bind to pin)",
                effective_bind,
                local_addr,
                state.data_dir.display()
            );
        }

        // First-run desktop experience: open setup automatically when uninitialized.
        if state.store.read().await.users.is_empty() {
            let setup_url = format!("{}/setup", panel_url_str);
            tracing::info!("first run: opening setup page");
            open_browser(&setup_url);
        }

        // Handle tray password recovery requests.
        let recovery_state = Arc::clone(&state);
        let recovery_panel_url = panel_url_str.clone();
        let recovery_task = tray_reset.map(|mut rx| {
            tokio::spawn(async move {
                let mut last = *rx.borrow();
                loop {
                    if rx.changed().await.is_err() {
                        break;
                    }
                    let current = *rx.borrow();
                    if current == last {
                        continue;
                    }
                    last = current;
                    // Find administrator to reset (first admin).
                    let admin = {
                        let data = recovery_state.store.read().await;
                        data.users.iter().find(|u| u.admin).cloned()
                    };
                    let Some(admin) = admin else {
                        tracing::warn!("password recovery requested but no administrator exists");
                        continue;
                    };
                    let token = recovery_state.recovery.generate(admin.username.clone());
                    tracing::info!(user = %admin.username, "password recovery requested");
                    let url = format!("{}/recovery#{}", recovery_panel_url, token);
                    // Never log token or url with token.
                    open_browser(&url);
                }
            })
        });

        let serve_result = axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(shutdown_with_tray(tray_exit))
        .await
        .context("server error");

        if let Some(task) = recovery_task {
            task.abort();
        }
        if let Some(tray) = tray {
            tray.shutdown();
        }

        // Clean up instance lock (data_dir is canonical after bootstrap).
        remove_instance_lock(&state.data_dir).await;

        serve_result
    }
    .await;

    // Also ensure lock is gone if bootstrap succeeded but server never started (bind error).
    // state.data_dir is canonical; best-effort.
    remove_instance_lock(&state.data_dir).await;

    state.shutdown_servers().await;

    if let Err(error) = state.playit.shutdown().await {
        tracing::error!(error = %error, "failed to shut down Playit runtime cleanly");
    }

    server_result
}

fn open_browser(url: &str) {
    // Use tray helper when available, otherwise direct spawn.
    // Replicate minimal cross-platform opening without shell.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // rundll32 is a console program; without this flag it allocates a
        // visible console window even when the panel itself has none.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let _ = std::process::Command::new("rundll32.exe")
            .args(["url.dll,FileProtocolHandler", url])
            .creation_flags(CREATE_NO_WINDOW)
            .spawn();
        return;
    }
    #[cfg(all(target_os = "linux", not(target_env = "musl")))]
    {
        let candidates: [&[&str]; 4] = [
            &["xdg-open", url],
            &["gio", "open", url],
            &["kde-open5", url],
            &["sensible-browser", url],
        ];
        for candidate in candidates {
            if std::process::Command::new(candidate[0])
                .args(&candidate[1..])
                .spawn()
                .is_ok()
            {
                return;
            }
        }
        tracing::warn!("could not open browser for setup/recovery");
        return;
    }
    #[cfg(not(any(windows, all(target_os = "linux", not(target_env = "musl")))))]
    {
        // Fallback: try via `xdg-open` or warn.
        let _ = url;
        tracing::warn!("browser opening not supported on this platform");
    }
}

/// Resolve on Ctrl-C, or on SIGTERM under a service manager.
async fn shutdown() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut signal) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            signal.recv().await;
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {}
        _ = terminate => {}
    }

    tracing::info!("shutting down");
}

/// Resolve on a native tray Exit action or the usual process signals.
async fn shutdown_with_tray(tray_exit: Option<watch::Receiver<bool>>) {
    let Some(mut tray_exit) = tray_exit else {
        shutdown().await;
        return;
    };

    tokio::select! {
        _ = shutdown() => {}
        changed = tray_exit.changed() => {
            if changed.is_ok() && *tray_exit.borrow() {
                tracing::info!("shutting down from the system tray");
            }
        }
    }
}

async fn security_headers(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let peer_is_trusted = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .is_some_and(|ConnectInfo(address)| state.trusted_proxies.contains(&address.ip()));
    let secure = request
        .headers()
        .get("x-forwarded-proto")
        .and_then(|value| value.to_str().ok())
        .filter(|_| peer_is_trusted)
        .is_some_and(|value| {
            value
                .split(',')
                .next()
                .is_some_and(|item| item.trim() == "https")
        });
    let mut response = next.run(request).await;
    add_security_headers(response.headers_mut(), secure);
    response
}

fn add_security_headers(headers: &mut HeaderMap, secure: bool) {
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "default-src 'self'; base-uri 'none'; object-src 'none'; frame-ancestors 'none'; form-action 'self'; script-src 'self'; style-src 'self'; img-src 'self' data: https:; connect-src 'self' ws: wss:",
        ),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(
        "permissions-policy",
        HeaderValue::from_static("camera=(), microphone=(), geolocation=(), payment=()"),
    );
    headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    if secure {
        headers.insert(
            header::STRICT_TRANSPORT_SECURITY,
            HeaderValue::from_static("max-age=31536000; includeSubDomains"),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_security_headers_are_restrictive_and_hsts_is_conditional() {
        let mut headers = HeaderMap::new();
        add_security_headers(&mut headers, false);
        let csp = headers
            .get(header::CONTENT_SECURITY_POLICY)
            .unwrap()
            .to_str()
            .unwrap();
        assert!(csp.contains("script-src 'self'"));
        assert!(csp.contains("frame-ancestors 'none'"));
        assert_eq!(
            headers.get(header::X_CONTENT_TYPE_OPTIONS).unwrap(),
            "nosniff"
        );
        assert_eq!(headers.get(header::X_FRAME_OPTIONS).unwrap(), "DENY");
        assert!(headers.get(header::STRICT_TRANSPORT_SECURITY).is_none());

        add_security_headers(&mut headers, true);
        assert!(headers.get(header::STRICT_TRANSPORT_SECURITY).is_some());
    }

    #[test]
    fn tray_urls_follow_the_bound_listener_address() {
        assert_eq!(
            panel_url("127.0.0.1:8080".parse().unwrap()),
            "http://127.0.0.1:8080"
        );
        assert_eq!(
            panel_url("0.0.0.0:8080".parse().unwrap()),
            "http://127.0.0.1:8080"
        );
        assert_eq!(
            panel_url("[::1]:8080".parse().unwrap()),
            "http://[::1]:8080"
        );
        assert_eq!(panel_url("[::]:8080".parse().unwrap()), "http://[::1]:8080");
        assert_eq!(
            panel_url("192.0.2.10:8080".parse().unwrap()),
            "http://192.0.2.10:8080"
        );
    }

    #[tokio::test]
    async fn tray_exit_signal_requests_graceful_shutdown() {
        let (sender, receiver) = watch::channel(false);
        sender.send(true).expect("test receiver should be alive");

        tokio::time::timeout(
            std::time::Duration::from_secs(1),
            shutdown_with_tray(Some(receiver)),
        )
        .await
        .expect("tray exit should wake the shutdown path");
    }
}
