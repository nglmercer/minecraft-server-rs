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
    /// What is actually installed on disk, which may lag the config.
    installed: Option<guardian::Installation>,
    /// Whether starting would download a new artifact first.
    needs_install: bool,
    /// Bytes on disk in this server's directory.
    disk_bytes: u64,
    /// The panel-owned Playit association, when one exists.
    playit: Option<crate::store::PlayitBinding>,
    /// Whether the running process was launched with a different configuration
    /// than the one stored, so the edit only takes effect on the next start.
    pending_restart: bool,
}

/// Whether a live process would have to restart to honour `stored`.
fn differs_at_launch(running: &ServerConfig, stored: &ServerConfig) -> bool {
    running.memory.min_mb != stored.memory.min_mb
        || running.memory.max_mb != stored.memory.max_mb
        || running.port != stored.port
        || running.jvm_args != stored.jvm_args
        || running.server_args != stored.server_args
        || running.artifact_key() != stored.artifact_key()
}

async fn view(
    state: &AppState,
    record: &ServerRecord,
    show_jvm_args: bool,
) -> ApiResult<ServerView> {
    let guardian = state.guardian(&record.id).await?;
    let live = guardian.snapshot().await;
    let metrics = live.pid.and_then(|pid| state.metrics.of(pid));

    let disk_bytes = state.metrics.disk_usage(&record.config.directory).await;
    let pending_restart = guardian
        .active_config()
        .await
        .is_some_and(|running| differs_at_launch(&running, &record.config));
    let installed = guardian.installation().await;
    let needs_install = installed
        .as_ref()
        .map(|i| !i.satisfies(&record.config))
        .unwrap_or(true);
    Ok(ServerView {
        id: record.id.clone(),
        name: record.name.clone(),
        core: record.config.core.clone(),
        version: record.config.version.clone(),
        port: record.config.port,
        java_major: record.config.java_major,
        memory: record.config.memory,
        eula_accepted: record.config.eula_accepted,
        // JVM arguments are an administrator-controlled field. In addition to
        // rejecting non-admin writes, do not disclose them to server-only
        // operators because flags can contain paths, agent settings, or other
        // deployment-specific data.
        jvm_args: if show_jvm_args {
            record.config.jvm_args.clone()
        } else {
            Vec::new()
        },
        server_args: record.config.server_args.clone(),
        policy: record.policy.clone(),
        created_at: record.created_at.clone(),
        live,
        metrics,
        installed,
        needs_install,
        disk_bytes,
        playit: record.playit.clone(),
        pending_restart,
    })
}

/// Resolve a record the caller is allowed to see.
async fn authorized(state: &AppState, identity: &Identity, id: &str) -> ApiResult<ServerRecord> {
    if !identity.may_access(id) {
        return Err(ApiError::NotFound("server".into()));
    }
    state
        .store
        .server(id)
        .await
        .ok_or_else(|| ApiError::NotFound("server".into()))
}

async fn list(
    State(state): State<Arc<AppState>>,
    identity: Identity,
) -> ApiResult<Json<Vec<ServerView>>> {
    let records = state.store.read().await.servers;
    let mut out = Vec::new();
    for record in records {
        if identity.may_access(&record.id) {
            out.push(view(&state, &record, identity.admin).await?);
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

fn validate_name(name: &str) -> ApiResult<String> {
    let name = name.trim();
    if name.is_empty() || name.len() > 128 || name.contains(['\0', '\r', '\n']) {
        return Err(ApiError::BadRequest(
            "name must be 1-128 bytes and contain no control characters".into(),
        ));
    }
    Ok(name.to_owned())
}

fn validate_record(state: &AppState, record: &ServerRecord) -> ApiResult<()> {
    validate_record_with_max(state.limits.max_server_memory_mb, record)
}

fn validate_record_with_max(max_memory_mb: u32, record: &ServerRecord) -> ApiResult<()> {
    record.config.validate().map_err(ApiError::BadRequest)?;
    if record.config.memory.max_mb > max_memory_mb {
        return Err(ApiError::BadRequest(format!(
            "memory max_mb must not exceed {} MiB",
            max_memory_mb
        )));
    }
    record.policy.validate().map_err(ApiError::BadRequest)?;
    Ok(())
}

/// Launch configurations of the servers whose JVMs are still alive, by id.
type ActiveConfigs = std::collections::HashMap<String, ServerConfig>;

/// Whether `port` is already spoken for by a server other than `excluding`.
///
/// A running server holds the port it was launched with even after its stored
/// config has been pointed somewhere else, so both values are reserved until
/// the process is restarted.
fn port_is_taken(
    servers: &[ServerRecord],
    active: &ActiveConfigs,
    port: u16,
    excluding: Option<&str>,
) -> bool {
    servers.iter().any(|server| {
        if excluding == Some(server.id.as_str()) {
            return false;
        }
        server.config.port == port
            || active
                .get(&server.id)
                .is_some_and(|running| running.port == port)
    })
}

/// The heap a server occupies right now: the larger of stored and running.
fn reserved_memory_mb(id: &str, configured: u32, active: &ActiveConfigs) -> u64 {
    let running = active
        .get(id)
        .map(|config| config.memory.max_mb)
        .unwrap_or(0);
    configured.max(running) as u64
}

fn validate_aggregate_memory(
    servers: &[ServerRecord],
    active: &ActiveConfigs,
    replacing: Option<(&str, u32)>,
    additional: Option<u32>,
    max_memory_mb: u32,
) -> ApiResult<()> {
    let mut total = 0_u64;
    for server in servers {
        if replacing.is_some_and(|(id, _)| id == server.id) {
            continue;
        }
        total = total
            .checked_add(reserved_memory_mb(
                &server.id,
                server.config.memory.max_mb,
                active,
            ))
            .ok_or_else(|| ApiError::Conflict("aggregate server memory quota exceeded".into()))?;
    }
    if let Some((id, memory)) = replacing {
        // Lowering a running server's heap does not hand memory back: the JVM
        // keeps the maximum it was started with until it is restarted.
        total = total
            .checked_add(reserved_memory_mb(id, memory, active))
            .ok_or_else(|| ApiError::Conflict("aggregate server memory quota exceeded".into()))?;
    }
    if let Some(memory) = additional {
        total = total
            .checked_add(memory as u64)
            .ok_or_else(|| ApiError::Conflict("aggregate server memory quota exceeded".into()))?;
    }
    if total > max_memory_mb as u64 {
        return Err(ApiError::Conflict(
            "aggregate server memory quota exceeded".into(),
        ));
    }
    Ok(())
}

async fn create(
    State(state): State<Arc<AppState>>,
    AdminIdentity(admin): AdminIdentity,
    Json(body): Json<CreateServer>,
) -> ApiResult<Json<ServerView>> {
    let _server_lock = state.server_mutation_lock.lock().await;
    let name = validate_name(&body.name)?;

    let id = Uuid::new_v4().to_string();
    let directory = state.server_dir(&id);

    let record = ServerRecord {
        id: id.clone(),
        name,
        config: ServerConfig {
            core: body.core,
            version: body.version,
            build: body.build,
            java_major: body.java_major,
            memory: body.memory,
            jvm_args: Vec::new(),
            server_args: vec!["nogui".into()],
            directory: directory.clone(),
            port: body.port,
            eula_accepted: body.eula_accepted,
        },
        policy: GuardianConfig::default(),
        playit: None,
        created_at: now(),
        backup_policy: None,
    };
    validate_record(&state, &record)?;
    // Provisional directory: must be removed unless the store commit succeeds.
    crate::state::secure_directory(&directory)
        .await
        .map_err(ApiError::Internal)?;

    let max_memory_mb = state.limits.max_server_memory_mb;
    let active = state.active_configs().await;
    // Use a guard-style cleanup: any failure before commit removes the provisional directory.
    let write = state
        .store
        .try_update(|data| -> ApiResult<()> {
            if port_is_taken(data.servers.as_slice(), &active, record.config.port, None) {
                return Err(ApiError::Conflict(format!(
                    "port {} is already assigned",
                    record.config.port
                )));
            }
            validate_aggregate_memory(
                data.servers.as_slice(),
                &active,
                None,
                Some(record.config.memory.max_mb),
                max_memory_mb,
            )?;
            data.servers.push(record.clone());
            Ok(())
        })
        .await;

    let write = match write {
        Ok(inner) => inner,
        Err(error) => {
            // Persistence/I/O failure before commit: remove provisional directory.
            state.remove_uncommitted_server_dir(&id).await;
            return Err(ApiError::Internal(error));
        }
    };
    if let Err(error) = write {
        state.remove_uncommitted_server_dir(&id).await;
        return Err(error);
    }
    state.insert_guardian(&record).await;
    tracing::info!(server = %record.id, by = %admin.username, "server created");

    Ok(Json(view(&state, &record, true).await?))
}

async fn get_one(
    State(state): State<Arc<AppState>>,
    identity: Identity,
    Path(id): Path<String>,
) -> ApiResult<Json<ServerView>> {
    let record = authorized(&state, &identity, &id).await?;
    Ok(Json(view(&state, &record, identity.admin).await?))
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
    let _server_lock = state.server_mutation_lock.lock().await;
    authorized(&state, &identity, &id).await?;
    if body.jvm_args.is_some() && !identity.admin {
        return Err(ApiError::Forbidden);
    }
    let guardian = state.guardian(&id).await?;
    let _resource_lock = guardian.lock_resources().await;
    let max_memory_mb = state.limits.max_server_memory_mb;
    let active = state.active_configs().await;
    let server_id = id.clone();
    let record = state
        .store
        .try_update(move |data| -> ApiResult<ServerRecord> {
            let Some(slot) = data.servers.iter().find(|server| server.id == server_id) else {
                return Err(ApiError::NotFound("server".into()));
            };
            let mut next = slot.clone();

            if let Some(name) = body.name {
                next.name = validate_name(&name)?;
            }
            if let Some(core) = body.core {
                next.config.core = core;
            }
            if let Some(version) = body.version {
                next.config.version = version;
            }
            if let Some(build) = body.build {
                next.config.build = build;
            }
            if let Some(java) = body.java_major {
                next.config.java_major = java;
            }
            if let Some(memory) = body.memory {
                next.config.memory = memory;
            }
            if let Some(port) = body.port {
                if port_is_taken(data.servers.as_slice(), &active, port, Some(&next.id)) {
                    return Err(ApiError::Conflict(format!(
                        "port {port} is already assigned"
                    )));
                }
                if next
                    .playit
                    .as_ref()
                    .is_some_and(|binding| binding.local_port != port)
                {
                    return Err(ApiError::Conflict(
                        "disable the Playit tunnel before changing the server port".into(),
                    ));
                }
                next.config.port = port;
            }
            if let Some(eula) = body.eula_accepted {
                next.config.eula_accepted = eula;
            }
            if let Some(args) = body.jvm_args {
                next.config.jvm_args = args;
            }
            if let Some(args) = body.server_args {
                next.config.server_args = args;
            }
            if let Some(policy) = body.policy {
                next.policy = policy;
            }
            validate_record_with_max(max_memory_mb, &next)?;
            validate_aggregate_memory(
                data.servers.as_slice(),
                &active,
                Some((&next.id, next.config.memory.max_mb)),
                None,
                max_memory_mb,
            )?;
            *data
                .servers
                .iter_mut()
                .find(|server| server.id == next.id)
                .expect("the slot was found above") = next.clone();
            Ok(next)
        })
        .await??;

    // Config changes apply on the next start; a running server keeps its
    // current process rather than being restarted out from under its players.
    guardian.set_config(record.config.clone()).await;
    guardian.set_policy(record.policy.clone()).await;

    Ok(Json(view(&state, &record, identity.admin).await?))
}

async fn delete(
    State(state): State<Arc<AppState>>,
    AdminIdentity(admin): AdminIdentity,
    Path(id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    // Lock ordering: server_mutation_lock -> guardian resource lock -> store lock
    // This prevents races with server creation, file operations, and Playit attach.
    let _server_lock = state.server_mutation_lock.lock().await;
    // Validate existence before acquiring resource lock.
    state
        .store
        .server(&id)
        .await
        .ok_or_else(|| ApiError::NotFound(format!("server {id}")))?;

    let guardian = state.guardian(&id).await?;
    let _resource_lock = guardian.lock_resources().await;
    // Re-validate after acquiring the resource lock to avoid TOCTOU with concurrent delete.
    let record_after_lock = state
        .store
        .server(&id)
        .await
        .ok_or_else(|| ApiError::NotFound(format!("server {id}")))?;

    // ---- Lifecycle: ensure no managed process/preparation remains before committing deletion ----
    // Must not commit a successful logical deletion while a JVM or preparation task is still alive.
    let status = guardian.status().await;
    let needs_stop = status.is_running() || status == guardian::ServerStatus::Preparing;
    if needs_stop {
        guardian
            .stop()
            .await
            .map_err(|e| ApiError::Internal(anyhow::anyhow!("failed to stop server {id}: {e}")))?;
        if !guardian
            .wait_for_settled(std::time::Duration::from_secs(10))
            .await
        {
            return Err(ApiError::Internal(anyhow::anyhow!(
                "server {id} process did not exit after stop; deletion aborted"
            )));
        }
    } else {
        // Offline/Crashed: ensure no lingering preparation task or orphaned child.
        if !guardian
            .wait_for_settled(std::time::Duration::from_secs(10))
            .await
        {
            return Err(ApiError::Internal(anyhow::anyhow!(
                "server {id} preparation did not settle; deletion aborted"
            )));
        }
        let snapshot = guardian.snapshot().await;
        if snapshot.pid.is_some() {
            return Err(ApiError::Internal(anyhow::anyhow!(
                "server {id} still has live process"
            )));
        }
    }
    // Final verification that no process or preparation remains.
    {
        let snapshot = guardian.snapshot().await;
        if snapshot.status.is_running()
            || snapshot.status == guardian::ServerStatus::Preparing
            || snapshot.pid.is_some()
        {
            return Err(ApiError::Internal(anyhow::anyhow!(
                "server {id} still running after stop"
            )));
        }
        // Also check internal settled state directly.
        if !guardian
            .wait_for_settled(std::time::Duration::from_secs(1))
            .await
        {
            return Err(ApiError::Internal(anyhow::anyhow!(
                "server {id} did not settle"
            )));
        }
    }

    // ---- Atomically update persisted state: remove server and clean all user permissions + backups ----
    // External Playit cleanup happens after local commit so a failed tunnel delete
    // does not leave an orphan server record binding to a now-deleted tunnel.
    state
        .store
        .update(|data| {
            data.servers.retain(|s| s.id != id);
            for user in &mut data.users {
                user.servers.retain(|sid| sid != &id);
            }
            let before = data.backups.len();
            data.backups.retain(|b| b.server_id != id);
            if before != data.backups.len() {
                tracing::info!(server = %id, removed = before - data.backups.len(), "removed backup metadata for deleted server");
            }
        })
        .await
        .map_err(ApiError::Internal)?;
    tracing::info!(server = %id, by = %admin.username, "server deleted");

    // ---- External Playit cleanup after local commit (best-effort) ----
    if let Some(binding) = record_after_lock.playit.as_ref() {
        match state.playit.account_tunnels().await {
            Ok(tunnels) if tunnels.iter().any(|t| t.id == binding.tunnel_id) => {
                if let Err(e) = state.playit.delete_tunnel(&binding.tunnel_id).await {
                    tracing::warn!(server = %id, tunnel = %binding.tunnel_id, error = %e, "playit tunnel deletion failed after server removal; orphan tunnel may need manual cleanup");
                }
            }
            Ok(_) => {}
            Err(e) => {
                tracing::warn!(server = %id, error = %e, "playit status failed after server removal; tunnel may remain");
            }
        }
    }

    // ---- Remove guardian: already stopped and verified, just drop from map ----
    state
        .remove_guardian_verified(&id, guardian)
        .await
        .map_err(ApiError::Internal)?;

    // Server files are deliberately left on disk: deleting a world by clicking
    // a button in a web UI is not a mistake anyone should be able to make.
    Ok(Json(serde_json::json!({
        "ok": true,
        "files_kept": true,
        "playit_tunnel_deleted": record_after_lock.playit.is_some()
    })))
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
    let _server_lock = state.server_mutation_lock.lock().await;
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
    let _server_lock = state.server_mutation_lock.lock().await;
    authorized(&state, &identity, &id).await?;
    guardian::process::validate_command(body.command.trim())?;
    state
        .guardian(&id)
        .await?
        .command(body.command.trim())
        .await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

/// Force a re-resolve and download, replacing the installed artifact.
async fn reinstall(
    State(state): State<Arc<AppState>>,
    identity: Identity,
    Path(id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let _server_lock = state.server_mutation_lock.lock().await;
    authorized(&state, &identity, &id).await?;

    let guardian = state.guardian(&id).await?;
    let installed = guardian.reinstall().await?;

    tracing::info!(server = %id, jar = %installed.jar.display(), "reinstalled");
    Ok(Json(serde_json::json!({ "ok": true })))
}

/// Pre-download Java and the server jar without starting, with progress events.
///
/// Uses `Provision::IfNeeded` so a second call is a no-op when already installed.
/// Moves through `Preparing` -> `Offline` so the UI can show the same progress
/// bar as a first start.
async fn prepare_install(
    State(state): State<Arc<AppState>>,
    identity: Identity,
    Path(id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let _server_lock = state.server_mutation_lock.lock().await;
    authorized(&state, &identity, &id).await?;

    let guardian = state.guardian(&id).await?;
    let installed = guardian.prefetch().await?;

    tracing::info!(server = %id, jar = %installed.jar.display(), "prepared");
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
        .route("/{id}/reinstall", post(reinstall))
        .route("/{id}/prepare", post(prepare_install))
        .route("/{id}/logs", get(logs))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::PlayitMode;
    use std::time::Duration;

    fn operator() -> Identity {
        Identity {
            username: "operator".into(),
            admin: true,
            servers: Vec::new(),
        }
    }

    async fn power_cancels_preparation(action: PowerAction) {
        let data_dir = tempfile::tempdir().unwrap();
        let state = crate::state::AppState::bootstrap(data_dir.path(), PlayitMode::External)
            .await
            .unwrap();
        let record = ServerRecord {
            id: "server-1".into(),
            name: "Server".into(),
            config: ServerConfig::paper(state.server_dir("server-1"), "1.21.8"),
            policy: GuardianConfig::default(),
            playit: None,
            created_at: "2026-08-29T00:00:00Z".into(),
            backup_policy: None,
        };
        state
            .store
            .update(|data| data.servers.push(record.clone()))
            .await
            .unwrap();
        let guardian = state.insert_guardian(&record).await;
        let resource_lock = guardian.lock_resources().await;

        let started = power(
            State(Arc::clone(&state)),
            operator(),
            Path(record.id.clone()),
            Json(PowerRequest {
                action: PowerAction::Start,
            }),
        )
        .await
        .unwrap();
        assert_eq!(started.0.status, guardian::ServerStatus::Preparing);

        let stopped = tokio::time::timeout(
            Duration::from_secs(1),
            power(
                State(Arc::clone(&state)),
                operator(),
                Path(record.id.clone()),
                Json(PowerRequest { action }),
            ),
        )
        .await
        .expect("power cancellation must not wait for the resource lock")
        .unwrap();
        assert_eq!(stopped.0.status, guardian::ServerStatus::Offline);

        drop(resource_lock);
        assert!(guardian.wait_for_settled(Duration::from_secs(1)).await);
        state.playit.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn api_stop_can_cancel_preparation_behind_a_resource_lock() {
        power_cancels_preparation(PowerAction::Stop).await;
    }

    #[tokio::test]
    async fn api_kill_can_cancel_preparation_behind_a_resource_lock() {
        power_cancels_preparation(PowerAction::Kill).await;
    }

    fn record(id: &str, port: u16, max_mb: u32) -> ServerRecord {
        let mut config = ServerConfig::paper(std::path::PathBuf::from(id), "1.21.8");
        config.port = port;
        config.memory.max_mb = max_mb;
        config.memory.min_mb = max_mb.min(512);
        ServerRecord {
            id: id.into(),
            name: id.into(),
            config,
            policy: GuardianConfig::default(),
            playit: None,
            created_at: "2026-08-29T00:00:00Z".into(),
            backup_policy: None,
        }
    }

    #[test]
    fn a_running_server_keeps_reserving_the_port_it_was_launched_with() {
        let servers = vec![record("a", 25566, 1024)];
        // "a" was started on 25565 and re-pointed at 25566 while running.
        let mut active = ActiveConfigs::new();
        active.insert("a".into(), record("a", 25565, 1024).config);

        assert!(port_is_taken(&servers, &active, 25565, None));
        assert!(port_is_taken(&servers, &active, 25566, None));
        // The server that holds both ports is not in conflict with itself.
        assert!(!port_is_taken(&servers, &active, 25565, Some("a")));
        assert!(!port_is_taken(&servers, &active, 25567, None));
    }

    #[test]
    fn lowering_a_running_heap_does_not_free_aggregate_memory() {
        let servers = vec![record("a", 25565, 8192)];
        let mut active = ActiveConfigs::new();
        active.insert("a".into(), record("a", 25565, 8192).config);

        // Shrinking the stored heap to 1 GiB must not let another 7 GiB server
        // fit an 8 GiB budget while the old JVM can still reach 8 GiB.
        validate_aggregate_memory(&servers, &active, Some(("a", 1024)), None, 8192).unwrap();
        let error =
            validate_aggregate_memory(&servers, &active, Some(("a", 1024)), Some(7168), 8192)
                .unwrap_err();
        assert!(matches!(error, ApiError::Conflict(_)));

        // With nothing running, the reduced configuration frees the budget.
        let idle = ActiveConfigs::new();
        validate_aggregate_memory(&servers, &idle, Some(("a", 1024)), Some(7168), 8192).unwrap();
    }

    #[test]
    fn raising_a_stopped_servers_heap_still_counts_against_the_budget() {
        let servers = vec![record("a", 25565, 1024), record("b", 25566, 1024)];
        let active = ActiveConfigs::new();
        let error = validate_aggregate_memory(&servers, &active, Some(("a", 8192)), None, 4096)
            .unwrap_err();
        assert!(matches!(error, ApiError::Conflict(_)));
    }

    #[test]
    fn a_launch_config_that_matches_the_record_is_not_pending_restart() {
        let stored = record("a", 25565, 2048);
        assert!(!differs_at_launch(&stored.config, &stored.config));

        let mut edited = stored.config.clone();
        edited.memory.max_mb = 4096;
        assert!(differs_at_launch(&stored.config, &edited));

        let mut moved = stored.config.clone();
        moved.port = 25566;
        assert!(differs_at_launch(&stored.config, &moved));
    }
}
