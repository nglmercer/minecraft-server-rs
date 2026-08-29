//! Playit account, claim, and tunnel endpoints.

use axum::extract::{Path, State};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use playit_integration::{
    ClaimInfo, PlayitAccount, PlayitConnectionState, PlayitProtocol, PlayitStatus, PlayitTunnel,
    TunnelCreateInfo,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::auth::{AdminIdentity, Identity};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use crate::store::{PlayitBinding, ServerRecord};

/// The panel's view of a server's Playit tunnel association.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ServerPlayitState {
    /// No tunnel is configured for this server.
    Disabled,
    /// The id is stored, but Playit has not materialized it in the list yet.
    Provisioning,
    /// The Playit service reports a usable tunnel with the expected destination.
    Connected,
    /// Playit knows the tunnel but has disabled it.
    DisabledByPlayit,
    /// Playit reports a destination different from the panel binding.
    Drifted,
    /// The Playit service could not be queried.
    Unavailable,
}

/// A safe server-scoped Playit response.
#[derive(Debug, Serialize)]
pub struct ServerPlayitView {
    /// The state the client should display.
    pub state: ServerPlayitState,
    /// The panel's persisted association, if one exists.
    pub binding: Option<PlayitBinding>,
    /// The matching live Playit tunnel, if it is currently visible.
    pub tunnel: Option<PlayitTunnel>,
    /// A diagnostic or provisioning note, when useful.
    pub message: Option<String>,
}

/// GET `/api/playit/status`.
///
/// A missing or stopped Playit service is a normal deployment state, so this endpoint
/// returns a usable status document instead of preventing the panel from
/// starting or turning the status check into a generic HTTP 500.
async fn status(State(state): State<Arc<AppState>>, _: Identity) -> Json<PlayitStatus> {
    let status = match state.playit.status().await {
        Ok(status) => status,
        Err(error) => {
            tracing::warn!(error = ?error, "Playit status unavailable");
            safe_status(playit_integration::PlayitManager::status_from_error(&error))
        }
    };
    Json(safe_status(status))
}

/// Convert a detailed integration failure into a status message that is safe
/// to expose over HTTP. The full error is logged above for operators, but it
/// may contain local paths, IPC details, or secret-file names.
fn safe_status(mut status: PlayitStatus) -> PlayitStatus {
    status.message = match status.status {
        PlayitConnectionState::Unavailable => Some("Playit service unavailable".into()),
        PlayitConnectionState::Unsupported => Some("Playit service protocol unsupported".into()),
        PlayitConnectionState::Error => Some("Playit service error".into()),
        _ => None,
    };
    status
}

/// GET `/api/playit/account`.
async fn account(
    State(state): State<Arc<AppState>>,
    AdminIdentity(_admin): AdminIdentity,
) -> ApiResult<Json<PlayitAccount>> {
    Ok(Json(state.playit.account().await?))
}

/// POST `/api/playit/claim`.
async fn claim(
    State(state): State<Arc<AppState>>,
    AdminIdentity(_admin): AdminIdentity,
) -> ApiResult<Json<ClaimInfo>> {
    Ok(Json(state.playit.start_claim().await?))
}

#[derive(Debug, Deserialize)]
struct CreateTunnelRequest {
    local_port: u16,
    #[serde(default)]
    protocol: PlayitProtocol,
    #[serde(default)]
    local_address: Option<String>,
    #[serde(default)]
    name: Option<String>,
}

/// GET `/api/playit/tunnels`.
///
/// The account-level API is used here so tunnels assigned to another Playit
/// agent are visible and can be identified before an operator creates another
/// one.
async fn list_tunnels(
    State(state): State<Arc<AppState>>,
    AdminIdentity(_admin): AdminIdentity,
) -> ApiResult<Json<Vec<PlayitTunnel>>> {
    Ok(Json(state.playit.account_tunnels().await?))
}

/// POST `/api/playit/tunnels`.
async fn create_tunnel(
    State(state): State<Arc<AppState>>,
    AdminIdentity(_admin): AdminIdentity,
    Json(body): Json<CreateTunnelRequest>,
) -> ApiResult<Json<TunnelCreateInfo>> {
    let local_address = local_address(body.local_address)?;
    let name = tunnel_name(body.name)?;

    if body.local_port == 0 {
        return Err(ApiError::BadRequest(
            "local_port must be between 1 and 65535".into(),
        ));
    }

    Ok(Json(
        state
            .playit
            .create_tunnel(body.local_port, body.protocol, Some(local_address), name)
            .await?,
    ))
}

/// DELETE `/api/playit/tunnels/:id`.
async fn delete_tunnel(
    State(state): State<Arc<AppState>>,
    AdminIdentity(_admin): AdminIdentity,
    Path(tunnel_id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let _server_lock = state.server_mutation_lock.lock().await;
    state.playit.delete_tunnel(&tunnel_id).await?;

    // A direct admin delete must not leave a stale panel association behind.
    state
        .store
        .update(|data| {
            for server in &mut data.servers {
                if server
                    .playit
                    .as_ref()
                    .map(|binding| binding.tunnel_id == tunnel_id)
                    .unwrap_or(false)
                {
                    server.playit = None;
                }
            }
        })
        .await?;

    Ok(Json(serde_json::json!({ "ok": true })))
}

#[derive(Debug, Default, Deserialize)]
struct AttachServerTunnelRequest {
    /// An optional operator-facing override. The default is stable and
    /// includes the server id so renaming a server does not break matching.
    #[serde(default)]
    name: Option<String>,
}

/// GET `/api/servers/:id/playit`.
async fn server_playit(
    State(state): State<Arc<AppState>>,
    identity: Identity,
    Path(id): Path<String>,
) -> ApiResult<Json<ServerPlayitView>> {
    let record = authorized_server(&state, &identity, &id).await?;
    Ok(Json(server_playit_view(&state, &record).await))
}

/// POST `/api/servers/:id/playit`.
///
/// Server tunnels deliberately use loopback and the server's configured port;
/// arbitrary destinations belong to the admin-only global tunnel endpoint.
async fn attach_server_playit(
    State(state): State<Arc<AppState>>,
    AdminIdentity(admin): AdminIdentity,
    Path(id): Path<String>,
    Json(body): Json<AttachServerTunnelRequest>,
) -> ApiResult<Json<ServerPlayitView>> {
    let _server_lock = state.server_mutation_lock.lock().await;
    let mut record = authorized_server(&state, &admin, &id).await?;

    if record.playit.is_some() {
        return Err(ApiError::Conflict(
            "this server already has a Playit tunnel".into(),
        ));
    }

    let created = match tunnel_name(body.name)? {
        None => {
            state
                .playit
                .ensure_server_tunnel(&id, &record.name, record.config.port)
                .await?
        }
        Some(name) => {
            state
                .playit
                .create_minecraft_java_tunnel(
                    record.config.port,
                    Some("127.0.0.1".into()),
                    Some(name),
                )
                .await?
        }
    };

    let binding = PlayitBinding {
        tunnel_id: created.tunnel_id,
        protocol: playit_integration::PlayitProtocol::Tcp,
        local_address: "127.0.0.1".into(),
        local_port: record.config.port,
    };

    let binding_for_store = binding.clone();
    let write = state
        .store
        .try_update(move |data| -> ApiResult<()> {
            let Some(server) = data.servers.iter_mut().find(|server| server.id == id) else {
                return Err(ApiError::NotFound("server".into()));
            };
            if server.playit.is_some() {
                return Err(ApiError::Conflict(
                    "this server already has a Playit tunnel".into(),
                ));
            }
            if server.config.port != binding_for_store.local_port {
                return Err(ApiError::Conflict(
                    "the server port changed while the Playit tunnel was being created; retry"
                        .into(),
                ));
            }
            server.playit = Some(binding_for_store);
            Ok(())
        })
        .await;

    match write {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            let _ = state.playit.delete_tunnel(&binding.tunnel_id).await;
            return Err(error);
        }
        Err(error) => {
            let _ = state.playit.delete_tunnel(&binding.tunnel_id).await;
            return Err(ApiError::Internal(error));
        }
    }

    record.playit = Some(binding);
    Ok(Json(server_playit_view(&state, &record).await))
}

/// DELETE `/api/servers/:id/playit`.
async fn detach_server_playit(
    State(state): State<Arc<AppState>>,
    AdminIdentity(admin): AdminIdentity,
    Path(id): Path<String>,
) -> ApiResult<Json<ServerPlayitView>> {
    let _server_lock = state.server_mutation_lock.lock().await;
    let mut record = authorized_server(&state, &admin, &id).await?;

    let Some(binding) = record.playit.clone() else {
        return Ok(Json(server_playit_view(&state, &record).await));
    };

    // Listing first makes detaching idempotent when an operator removed the
    // tunnel directly in the Playit client.
    let tunnels = state.playit.account_tunnels().await?;
    if tunnels.iter().any(|tunnel| tunnel.id == binding.tunnel_id) {
        state.playit.delete_tunnel(&binding.tunnel_id).await?;
    }

    let cleared = state
        .store
        .try_update(|data| -> ApiResult<bool> {
            let Some(server) = data.servers.iter_mut().find(|server| server.id == id) else {
                return Err(ApiError::NotFound("server".into()));
            };
            if server
                .playit
                .as_ref()
                .is_some_and(|current| current.tunnel_id == binding.tunnel_id)
            {
                server.playit = None;
                Ok(true)
            } else {
                Ok(false)
            }
        })
        .await??;

    if !cleared {
        return Err(ApiError::Conflict(
            "the server's Playit tunnel changed while it was being detached; retry".into(),
        ));
    }

    record.playit = None;
    Ok(Json(server_playit_view(&state, &record).await))
}

async fn authorized_server(
    state: &AppState,
    identity: &Identity,
    id: &str,
) -> ApiResult<ServerRecord> {
    if !identity.may_access(id) {
        return Err(ApiError::NotFound("server".into()));
    }
    state
        .store
        .server(id)
        .await
        .ok_or_else(|| ApiError::NotFound(format!("server {id}")))
}

async fn server_playit_view(state: &AppState, record: &ServerRecord) -> ServerPlayitView {
    let Some(binding) = record.playit.clone() else {
        return ServerPlayitView {
            state: ServerPlayitState::Disabled,
            binding: None,
            tunnel: None,
            message: None,
        };
    };

    let tunnels = match state.playit.account_tunnels().await {
        Ok(tunnels) => tunnels,
        Err(error) => {
            tracing::warn!(error = ?error, "Playit status unavailable for server view");
            return ServerPlayitView {
                state: ServerPlayitState::Unavailable,
                binding: Some(binding),
                tunnel: None,
                message: Some("Playit service unavailable".into()),
            };
        }
    };

    let Some(tunnel) = tunnels
        .into_iter()
        .find(|tunnel| tunnel.id == binding.tunnel_id)
    else {
        return ServerPlayitView {
            state: ServerPlayitState::Provisioning,
            binding: Some(binding),
            tunnel: None,
            message: Some(
                "Playit accepted the tunnel, but it is not visible in the service yet".into(),
            ),
        };
    };

    let drifted = tunnel
        .local_address
        .as_ref()
        .map(|address| address != &binding.local_address)
        .unwrap_or(false)
        || tunnel
            .local_port
            .map(|port| port != binding.local_port)
            .unwrap_or(false);

    let state = if tunnel.disabled {
        ServerPlayitState::DisabledByPlayit
    } else if drifted {
        ServerPlayitState::Drifted
    } else {
        ServerPlayitState::Connected
    };
    let message = match state {
        ServerPlayitState::DisabledByPlayit => tunnel.disabled_reason.clone(),
        ServerPlayitState::Drifted => {
            Some("the Playit destination differs from the server port".into())
        }
        _ => None,
    };

    ServerPlayitView {
        state,
        binding: Some(binding),
        tunnel: Some(tunnel),
        message,
    }
}

fn local_address(address: Option<String>) -> ApiResult<String> {
    let address = address.unwrap_or_else(|| "127.0.0.1".into());
    let address = address.trim();
    if address != "127.0.0.1" && address != "::1" {
        return Err(ApiError::BadRequest(
            "local_address must be 127.0.0.1 or ::1".into(),
        ));
    }
    Ok(address.into())
}

fn tunnel_name(name: Option<String>) -> ApiResult<Option<String>> {
    let Some(name) = name else {
        return Ok(None);
    };
    let name = name.trim();
    if name.is_empty() {
        return Ok(None);
    }
    if name.chars().count() > 100 {
        return Err(ApiError::BadRequest("tunnel name is too long".into()));
    }
    Ok(Some(name.into()))
}

/// Routes under `/api`.
pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/playit/status", get(status))
        .route("/playit/account", get(account))
        .route("/playit/claim", post(claim))
        .route("/playit/tunnels", get(list_tunnels).post(create_tunnel))
        .route("/playit/tunnels/{tunnel_id}", delete(delete_tunnel))
        .route(
            "/servers/{id}/playit",
            get(server_playit)
                .post(attach_server_playit)
                .delete(detach_server_playit),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_tunnels_are_restricted_to_loopback() {
        assert_eq!(local_address(None).unwrap(), "127.0.0.1");
        assert_eq!(local_address(Some(" ::1 ".into())).unwrap(), "::1");
        assert!(local_address(Some("192.168.1.10".into())).is_err());
    }

    #[test]
    fn blank_names_are_omitted_and_long_names_are_rejected() {
        assert_eq!(tunnel_name(Some("  ".into())).unwrap(), None);
        assert_eq!(
            tunnel_name(Some(" survival ".into())).unwrap(),
            Some("survival".into())
        );
        assert!(tunnel_name(Some("x".repeat(101))).is_err());
    }

    #[test]
    fn status_diagnostics_do_not_contain_integration_error_details() {
        let status = safe_status(PlayitStatus {
            status: PlayitConnectionState::Unavailable,
            version: None,
            message: Some("C:\\private\\playit\\secret.toml leaked".into()),
        });

        assert_eq!(
            status.message.as_deref(),
            Some("Playit service unavailable")
        );
        assert!(!status.message.as_deref().unwrap().contains("secret.toml"));
    }
}
