//! High-level Playit operations used by the panel.

use std::path::PathBuf;
use std::sync::Arc;

use playit_ipc::model::{
    AccountResponse, AccountStatus, AgentLifecycle, ServicePhase, TunnelProtocol,
};
use playit_runtime::{PlayitRuntime, RuntimeOptions};
use tokio::sync::Mutex;

use crate::client::{IpcPlayitService, PlayitService};
use crate::error::PlayitError;
use crate::model::{
    ClaimInfo, PlayitAccount, PlayitAccountStatus, PlayitConnectionState, PlayitProtocol,
    PlayitStatus, PlayitTunnel, TunnelCreateInfo,
};

/// The panel-facing Playit service facade.
///
/// External mode deliberately does not own a persistent IPC connection. A dead
/// socket can therefore only fail one operation instead of poisoning the panel
/// forever. Embedded mode owns one runtime shared by all manager clones.
#[derive(Clone)]
pub struct PlayitManager {
    service: Arc<dyn PlayitService>,
    runtime: Option<Arc<Mutex<Option<PlayitRuntime>>>>,
}

impl Default for PlayitManager {
    fn default() -> Self {
        Self::new()
    }
}

impl PlayitManager {
    /// Construct a manager using the optional external Playit IPC service.
    ///
    /// This compatibility alias preserves the original integration API. New
    /// panel startup code should choose [`Self::embedded`] or [`Self::external`]
    /// explicitly.
    pub fn new() -> Self {
        Self::external()
    }

    /// Construct a manager using a direct, in-process Playit runtime.
    pub async fn embedded(secret_path: impl Into<PathBuf>) -> Result<Self, PlayitError> {
        let options = RuntimeOptions {
            secret_path: secret_path.into(),
            ..RuntimeOptions::default()
        };
        let (runtime, handle) = PlayitRuntime::start(options).await?;

        Ok(Self {
            service: Arc::new(crate::embedded::EmbeddedPlayitService::new(handle)),
            runtime: Some(Arc::new(Mutex::new(Some(runtime)))),
        })
    }

    /// Construct a manager using the separately managed external daemon.
    pub fn external() -> Self {
        Self {
            service: Arc::new(IpcPlayitService),
            runtime: None,
        }
    }

    /// Construct a manager whose operations report a startup failure.
    pub fn unavailable(message: impl Into<String>) -> Self {
        Self::with_service(UnavailablePlayitService {
            message: message.into(),
        })
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
            runtime: None,
        }
    }

    /// Stop the embedded runtime owned by this manager, if any.
    ///
    /// The runtime owner is stored behind a shared mutex so clones all observe
    /// the same one-shot shutdown. External mode intentionally does nothing;
    /// its daemon belongs to the operator's service manager.
    pub async fn shutdown(&self) -> Result<(), PlayitError> {
        let Some(runtime) = &self.runtime else {
            return Ok(());
        };

        let runtime = runtime.lock().await.take();
        if let Some(runtime) = runtime {
            runtime.shutdown().await?;
        }
        Ok(())
    }

    /// Read and normalize the Playit service's status and lifecycle.
    pub async fn status(&self) -> Result<PlayitStatus, PlayitError> {
        let service_status = self.service.status().await?;
        let lifecycle = self.service.lifecycle().await?;

        let version = (!service_status.version.is_empty()).then_some(service_status.version);
        let (status, message) = match (&lifecycle, service_status.has_secret) {
            (AgentLifecycle::Running(_), false) => (PlayitConnectionState::NeedsClaim, None),
            _ => lifecycle_state(&service_status.phase, &lifecycle),
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

    /// Create a semantic Minecraft Java tunnel for a local server.
    pub async fn create_minecraft_java_tunnel(
        &self,
        local_port: u16,
        local_address: Option<String>,
        name: Option<String>,
    ) -> Result<TunnelCreateInfo, PlayitError> {
        let response = self
            .service
            .create_minecraft_java_tunnel(local_port, local_address, name)
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

    /// Create the default Minecraft Java tunnel for one Minecraft server.
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
            .create_minecraft_java_tunnel(
                port,
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

struct UnavailablePlayitService {
    message: String,
}

#[async_trait::async_trait]
impl PlayitService for UnavailablePlayitService {
    async fn status(&self) -> Result<playit_ipc::model::ServiceStatus, PlayitError> {
        Err(PlayitError::Unavailable(self.message.clone()))
    }

    async fn lifecycle(&self) -> Result<AgentLifecycle, PlayitError> {
        Err(PlayitError::Unavailable(self.message.clone()))
    }

    async fn account(&self) -> Result<AccountResponse, PlayitError> {
        Err(PlayitError::Unavailable(self.message.clone()))
    }

    async fn start_claim(&self) -> Result<playit_ipc::model::ClaimResponse, PlayitError> {
        Err(PlayitError::Unavailable(self.message.clone()))
    }

    async fn list_tunnels(&self) -> Result<playit_ipc::model::TunnelListResponse, PlayitError> {
        Err(PlayitError::Unavailable(self.message.clone()))
    }

    async fn create_tunnel(
        &self,
        _: u16,
        _: TunnelProtocol,
        _: Option<String>,
        _: Option<String>,
    ) -> Result<playit_ipc::model::TunnelCreateResponse, PlayitError> {
        Err(PlayitError::Unavailable(self.message.clone()))
    }

    async fn create_minecraft_java_tunnel(
        &self,
        _: u16,
        _: Option<String>,
        _: Option<String>,
    ) -> Result<playit_ipc::model::TunnelCreateResponse, PlayitError> {
        Err(PlayitError::Unavailable(self.message.clone()))
    }

    async fn delete_tunnel(
        &self,
        _: &str,
    ) -> Result<playit_ipc::model::CommandResponse, PlayitError> {
        Err(PlayitError::Unavailable(self.message.clone()))
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
        async fn status(&self) -> Result<ServiceStatus, PlayitError> {
            Ok(self.status.lock().unwrap().clone())
        }

        async fn lifecycle(&self) -> Result<AgentLifecycle, PlayitError> {
            Ok(self.lifecycle.lock().unwrap().clone())
        }

        async fn account(&self) -> Result<AccountResponse, PlayitError> {
            Ok(self.account.lock().unwrap().clone())
        }

        async fn start_claim(&self) -> Result<ClaimResponse, PlayitError> {
            Ok(self.claim.lock().unwrap().clone())
        }

        async fn list_tunnels(&self) -> Result<TunnelListResponse, PlayitError> {
            Ok(self.tunnels.lock().unwrap().clone())
        }

        async fn create_tunnel(
            &self,
            local_port: u16,
            protocol: TunnelProtocol,
            local_address: Option<String>,
            name: Option<String>,
        ) -> Result<TunnelCreateResponse, PlayitError> {
            self.created
                .lock()
                .unwrap()
                .push((local_port, protocol, local_address, name));
            Ok(TunnelCreateResponse {
                tunnel_id: "generic-tunnel".into(),
                message: None,
            })
        }

        async fn create_minecraft_java_tunnel(
            &self,
            local_port: u16,
            local_address: Option<String>,
            name: Option<String>,
        ) -> Result<TunnelCreateResponse, PlayitError> {
            self.created.lock().unwrap().push((
                local_port,
                TunnelProtocol::Tcp,
                local_address,
                name,
            ));
            Ok(TunnelCreateResponse {
                tunnel_id: "tunnel-1".into(),
                message: None,
            })
        }

        async fn delete_tunnel(&self, _: &str) -> Result<CommandResponse, PlayitError> {
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

    #[tokio::test]
    async fn starting_and_stopping_states_are_preserved_without_a_secret() {
        for (lifecycle, phase, expected) in [
            (
                AgentLifecycle::Starting,
                ServicePhase::Starting,
                PlayitConnectionState::Starting,
            ),
            (
                AgentLifecycle::Stopping,
                ServicePhase::Stopping,
                PlayitConnectionState::Stopping,
            ),
        ] {
            let service = MockService {
                status: Mutex::new(ServiceStatus {
                    phase,
                    has_secret: false,
                    ..ServiceStatus::default()
                }),
                lifecycle: Mutex::new(lifecycle),
                ..MockService::default()
            };
            let manager = PlayitManager::with_service(service);

            assert_eq!(manager.status().await.unwrap().status, expected);
        }
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

    #[test]
    fn runtime_stopped_is_unavailable() {
        let error = PlayitError::from(playit_runtime::RuntimeError::Stopped);

        assert!(error.is_unavailable());
        assert_eq!(
            PlayitManager::status_from_error(&error).status,
            PlayitConnectionState::Unavailable
        );
    }

    #[test]
    fn runtime_api_unavailable_is_unavailable_but_business_errors_are_not() {
        let unavailable = PlayitError::from(playit_runtime::RuntimeError::Api {
            code: playit_ipc::model::ServiceErrorCode::ApiUnavailable,
            message: "not ready".into(),
            retryable: true,
            details: None,
        });
        let rejected = PlayitError::from(playit_runtime::RuntimeError::InvalidState {
            code: playit_ipc::model::ServiceErrorCode::InvalidTunnelRequest,
            message: "bad tunnel".into(),
            retryable: false,
            details: None,
        });

        assert!(unavailable.is_unavailable());
        assert!(!rejected.is_unavailable());
    }

    #[tokio::test]
    async fn unavailable_backend_reports_the_startup_message() {
        let manager = PlayitManager::unavailable("embedded startup failed");
        let error = manager.status().await.unwrap_err();

        assert!(error.is_unavailable());
        assert!(error.to_string().contains("embedded startup failed"));
    }

    #[tokio::test]
    async fn embedded_shutdown_is_idempotent_and_shared_by_clones() {
        let secret_path = std::env::temp_dir().join(format!(
            "mcpanel-manager-shutdown-{}-{}.toml",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let manager = PlayitManager::embedded(secret_path.clone()).await.unwrap();
        let clone = manager.clone();

        clone.shutdown().await.unwrap();
        manager.shutdown().await.unwrap();

        assert!(matches!(
            manager.account().await,
            Err(PlayitError::Runtime(playit_runtime::RuntimeError::Stopped))
        ));
        let _ = tokio::fs::remove_file(secret_path).await;
    }

    #[tokio::test]
    async fn external_shutdown_is_a_no_op() {
        PlayitManager::external().shutdown().await.unwrap();
    }
}
