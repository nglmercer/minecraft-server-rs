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

mod api;
mod auth;
mod error;
mod filesystem;
mod limits;
mod metrics;
mod state;
mod store;
mod tickets;
mod web;

use anyhow::{Context, Result};
use axum::body::Body;
use axum::http::{header, HeaderMap, HeaderValue, Request};
use axum::middleware::Next;
use axum::response::Response;
use axum::Router;
use clap::Parser;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

use crate::limits::ResourceLimits;
use crate::state::AppState;
use crate::state::PlayitMode;
use crate::store::User;

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

    /// Address to listen on.
    #[arg(long, default_value = "127.0.0.1:8080", env = "MCPANEL_BIND")]
    bind: SocketAddr,

    /// Allow browser requests from any origin. Needed only for `npm run dev`.
    #[arg(long, env = "MCPANEL_DEV_CORS")]
    dev_cors: bool,

    /// Permit plaintext HTTP on a non-loopback bind. Prefer a TLS reverse
    /// proxy; this flag explicitly acknowledges the risk.
    #[arg(long, env = "MCPANEL_ALLOW_INSECURE_HTTP")]
    allow_insecure_http: bool,

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
    if !args.bind.ip().is_loopback() && !args.allow_insecure_http {
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
        max_server_memory_mb,
    };
    limits.validate()?;
    let state = AppState::bootstrap_with_limits(&args.data_dir, args.playit_mode, limits).await?;

    let server_result = async {
        ensure_admin(&state).await?;

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
            .layer(axum::middleware::from_fn(security_headers));

        if args.dev_cors {
            app = app.layer(CorsLayer::very_permissive());
            tracing::warn!("permissive CORS is enabled; do not use this in production");
        }

        let app = app.with_state(state.clone());

        let listener = tokio::net::TcpListener::bind(args.bind)
            .await
            .with_context(|| format!("binding {}", args.bind))?;

        tracing::info!(
            "panel listening on http://{} (use a TLS reverse proxy for remote access)",
            args.bind
        );

        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(shutdown())
        .await
        .context("server error")?;

        Ok::<(), anyhow::Error>(())
    }
    .await;

    if let Err(error) = state.playit.shutdown().await {
        tracing::error!(error = %error, "failed to shut down Playit runtime cleanly");
    }

    server_result
}

/// Create the initial admin account when the panel has no users yet.
async fn ensure_admin(state: &Arc<AppState>) -> Result<()> {
    let password = auth::generate_password();
    let hash = auth::hash_password(&password)
        .map_err(|e| anyhow::anyhow!("hashing the initial password failed: {e}"))?;

    let created = state
        .store
        .update(|data| {
            if !data.users.is_empty() {
                return false;
            }
            data.users.push(User {
                username: "admin".into(),
                password_hash: hash,
                admin: true,
                servers: Vec::new(),
            });
            true
        })
        .await?;

    if !created {
        return Ok(());
    }

    // Printed exactly once, and never recoverable afterwards.
    println!("\n  Created the initial administrator account:\n");
    println!("      username: admin");
    println!("      password: {password}\n");
    println!("  Change it from the panel; this is the only time it is shown.\n");

    Ok(())
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

async fn security_headers(request: Request<Body>, next: Next) -> Response {
    let secure = request
        .headers()
        .get("x-forwarded-proto")
        .and_then(|value| value.to_str().ok())
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
}
