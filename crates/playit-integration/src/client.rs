//! The mockable boundary between the manager and Playit's IPC client.

use async_trait::async_trait;
use playit_ipc::ipc::{IpcClient, IpcError};
use playit_ipc::model::{
    AccountResponse, AgentLifecycle, ClaimResponse, CommandResponse, ServiceStatus,
    TunnelCreateResponse, TunnelListResponse, TunnelProtocol,
};

/// Operations needed from a Playit daemon.
///
/// Keeping this boundary above [`IpcClient`] makes manager tests deterministic
/// and keeps the panel independent of a real Playit account.
#[async_trait]
pub trait PlayitService: Send + Sync {
    /// Read the daemon service status.
    async fn status(&self) -> Result<ServiceStatus, IpcError>;
    /// Read the daemon lifecycle state.
    async fn lifecycle(&self) -> Result<AgentLifecycle, IpcError>;
    /// Read the configured account summary.
    async fn account(&self) -> Result<AccountResponse, IpcError>;
    /// Start the daemon's browser-based claim flow.
    async fn start_claim(&self) -> Result<ClaimResponse, IpcError>;
    /// List existing tunnels.
    async fn list_tunnels(&self) -> Result<TunnelListResponse, IpcError>;
    /// Create a tunnel.
    async fn create_tunnel(
        &self,
        local_port: u16,
        protocol: TunnelProtocol,
        local_address: Option<String>,
        name: Option<String>,
    ) -> Result<TunnelCreateResponse, IpcError>;
    /// Delete a tunnel by its stable identifier.
    async fn delete_tunnel(&self, tunnel_id: &str) -> Result<CommandResponse, IpcError>;
}

/// The production implementation backed by a fresh Playit IPC connection per
/// operation.
#[derive(Debug, Clone, Copy, Default)]
pub struct IpcPlayitService;

#[async_trait]
impl PlayitService for IpcPlayitService {
    async fn status(&self) -> Result<ServiceStatus, IpcError> {
        let mut client = IpcClient::connect().await?;
        client.status().await
    }

    async fn lifecycle(&self) -> Result<AgentLifecycle, IpcError> {
        let mut client = IpcClient::connect().await?;
        client.lifecycle().await
    }

    async fn account(&self) -> Result<AccountResponse, IpcError> {
        let mut client = IpcClient::connect().await?;
        client.account().await
    }

    async fn start_claim(&self) -> Result<ClaimResponse, IpcError> {
        let mut client = IpcClient::connect().await?;
        client.start_claim().await
    }

    async fn list_tunnels(&self) -> Result<TunnelListResponse, IpcError> {
        let mut client = IpcClient::connect().await?;
        client.list_tunnels().await
    }

    async fn create_tunnel(
        &self,
        local_port: u16,
        protocol: TunnelProtocol,
        local_address: Option<String>,
        name: Option<String>,
    ) -> Result<TunnelCreateResponse, IpcError> {
        let mut client = IpcClient::connect().await?;
        client
            .create_tunnel(local_port, protocol, local_address, name)
            .await
    }

    async fn delete_tunnel(&self, tunnel_id: &str) -> Result<CommandResponse, IpcError> {
        let mut client = IpcClient::connect().await?;
        client.delete_tunnel(tunnel_id).await
    }
}
