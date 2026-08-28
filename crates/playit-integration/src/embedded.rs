//! Direct Playit runtime backend.

use async_trait::async_trait;
use playit_ipc::model::{
    AccountResponse, AgentLifecycle, ClaimResponse, CommandResponse, ServiceStatus,
    TunnelCreateResponse, TunnelListResponse, TunnelProtocol,
};
use playit_runtime::PlayitHandle;

use crate::client::PlayitService;
use crate::error::PlayitError;

/// A Playit backend that talks directly to an in-process runtime handle.
///
/// This backend never creates or connects to a Playit IPC endpoint. The
/// runtime handle is cloneable, so all service clones operate on the same
/// embedded runtime owner.
#[derive(Clone)]
pub struct EmbeddedPlayitService {
    handle: PlayitHandle,
}

impl EmbeddedPlayitService {
    /// Create a service around an already-started embedded runtime.
    pub fn new(handle: PlayitHandle) -> Self {
        Self { handle }
    }
}

#[async_trait]
impl PlayitService for EmbeddedPlayitService {
    async fn status(&self) -> Result<ServiceStatus, PlayitError> {
        Ok(self.handle.status().await)
    }

    async fn lifecycle(&self) -> Result<AgentLifecycle, PlayitError> {
        Ok(self.handle.lifecycle().await)
    }

    async fn account(&self) -> Result<AccountResponse, PlayitError> {
        Ok(self.handle.account().await?)
    }

    async fn start_claim(&self) -> Result<ClaimResponse, PlayitError> {
        Ok(self.handle.start_claim().await?)
    }

    async fn list_tunnels(&self) -> Result<TunnelListResponse, PlayitError> {
        Ok(self.handle.list_tunnels().await?)
    }

    async fn create_tunnel(
        &self,
        local_port: u16,
        protocol: TunnelProtocol,
        local_address: Option<String>,
        name: Option<String>,
    ) -> Result<TunnelCreateResponse, PlayitError> {
        Ok(self
            .handle
            .create_tunnel(local_port, protocol, local_address, name)
            .await?)
    }

    async fn create_minecraft_java_tunnel(
        &self,
        local_port: u16,
        local_address: Option<String>,
        name: Option<String>,
    ) -> Result<TunnelCreateResponse, PlayitError> {
        Ok(self
            .handle
            .create_minecraft_java_tunnel(local_port, local_address, name)
            .await?)
    }

    async fn delete_tunnel(&self, tunnel_id: &str) -> Result<CommandResponse, PlayitError> {
        Ok(self.handle.delete_tunnel(tunnel_id).await?)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::Duration;

    use super::EmbeddedPlayitService;
    use crate::client::PlayitService;
    use crate::error::PlayitError;
    use playit_runtime::{AgentLifecycle, PlayitRuntime, RuntimeOptions};
    use tokio::net::TcpListener;

    fn secret_path(test_name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "mcpanel-playit-{test_name}-{}-{}.toml",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock must be after the Unix epoch")
                .as_nanos()
        ))
    }

    async fn wait_for_waiting(service: &EmbeddedPlayitService) {
        for _ in 0..100 {
            if matches!(
                service.lifecycle().await.unwrap(),
                AgentLifecycle::WaitingForSecret
            ) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("embedded runtime did not reach WaitingForSecret");
    }

    #[tokio::test]
    async fn reads_direct_runtime_state_without_ipc() {
        let secret_path = secret_path("state");
        let (runtime, handle) = PlayitRuntime::start(RuntimeOptions {
            secret_path: secret_path.clone(),
            ..RuntimeOptions::default()
        })
        .await
        .unwrap();
        let service = EmbeddedPlayitService::new(handle);

        wait_for_waiting(&service).await;
        let status = service.status().await.unwrap();
        assert!(status.socket_path.is_empty());
        assert!(!status.has_secret);

        runtime.shutdown().await.unwrap();
        let _ = tokio::fs::remove_file(secret_path).await;
    }

    #[tokio::test]
    async fn runtime_errors_are_preserved_as_runtime_errors() {
        let secret_path = secret_path("error");
        let (runtime, handle) = PlayitRuntime::start(RuntimeOptions {
            secret_path: secret_path.clone(),
            ..RuntimeOptions::default()
        })
        .await
        .unwrap();
        let service = EmbeddedPlayitService::new(handle);
        runtime.shutdown().await.unwrap();

        assert!(matches!(
            service.account().await,
            Err(PlayitError::Runtime(playit_runtime::RuntimeError::Stopped))
        ));
        assert!(service.account().await.unwrap_err().is_unavailable());
        let _ = tokio::fs::remove_file(secret_path).await;
    }

    #[tokio::test]
    async fn claim_is_direct_and_idempotent() {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let api_base = format!("http://{}", listener.local_addr().unwrap());
        let (server_cancel, mut server_cancel_task) = tokio::sync::oneshot::channel();
        let server_task = tokio::spawn(async move {
            let mut connections = Vec::new();
            loop {
                tokio::select! {
                    _ = &mut server_cancel_task => break,
                    accepted = listener.accept() => {
                        if let Ok((stream, _)) = accepted {
                            connections.push(stream);
                        }
                    }
                }
            }
            drop(connections);
        });

        let secret_path = secret_path("claim");
        let (runtime, handle) = PlayitRuntime::start(RuntimeOptions {
            secret_path: secret_path.clone(),
            api_base,
            ..RuntimeOptions::default()
        })
        .await
        .unwrap();
        let service = EmbeddedPlayitService::new(handle);
        wait_for_waiting(&service).await;

        let first = service.start_claim().await.unwrap();
        let second = service.start_claim().await.unwrap();
        assert_eq!(first.claim_url, second.claim_url);

        runtime.shutdown().await.unwrap();
        let _ = server_cancel.send(());
        server_task.await.unwrap();
        let _ = tokio::fs::remove_file(secret_path).await;
    }
}
