//! Creating, inspecting, configuring and powering servers.

use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use guardian::{GuardianConfig, Memory, ServerConfig};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::auth::{AdminIdentity, Identity};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use crate::store::ServerRecord;

/// A server as the API presents it: stored config plus live status.
#[derive(Serialize)]
pub struct ServerView {
    id: String,
    name: String,
    core: String,
    version: String,
    port: u16,
    java_major: u32,
    memory: Memory,
    eula_accepted: bool,
    jvm_args: Vec<String>,
    server_args: Vec<String>,
    policy: GuardianConfig,
    created_at: String,
    #[serde(flatten)]
    live: guardian::Snapshot,
    /// CPU and memory of this server's JVM, when one is running.
    metrics: Option<crate::metrics::ProcessMetrics>,
}

async fn view(state: &AppState, record: &ServerRecord) -> ApiResult<ServerView> {
    let live = state.guardian(&record.id).await?.snapshot().await;
    let metrics = live.pid.and_then(|pid| state.metrics.of(pid));
    Ok(ServerView {
        id: record.id.clone(),
        name: record.name.clone(),
        core: record.config.core.clone(),
        version: record.config.version.clone(),
        port: record.config.port,
        java_major: record.config.java_major,
        memory: record.config.memory,
        eula_accepted: record.config.eula_accepted,
        jvm_args: record.config.jvm_args.clone(),
        server_args: record.config.server_args.clone(),
        policy: record.policy.clone(),
        created_at: record.created_at.clone(),
        live,
        metrics,
    })
}

/// Resolve a record the caller is allowed to see.
async fn authorized(
    state: &AppState,
    identity: &Identity,
    id: &str,
) -> ApiResult<ServerRecord> {
    if !identity.may_access(id) {
        return Err(ApiError::Forbidden);
    }
    state
        .store
        .server(id)
        .await
        .ok_or_else(|| ApiError::NotFound(format!("server {id}")))
}

async fn list(
    State(state): State<Arc<AppState>>,
    identity: Identity,
) -> ApiResult<Json<Vec<ServerView>>> {
    let records = state.store.read().await.servers;
    let mut out = Vec::new();
    for record in records {
        if identity.may_access(&record.id) {
            out.push(view(&state, &record).await?);
        }
    }
    Ok(Json(out))
}

/// Body of `POST /api/servers`.
#[derive(Deserialize)]
pub struct CreateServer {
    name: String,
    core: String,
    version: String,
    #[serde(default)]
    build: Option<String>,
    #[serde(default = "default_java")]
    java_major: u32,
    #[serde(default)]
    memory: Memory,
    #[serde(default = "default_port")]
    port: u16,
    /// The panel refuses to start a server whose operator has not accepted this.
    #[serde(default)]
    eula_accepted: bool,
}

fn default_java() -> u32 {
    21
}

fn default_port() -> u16 {
    25565
}

async fn create(
    State(state): State<Arc<AppState>>,
    AdminIdentity(admin): AdminIdentity,
    Json(body): Json<CreateServer>,
) -> ApiResult<Json<ServerView>> {
    if body.name.trim().is_empty() {
        return Err(ApiError::BadRequest("name is required".into()));
    }

    port_is_free(&state, body.port, None).await?;

    let id = Uuid::new_v4().to_string();
    let directory = state.server_dir(&id);
    tokio::fs::create_dir_all(&directory)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;

    let record = ServerRecord {
        id: id.clone(),
        name: body.name.trim().to_string(),
        config: ServerConfig {
            core: body.core,
            version: body.version,
            build: body.build,
            java_major: body.java_major,
            memory: body.memory,
            jvm_args: Vec::new(),
            server_args: vec!["nogui".into()],
            directory,
            port: body.port,
            eula_accepted: body.eula_accepted,
        },
        policy: GuardianConfig::default(),
        created_at: now(),
    };

    state.store.update(|data| data.servers.push(record.clone())).await?;
    state.insert_guardian(&record).await;
    tracing::info!(server = %record.id, by = %admin.username, "server created");

    Ok(Json(view(&state, &record).await?))
}

async fn get_one(
    State(state): State<Arc<AppState>>,
    identity: Identity,
    Path(id): Path<String>,
) -> ApiResult<Json<ServerView>> {
    let record = authorized(&state, &identity, &id).await?;
    Ok(Json(view(&state, &record).await?))
}

/// Body of `PATCH /api/servers/:id`. Every field is optional.
#[derive(Deserialize)]
pub struct UpdateServer {
    name: Option<String>,
    core: Option<String>,
    version: Option<String>,
    build: Option<Option<String>>,
    java_major: Option<u32>,
    memory: Option<Memory>,
    port: Option<u16>,
    eula_accepted: Option<bool>,
    jvm_args: Option<Vec<String>>,
    server_args: Option<Vec<String>>,
    policy: Option<GuardianConfig>,
}

async fn update(
    State(state): State<Arc<AppState>>,
    identity: Identity,
    Path(id): Path<String>,
    Json(body): Json<UpdateServer>,
) -> ApiResult<Json<ServerView>> {
    let mut record = authorized(&state, &identity, &id).await?;

    if let Some(name) = body.name {
        record.name = name;
    }
    if let Some(core) = body.core {
        record.config.core = core;
    }
    if let Some(version) = body.version {
        record.config.version = version;
    }
    if let Some(build) = body.build {
        record.config.build = build;
    }
    if let Some(java) = body.java_major {
        record.config.java_major = java;
    }
    if let Some(memory) = body.memory {
        record.config.memory = memory;
    }
    if let Some(port) = body.port {
        // Create checked this; update has to as well, or two servers end up
        // fighting over the same port and the second one dies at bind time.
        port_is_free(&state, port, Some(&id)).await?;
        record.config.port = port;
    }
    if let Some(eula) = body.eula_accepted {
        record.config.eula_accepted = eula;
    }
    if let Some(args) = body.jvm_args {
        record.config.jvm_args = args;
    }
    if let Some(args) = body.server_args {
        record.config.server_args = args;
    }
    if let Some(policy) = body.policy {
        record.policy = policy;
    }

    let stored = record.clone();
    state
        .store
        .update(move |data| {
            if let Some(slot) = data.servers.iter_mut().find(|s| s.id == stored.id) {
                *slot = stored;
            }
        })
        .await?;

    // Config changes apply on the next start; a running server keeps its
    // current process rather than being restarted out from under its players.
    let guardian = state.guardian(&id).await?;
    guardian.set_config(record.config.clone()).await;
    guardian.set_policy(record.policy.clone()).await;

    Ok(Json(view(&state, &record).await?))
}

async fn delete(
    State(state): State<Arc<AppState>>,
    AdminIdentity(admin): AdminIdentity,
    Path(id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    state
        .store
        .server(&id)
        .await
        .ok_or_else(|| ApiError::NotFound(format!("server {id}")))?;

    tracing::info!(server = %id, by = %admin.username, "server deleted");
    state.remove_guardian(&id).await;
    state.store.update(|data| data.servers.retain(|s| s.id != id)).await?;

    // Server files are deliberately left on disk: deleting a world by clicking
    // a button in a web UI is not a mistake anyone should be able to make.
    Ok(Json(serde_json::json!({ "ok": true, "files_kept": true })))
}

/// Body of `POST /api/servers/:id/power`.
#[derive(Deserialize)]
pub struct PowerRequest {
    action: PowerAction,
}

/// The four things you can do to a running process.
#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PowerAction {
    /// Provision if needed, then spawn.
    Start,
    /// Graceful shutdown, with a kill fallback.
    Stop,
    /// Stop then start.
    Restart,
    /// SIGKILL now.
    Kill,
}

async fn power(
    State(state): State<Arc<AppState>>,
    identity: Identity,
    Path(id): Path<String>,
    Json(body): Json<PowerRequest>,
) -> ApiResult<Json<guardian::Snapshot>> {
    authorized(&state, &identity, &id).await?;
    let guardian = state.guardian(&id).await?;

    match body.action {
        PowerAction::Start => guardian.start().await?,
        PowerAction::Stop => guardian.stop().await?,
        PowerAction::Restart => guardian.restart().await?,
        PowerAction::Kill => guardian.kill().await?,
    }

    Ok(Json(guardian.snapshot().await))
}

/// Body of `POST /api/servers/:id/command`.
#[derive(Deserialize)]
pub struct CommandRequest {
    command: String,
}

async fn command(
    State(state): State<Arc<AppState>>,
    identity: Identity,
    Path(id): Path<String>,
    Json(body): Json<CommandRequest>,
) -> ApiResult<Json<serde_json::Value>> {
    authorized(&state, &identity, &id).await?;
    state.guardian(&id).await?.command(body.command.trim()).await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

async fn logs(
    State(state): State<Arc<AppState>>,
    identity: Identity,
    Path(id): Path<String>,
) -> ApiResult<Json<Vec<guardian::ConsoleLine>>> {
    authorized(&state, &identity, &id).await?;
    Ok(Json(state.guardian(&id).await?.console().await))
}

/// Reject a port already assigned to a different server.
async fn port_is_free(state: &AppState, port: u16, except: Option<&str>) -> ApiResult<()> {
    let clash = state
        .store
        .read()
        .await
        .servers
        .iter()
        .any(|s| s.config.port == port && Some(s.id.as_str()) != except);

    if clash {
        return Err(ApiError::BadRequest(format!("port {port} is already assigned")));
    }
    Ok(())
}

fn now() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| String::from("1970-01-01T00:00:00Z"))
}

/// The collection itself, mounted at `/api/servers` so it matches with and
/// without a trailing slash regardless of how the client spells it.
pub fn collection_router() -> Router<Arc<AppState>> {
    Router::new().route("/servers", get(list).post(create))
}

/// Per-server routes, nested under `/api/servers`.
pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/{id}", get(get_one).patch(update).delete(delete))
        .route("/{id}/power", post(power))
        .route("/{id}/command", post(command))
        .route("/{id}/logs", get(logs))
}
