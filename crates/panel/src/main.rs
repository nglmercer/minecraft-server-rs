//! A single-binary Minecraft server control panel.
//!
//! ```text
//! mcpanel --data-dir ./data --bind 0.0.0.0:8080
//! ```
//!
//! On first run an `admin` account is created and its generated password is
//! printed once. Everything else — servers, users, sessions — is managed
//! through the web UI or the REST API under `/api`.

#![forbid(unsafe_code)]

mod api;
mod auth;
mod error;
mod metrics;
mod state;
mod store;
mod tickets;
mod web;

use anyhow::{Context, Result};
use axum::Router;
use clap::Parser;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

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
    let state = AppState::bootstrap(&args.data_dir, args.playit_mode).await?;

    let server_result = async {
        ensure_admin(&state).await?;

        let mut app = Router::new()
            .nest("/api", api::router())
            .fallback(web::serve)
            .layer(TraceLayer::new_for_http());

        if args.dev_cors {
            app = app.layer(CorsLayer::very_permissive());
            tracing::warn!("permissive CORS is enabled; do not use this in production");
        }

        let app = app.with_state(state.clone());

        let listener = tokio::net::TcpListener::bind(args.bind)
            .await
            .with_context(|| format!("binding {}", args.bind))?;

        tracing::info!("panel listening on http://{}", args.bind);

        axum::serve(listener, app)
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
    if !state.store.read().await.users.is_empty() {
        return Ok(());
    }

    let password = auth::generate_password();
    let hash = auth::hash_password(&password)
        .map_err(|e| anyhow::anyhow!("hashing the initial password failed: {e}"))?;

    state
        .store
        .update(|data| {
            data.users.push(User {
                username: "admin".into(),
                password_hash: hash,
                admin: true,
                servers: Vec::new(),
            })
        })
        .await?;

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
