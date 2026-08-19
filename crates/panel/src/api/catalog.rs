//! What can be installed: server flavours, their versions, and local Javas.
//!
//! These endpoints proxy `minecraft-core` and `java-path` so the browser never
//! talks to PaperMC or Adoptium directly.

use axum::extract::{Path, State};
use axum::routing::get;
use axum::{Json, Router};
use minecraft_core::MinecraftClient;
use serde::Serialize;
use std::sync::Arc;

use crate::auth::Identity;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

/// One installable server flavour.
#[derive(Serialize)]
pub struct ProviderView {
    id: String,
    /// Whether this provider produces a server rather than a proxy.
    server: bool,
}

fn client() -> ApiResult<MinecraftClient> {
    MinecraftClient::builder()
        .build()
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("minecraft client: {e}")))
}

async fn providers(_: Identity) -> ApiResult<Json<Vec<ProviderView>>> {
    let ids = client()?.providers();
    Ok(Json(
        ids.into_iter()
            .map(|id| {
                let id = id.to_string();
                // Velocity and Waterfall are proxies: they have no world and
                // the UI should not offer them a "start the world" flow.
                let server = !matches!(id.as_str(), "velocity" | "waterfall");
                ProviderView { id, server }
            })
            .collect(),
    ))
}

async fn versions(_: Identity, Path(provider): Path<String>) -> ApiResult<Json<Vec<String>>> {
    let versions = client()?
        .versions(&provider)
        .await
        .map_err(|e| ApiError::BadRequest(format!("{provider}: {e}")))?;

    // Newest first: nobody scrolls to the bottom looking for 1.21.
    let mut out: Vec<String> = versions.into_iter().map(|v| v.to_string()).collect();
    out.reverse();
    Ok(Json(out))
}

/// One build of one version.
#[derive(Serialize)]
pub struct BuildView {
    build: String,
    published_at: Option<String>,
}

async fn builds(
    _: Identity,
    Path((provider, version)): Path<(String, String)>,
) -> ApiResult<Json<Vec<BuildView>>> {
    let builds = client()?
        .builds(&provider, &version)
        .await
        .map_err(|e| ApiError::BadRequest(format!("{provider} {version}: {e}")))?;

    Ok(Json(
        builds
            .into_iter()
            .rev()
            .map(|b| BuildView {
                build: b.build_id.to_string(),
                published_at: b.published_at,
            })
            .collect(),
    ))
}

/// A Java installation the host already has.
#[derive(Serialize)]
pub struct JavaView {
    major: u32,
    version: String,
    path: String,
    vendor: Option<String>,
    jdk: bool,
}

async fn javas(_: Identity) -> Json<Vec<JavaView>> {
    let installs = java_path::discover().unwrap_or_default();
    let mut out: Vec<JavaView> = installs
        .iter()
        .map(|i| JavaView {
            major: i.major(),
            version: i.version.to_string(),
            path: i.java.display().to_string(),
            vendor: i.vendor.clone(),
            jdk: i.is_jdk(),
        })
        .collect();
    out.sort_by_key(|java| std::cmp::Reverse(java.major));
    Json(out)
}

/// Host resource usage, for the dashboard header.
#[derive(Serialize)]
pub struct SystemStats {
    cpu_percent: f32,
    memory_used_mb: u64,
    memory_total_mb: u64,
    servers_online: usize,
}

async fn system(
    State(state): State<Arc<AppState>>,
    identity: Identity,
) -> ApiResult<Json<SystemStats>> {
    let mut sys = sysinfo::System::new();
    sys.refresh_memory();
    sys.refresh_cpu_usage();

    let mut online = 0;
    for record in state.store.read().await.servers {
        if !identity.may_access(&record.id) {
            continue;
        }
        if state.guardian(&record.id).await?.status().await.is_running() {
            online += 1;
        }
    }

    Ok(Json(SystemStats {
        cpu_percent: sys.global_cpu_usage(),
        memory_used_mb: sys.used_memory() / 1024 / 1024,
        memory_total_mb: sys.total_memory() / 1024 / 1024,
        servers_online: online,
    }))
}

/// Routes under `/api`.
pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/catalog/providers", get(providers))
        .route("/catalog/{provider}/versions", get(versions))
        .route("/catalog/{provider}/{version}/builds", get(builds))
        .route("/catalog/javas", get(javas))
        .route("/system", get(system))
}
