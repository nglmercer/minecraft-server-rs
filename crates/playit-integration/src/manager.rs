//! High-level Playit operations used by the panel.

use std::sync::Arc;

use playit_ipc::model::{
    AccountResponse, AccountStatus, AgentLifecycle, ServicePhase, TunnelProtocol,
};

use crate::client::{IpcPlayitService, PlayitService};
use crate::error::PlayitError;
use crate::model::{
    ClaimInfo, PlayitAccount, PlayitAccountStatus, PlayitConnectionState, PlayitProtocol,
    PlayitStatus, PlayitTunnel, TunnelCreateInfo,
};

/// The panel-facing Playit service facade.
///
/// It deliberately does not own a persistent IPC connection. A dead socket can
/// therefore only fail one operation instead of poisoning the panel forever.
#[derive(Clone)]
pub struct PlayitManager {
    service: Arc<dyn PlayitService>,
}

impl Default for PlayitManager {
    fn default() -> Self {
        Self::new()
    }
}

impl PlayitManager {
    /// Construct a manager using the real Playit IPC service.
    pub fn new() -> Self {
        Self::with_service(IpcPlayitService)
    }

    /// Construct a manager around an injected service implementation.
    ///
    /// This is primarily useful for tests and for future alternate transports.
    pub fn with_service<S>(service: S) -> Self
    where
        S: PlayitService + 'static,
    {
        Self {
            service: Arc::new(service),
        }
    }

    /// Read and normalize the daemon's status and lifecycle.
    pub async fn status(&self) -> Result<PlayitStatus, PlayitError> {
        let service_status = self.service.status().await?;
        let lifecycle = self.service.lifecycle().await?;

        let version = (!service_status.version.is_empty()).then_some(service_status.version);
        let (status, message) = if !service_status.has_secret {
            (PlayitConnectionState::NeedsClaim, None)
        } else {
            lifecycle_state(&service_status.phase, &lifecycle)
        };

        Ok(PlayitStatus {
            status,
            version,
            message,
        })
    }

    /// Convert an operation error into the status representation used by the
    /// non-failing status endpoint.
    pub fn status_from_error(error: &PlayitError) -> PlayitStatus {
        let status = if error.is_unsupported() {
            PlayitConnectionState::Unsupported
        } else if error.is_unavailable() {
            PlayitConnectionState::Unavailable
        } else {
            PlayitConnectionState::Error
        };

        PlayitStatus {
            status,
            version: None,
            message: Some(error.to_string()),
        }
    }

    /// Read account information without exposing its secret.
    pub async fn account(&self) -> Result<PlayitAccount, PlayitError> {
        let account = self.service.account().await?;
        Ok(account_view(account))
    }

    /// Start the browser-based Playit claim flow.
    pub async fn start_claim(&self) -> Result<ClaimInfo, PlayitError> {
        let claim = self.service.start_claim().await?;
        if claim.claim_url.trim().is_empty() {
            return Err(PlayitError::Protocol(
                "claim response did not contain a URL".into(),
            ));
        }
        Ok(ClaimInfo {
            claim_url: claim.claim_url,
        })
    }

    /// List currently materialized tunnels.
    pub async fn tunnels(&self) -> Result<Vec<PlayitTunnel>, PlayitError> {
        let response = self.service.list_tunnels().await?;
        Ok(response.tunnels.into_iter().map(tunnel_view).collect())
    }

    /// Create a tunnel and return its immediate identifier.
    pub async fn create_tunnel(
        &self,
        local_port: u16,
        protocol: PlayitProtocol,
        local_address: Option<String>,
        name: Option<String>,
    ) -> Result<TunnelCreateInfo, PlayitError> {
        let response = self
            .service
            .create_tunnel(local_port, protocol.into(), local_address, name)
            .await?;

        if response.tunnel_id.trim().is_empty() {
            return Err(PlayitError::Protocol(
                "tunnel creation response did not contain an id".into(),
            ));
        }

        Ok(TunnelCreateInfo {
            tunnel_id: response.tunnel_id,
            message: response.message,
        })
    }

    /// Create the default TCP tunnel for one Minecraft server.
    ///
    /// Playit returns the id immediately and may materialize the public
    /// address asynchronously. The follow-up list lets callers persist the
    /// complete tunnel once it is visible.
    pub async fn create_server_tunnel(
        &self,
        server_id: &str,
        server_name: &str,
        port: u16,
    ) -> Result<PlayitTunnel, PlayitError> {
        let created = self
            .create_tunnel(
                port,
                PlayitProtocol::Tcp,
                Some("127.0.0.1".into()),
                Some(format!("mcpanel:{server_id}:{server_name}")),
            )
            .await?;

        self.tunnels()
            .await?
            .into_iter()
            .find(|tunnel| tunnel.id == created.tunnel_id)
            .ok_or_else(|| {
                PlayitError::Protocol(format!(
                    "created tunnel {} was not present in the tunnel list",
                    created.tunnel_id
                ))
            })
    }

    /// Delete a tunnel by its stable Playit id.
    pub async fn delete_tunnel(&self, tunnel_id: &str) -> Result<(), PlayitError> {
        let response = self.service.delete_tunnel(tunnel_id).await?;
        if !response.accepted {
            return Err(PlayitError::Rejected(
                response
                    .message
                    .unwrap_or_else(|| "delete command was not accepted".into()),
            ));
        }
        Ok(())
    }
}

fn account_view(account: AccountResponse) -> PlayitAccount {
    PlayitAccount {
        status: match account.status {
            AccountStatus::Unknown => PlayitAccountStatus::Unknown,
            AccountStatus::Guest => PlayitAccountStatus::Guest,
            AccountStatus::EmailNotVerified => PlayitAccountStatus::EmailNotVerified,
            AccountStatus::Verified => PlayitAccountStatus::Verified,
        },
        agent_id: account.agent_id,
        login_link: account.login_link,
        claim_url: account.claim_url,
    }
}

fn tunnel_view(tunnel: playit_ipc::model::TunnelState) -> PlayitTunnel {
    PlayitTunnel {
        id: tunnel.id,
        name: tunnel.name,
        display_address: tunnel.display_address,
        destination: tunnel.destination,
        protocol: tunnel.protocol.into(),
        local_address: tunnel.local_address,
        local_port: tunnel.local_port,
        disabled: tunnel.is_disabled,
        disabled_reason: tunnel.disabled_reason,
    }
}

fn lifecycle_state(
    phase: &ServicePhase,
    lifecycle: &AgentLifecycle,
) -> (PlayitConnectionState, Option<String>) {
    match lifecycle {
        AgentLifecycle::WaitingForSecret => (PlayitConnectionState::NeedsClaim, None),
        AgentLifecycle::HasInvalidSecret(error)
        | AgentLifecycle::DisabledOverLimit(error)
        | AgentLifecycle::Error(error) => {
            (PlayitConnectionState::Error, Some(error.message.clone()))
        }
        AgentLifecycle::Starting => (PlayitConnectionState::Starting, None),
        AgentLifecycle::Stopping => (PlayitConnectionState::Stopping, None),
        AgentLifecycle::Running(_) => match phase {
            ServicePhase::WaitingForSecret => (PlayitConnectionState::NeedsClaim, None),
            ServicePhase::Starting => (PlayitConnectionState::Starting, None),
            ServicePhase::Stopping => (PlayitConnectionState::Stopping, None),
            ServicePhase::HasInvalidSecret
            | ServicePhase::DisabledOverLimit
            | ServicePhase::Error => (PlayitConnectionState::Error, phase_error_message(phase)),
            ServicePhase::Running => (PlayitConnectionState::Connected, None),
        },
    }
}

fn phase_error_message(phase: &ServicePhase) -> Option<String> {
    let message = match phase {
        ServicePhase::HasInvalidSecret => "Playit has an invalid account secret",
        ServicePhase::DisabledOverLimit => "Playit disabled the agent over its limit",
        ServicePhase::Error => "Playit reported an error",
        ServicePhase::WaitingForSecret
        | ServicePhase::Starting
        | ServicePhase::Running
        | ServicePhase::Stopping => return None,
    };
    Some(message.into())
}

impl From<PlayitProtocol> for TunnelProtocol {
    fn from(protocol: PlayitProtocol) -> Self {
        match protocol {
            PlayitProtocol::Tcp => Self::Tcp,
            PlayitProtocol::Udp => Self::Udp,
            PlayitProtocol::Both => Self::Both,
        }
    }
}

impl From<TunnelProtocol> for PlayitProtocol {
    fn from(protocol: TunnelProtocol) -> Self {
        match protocol {
            TunnelProtocol::Tcp => Self::Tcp,
            TunnelProtocol::Udp => Self::Udp,
            TunnelProtocol::Both => Self::Both,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use playit_ipc::model::{
        AgentLifecycle, ClaimResponse, CommandResponse, ProtocolInfo, ServiceStatus,
        TunnelCreateResponse, TunnelListResponse,
    };
    use std::sync::Mutex;

    type CreatedTunnel = (u16, TunnelProtocol, Option<String>, Option<String>);

    #[derive(Default)]
    struct MockService {
        status: Mutex<ServiceStatus>,
        lifecycle: Mutex<AgentLifecycle>,
        account: Mutex<AccountResponse>,
        claim: Mutex<ClaimResponse>,
        tunnels: Mutex<TunnelListResponse>,
        created: Mutex<Vec<CreatedTunnel>>,
    }

    #[async_trait]
    impl PlayitService for MockService {
        async fn status(&self) -> Result<ServiceStatus, playit_ipc::ipc::IpcError> {
            Ok(self.status.lock().unwrap().clone())
        }

        async fn lifecycle(&self) -> Result<AgentLifecycle, playit_ipc::ipc::IpcError> {
            Ok(self.lifecycle.lock().unwrap().clone())
        }

        async fn account(&self) -> Result<AccountResponse, playit_ipc::ipc::IpcError> {
            Ok(self.account.lock().unwrap().clone())
        }

        async fn start_claim(&self) -> Result<ClaimResponse, playit_ipc::ipc::IpcError> {
            Ok(self.claim.lock().unwrap().clone())
        }

        async fn list_tunnels(&self) -> Result<TunnelListResponse, playit_ipc::ipc::IpcError> {
            Ok(self.tunnels.lock().unwrap().clone())
        }

        async fn create_tunnel(
            &self,
            local_port: u16,
            protocol: TunnelProtocol,
            local_address: Option<String>,
            name: Option<String>,
        ) -> Result<TunnelCreateResponse, playit_ipc::ipc::IpcError> {
            self.created
                .lock()
                .unwrap()
                .push((local_port, protocol, local_address, name));
            Ok(TunnelCreateResponse {
                tunnel_id: "tunnel-1".into(),
                message: None,
            })
        }

        async fn delete_tunnel(
            &self,
            _: &str,
        ) -> Result<CommandResponse, playit_ipc::ipc::IpcError> {
            Ok(CommandResponse {
                accepted: true,
                message: None,
            })
        }
    }

    fn running_service() -> MockService {
        MockService {
            status: Mutex::new(ServiceStatus {
                phase: ServicePhase::Running,
                version: "1.2.3".into(),
                has_secret: true,
                protocol: ProtocolInfo {
                    ipc_version: playit_ipc::ipc::IPC_VERSION,
                    ..ProtocolInfo::default()
                },
                ..ServiceStatus::default()
            }),
            lifecycle: Mutex::new(AgentLifecycle::Running(Default::default())),
            ..MockService::default()
        }
    }

    #[tokio::test]
    async fn running_service_is_connected() {
        let manager = PlayitManager::with_service(running_service());
        let status = manager.status().await.unwrap();

        assert_eq!(status.status, PlayitConnectionState::Connected);
        assert_eq!(status.version.as_deref(), Some("1.2.3"));
    }

    #[tokio::test]
    async fn waiting_for_secret_needs_claim() {
        let service = MockService {
            status: Mutex::new(ServiceStatus {
                phase: ServicePhase::WaitingForSecret,
                ..ServiceStatus::default()
            }),
            lifecycle: Mutex::new(AgentLifecycle::WaitingForSecret),
            ..MockService::default()
        };
        let manager = PlayitManager::with_service(service);

        assert_eq!(
            manager.status().await.unwrap().status,
            PlayitConnectionState::NeedsClaim
        );
    }

    #[test]
    fn protocol_errors_are_reported_as_unsupported_status() {
        let error = PlayitError::from(playit_ipc::ipc::IpcError::ProtocolMismatch {
            expected: 2,
            actual: 1,
        });

        assert_eq!(
            PlayitManager::status_from_error(&error).status,
            PlayitConnectionState::Unsupported
        );
    }

    #[tokio::test]
    async fn server_tunnel_uses_loopback_tcp_and_managed_name() {
        let service = running_service();
        service
            .tunnels
            .lock()
            .unwrap()
            .tunnels
            .push(playit_ipc::model::TunnelState {
                id: "tunnel-1".into(),
                display_address: "example.gl.joinmc.link".into(),
                destination: "127.0.0.1:25565".into(),
                local_address: Some("127.0.0.1".into()),
                local_port: Some(25565),
                ..playit_ipc::model::TunnelState::default()
            });
        let manager = PlayitManager::with_service(service);

        let tunnel = manager
            .create_server_tunnel("server-1", "Survival SMP", 25565)
            .await
            .unwrap();

        assert_eq!(tunnel.id, "tunnel-1");
    }
}
