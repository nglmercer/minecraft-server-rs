//! The mockable boundary between the manager and a Playit backend.

use async_trait::async_trait;
use playit_ipc::ipc::IpcClient;
use playit_ipc::model::{
    AccountResponse, AccountTunnelListResponse, AgentLifecycle, ClaimResponse, CommandResponse,
    ServiceStatus, TunnelCreateResponse, TunnelListResponse, TunnelProtocol,
};

use crate::error::PlayitError;

/// Operations needed from a Playit service.
///
/// Keeping this boundary above either [`IpcClient`] or the embedded runtime
/// makes manager tests deterministic and keeps the panel independent of
/// Playit's transport details.
#[async_trait]
pub trait PlayitService: Send + Sync {
    /// Read the Playit service status.
    async fn status(&self) -> Result<ServiceStatus, PlayitError>;
    /// Read the Playit lifecycle state.
    async fn lifecycle(&self) -> Result<AgentLifecycle, PlayitError>;
    /// Read the configured account summary.
    async fn account(&self) -> Result<AccountResponse, PlayitError>;
    /// Start the browser-based claim flow.
    async fn start_claim(&self) -> Result<ClaimResponse, PlayitError>;
    /// List existing tunnels.
    async fn list_tunnels(&self) -> Result<TunnelListResponse, PlayitError>;
    /// List every tunnel owned by the authenticated Playit account.
    async fn list_account_tunnels(&self) -> Result<AccountTunnelListResponse, PlayitError>;
    /// Create a tunnel.
    async fn create_tunnel(
        &self,
        local_port: u16,
        protocol: TunnelProtocol,
        local_address: Option<String>,
        name: Option<String>,
    ) -> Result<TunnelCreateResponse, PlayitError>;
    /// Create a semantic Minecraft Java tunnel.
    async fn create_minecraft_java_tunnel(
        &self,
        local_port: u16,
        local_address: Option<String>,
        name: Option<String>,
    ) -> Result<TunnelCreateResponse, PlayitError>;
    /// Delete a tunnel by its stable identifier.
    async fn delete_tunnel(&self, tunnel_id: &str) -> Result<CommandResponse, PlayitError>;
    /// Reassign a tunnel to the current agent and update its local destination.
    async fn reassign_tunnel(
        &self,
        tunnel_id: &str,
        local_port: u16,
        local_address: Option<String>,
    ) -> Result<CommandResponse, PlayitError>;
}

/// The optional external backend backed by a fresh Playit IPC connection per
/// operation.
#[derive(Debug, Clone, Copy, Default)]
pub struct IpcPlayitService;

#[async_trait]
impl PlayitService for IpcPlayitService {
    async fn status(&self) -> Result<ServiceStatus, PlayitError> {
        let mut client = IpcClient::connect().await?;
        Ok(client.status().await?)
    }

    async fn lifecycle(&self) -> Result<AgentLifecycle, PlayitError> {
        let mut client = IpcClient::connect().await?;
        Ok(client.lifecycle().await?)
    }

    async fn account(&self) -> Result<AccountResponse, PlayitError> {
        let mut client = IpcClient::connect().await?;
        Ok(client.account().await?)
    }

    async fn start_claim(&self) -> Result<ClaimResponse, PlayitError> {
        let mut client = IpcClient::connect().await?;
        Ok(client.start_claim().await?)
    }

    async fn list_tunnels(&self) -> Result<TunnelListResponse, PlayitError> {
        let mut client = IpcClient::connect().await?;
        Ok(client.list_tunnels().await?)
    }

    async fn list_account_tunnels(&self) -> Result<AccountTunnelListResponse, PlayitError> {
        let mut client = IpcClient::connect().await?;
        Ok(client.list_account_tunnels().await?)
    }

    async fn create_tunnel(
        &self,
        local_port: u16,
        protocol: TunnelProtocol,
        local_address: Option<String>,
        name: Option<String>,
    ) -> Result<TunnelCreateResponse, PlayitError> {
        let mut client = IpcClient::connect().await?;
        Ok(client
            .create_tunnel(local_port, protocol, local_address, name)
            .await?)
    }

    async fn create_minecraft_java_tunnel(
        &self,
        local_port: u16,
        local_address: Option<String>,
        name: Option<String>,
    ) -> Result<TunnelCreateResponse, PlayitError> {
        let mut client = IpcClient::connect().await?;
        Ok(client
            .create_minecraft_java_tunnel(local_port, local_address, name)
            .await?)
    }

    async fn delete_tunnel(&self, tunnel_id: &str) -> Result<CommandResponse, PlayitError> {
        let mut client = IpcClient::connect().await?;
        Ok(client.delete_tunnel(tunnel_id).await?)
    }

    async fn reassign_tunnel(
        &self,
        tunnel_id: &str,
        local_port: u16,
        local_address: Option<String>,
    ) -> Result<CommandResponse, PlayitError> {
        let mut client = IpcClient::connect().await?;
        Ok(client
            .reassign_tunnel(tunnel_id, local_port, local_address)
            .await?)
    }
}
