//! Shared application state: the store, the live guardians and the sessions.

use anyhow::{Context, Result};
use clap::ValueEnum;
use guardian::{Guardian, ScopedFs};
use playit_integration::PlayitManager;
use std::collections::{HashMap, HashSet};
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::{Mutex, OwnedMutexGuard, RwLock, Semaphore};

use crate::auth::{LoginLimiter, Sessions};
use crate::error::ApiError;
use crate::limits::ResourceLimits;
use crate::metrics::Metrics;
use crate::store::{PanelData, ServerRecord, Store};
use crate::tickets::Tickets;

/// The Playit backend used by the panel.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
pub enum PlayitMode {
    /// Run Playit directly inside the panel process.
    #[default]
    Embedded,
    /// Connect to an independently managed external `playitd` process.
    External,
}

/// Everything the HTTP layer needs.
pub struct AppState {
    /// Persisted panel state.
    pub store: Arc<Store>,
    /// In-memory sessions.
    pub sessions: Sessions,
    /// Bounded login-attempt accounting and Argon2 concurrency control.
    pub login_limiter: LoginLimiter,
    /// One live guardian per configured server.
    guardians: RwLock<HashMap<String, Arc<Guardian>>>,
    /// Root for Playit state, JDKs, server directories and `panel.json`.
    pub data_dir: PathBuf,
    /// Per-process CPU and memory sampling.
    pub metrics: Metrics,
    /// Short-lived grants for browser-driven downloads.
    pub tickets: Tickets,
    /// The panel's embedded or explicitly external Playit integration.
    pub playit: Arc<PlayitManager>,
    /// Installation-wide request/filesystem resource limits.
    pub limits: ResourceLimits,
    /// Policy for platform sandbox helpers used by each Guardian.
    sandbox_policy: guardian::sandbox::SandboxPolicy,
    /// Bounds open file descriptors held by browser downloads.
    pub download_slots: Arc<Semaphore>,
    /// Bounds live WebSocket connections and their associated tasks.
    pub websocket_slots: Arc<Semaphore>,
    /// Listener peer addresses whose forwarded client headers are trusted.
    pub(crate) trusted_proxies: Arc<HashSet<IpAddr>>,
    /// Serializes server record/guardian/Playit mutations that must be observed
    /// as one lifecycle.  In particular, a delete cannot race an attachment
    /// into leaving an untracked public tunnel behind.
    pub server_mutation_lock: Mutex<()>,
}

impl AppState {
    /// Build the state and instantiate a guardian for every stored server.
    ///
    /// Guardians are created but not started: a panel restart must not silently
    /// bring servers back up that the operator had stopped.
    #[allow(dead_code)]
    pub async fn bootstrap(
        data_dir: impl Into<PathBuf>,
        playit_mode: PlayitMode,
    ) -> Result<Arc<Self>> {
        Self::bootstrap_with_limits_and_sandbox(
            data_dir,
            playit_mode,
            ResourceLimits::default(),
            false,
        )
        .await
    }

    /// Build state with explicit resource limits.
    #[allow(dead_code)]
    pub async fn bootstrap_with_limits(
        data_dir: impl Into<PathBuf>,
        playit_mode: PlayitMode,
        limits: ResourceLimits,
    ) -> Result<Arc<Self>> {
        Self::bootstrap_with_limits_and_sandbox(data_dir, playit_mode, limits, false).await
    }

    /// Build state with explicit resource limits and sandbox policy.
    pub async fn bootstrap_with_limits_and_sandbox(
        data_dir: impl Into<PathBuf>,
        playit_mode: PlayitMode,
        limits: ResourceLimits,
        allow_unsandboxed_servers: bool,
    ) -> Result<Arc<Self>> {
        Self::bootstrap_with_limits_and_sandbox_and_trusted_proxies(
            data_dir,
            playit_mode,
            limits,
            allow_unsandboxed_servers,
            Vec::new(),
        )
        .await
    }

    /// Build state with explicit limits, sandbox policy, and trusted proxy IPs.
    pub async fn bootstrap_with_limits_and_sandbox_and_trusted_proxies(
        data_dir: impl Into<PathBuf>,
        playit_mode: PlayitMode,
        limits: ResourceLimits,
        allow_unsandboxed_servers: bool,
        trusted_proxies: Vec<IpAddr>,
    ) -> Result<Arc<Self>> {
        limits.validate()?;
        let data_dir = data_dir.into();
        let sandbox_policy = guardian::sandbox::SandboxPolicy::new(allow_unsandboxed_servers);
        let trusted_proxies = Arc::new(trusted_proxies.into_iter().collect::<HashSet<_>>());
        tokio::fs::create_dir_all(&data_dir)
            .await
            .with_context(|| format!("creating {}", data_dir.display()))?;

        // Absolute from here on. The JVM is spawned with its working directory
        // set to the server folder, so a data dir like the default `./data`
        // would make every path stored under it unusable at launch time.
        let data_dir = tokio::fs::canonicalize(&data_dir)
            .await
            .with_context(|| format!("resolving {}", data_dir.display()))?;
        #[cfg(unix)]
        tokio::fs::set_permissions(
            &data_dir,
            std::os::unix::fs::PermissionsExt::from_mode(0o700),
        )
        .await
        .with_context(|| format!("protecting {}", data_dir.display()))?;
        secure_directory(&data_dir.join("servers")).await?;
        secure_directory(&data_dir.join("backups")).await?;
        secure_directory(&data_dir.join("playit")).await?;
        secure_directory(&data_dir.join("jdks")).await?;

        let store = Arc::new(Store::load(&data_dir).await?);
        normalize_server_records(&store, &data_dir, limits.max_server_memory_mb).await?;

        let playit = match playit_mode {
            PlayitMode::Embedded => {
                let secret_path = data_dir.join("playit").join("secret.toml");
                match PlayitManager::embedded(secret_path).await {
                    Ok(manager) => manager,
                    Err(error) => {
                        tracing::error!(
                            error = %error,
                            "failed to start embedded Playit runtime"
                        );
                        PlayitManager::unavailable(format!(
                            "embedded Playit runtime failed to start: {error}"
                        ))
                    }
                }
            }
            PlayitMode::External => PlayitManager::external(),
        };
        if tokio::fs::symlink_metadata(data_dir.join("playit").join("secret.toml"))
            .await
            .is_ok()
        {
            let playit_dir = data_dir.join("playit");
            let secret = playit_dir.join("secret.toml");
            let secret_for_task = secret.clone();
            tokio::task::spawn_blocking(move || -> Result<()> {
                let fs = ScopedFs::open(&playit_dir)
                    .with_context(|| format!("opening {}", playit_dir.display()))?;
                fs.set_file_private("secret.toml")
                    .with_context(|| format!("protecting {}", secret_for_task.display()))?;
                Ok(())
            })
            .await
            .with_context(|| format!("protecting {}", secret.display()))??;
        }

        let mut guardians = HashMap::new();
        for record in store.read().await.servers {
            guardians.insert(
                record.id.clone(),
                spawn_guardian(&record, &data_dir, sandbox_policy),
            );
        }

        Ok(Arc::new(AppState {
            store,
            sessions: Sessions::default(),
            login_limiter: LoginLimiter::default(),
            guardians: RwLock::new(guardians),
            data_dir,
            metrics: Metrics::default(),
            tickets: Tickets::default(),
            playit: Arc::new(playit),
            limits,
            sandbox_policy,
            download_slots: Arc::new(Semaphore::new(
                crate::limits::DEFAULT_MAX_CONCURRENT_DOWNLOADS,
            )),
            websocket_slots: Arc::new(Semaphore::new(
                crate::limits::DEFAULT_MAX_CONCURRENT_WEBSOCKETS,
            )),
            trusted_proxies,
            server_mutation_lock: Mutex::new(()),
        }))
    }

    /// The guardian for `id`, or a 404.
    pub async fn guardian(&self, id: &str) -> Result<Arc<Guardian>, ApiError> {
        self.guardians
            .read()
            .await
            .get(id)
            .cloned()
            .ok_or_else(|| ApiError::NotFound(format!("server {id}")))
    }

    /// Acquire the filesystem/quota lock owned by one live server.
    pub async fn server_resource_lock(
        &self,
        id: &str,
    ) -> std::result::Result<(Arc<Guardian>, OwnedMutexGuard<()>), ApiError> {
        let guardian = self.guardian(id).await?;
        let lock = guardian.lock_resources().await;
        Ok((guardian, lock))
    }

    /// Register a guardian for a newly created server.
    pub async fn insert_guardian(&self, record: &ServerRecord) -> Arc<Guardian> {
        let guardian = spawn_guardian(record, &self.data_dir, self.sandbox_policy);
        self.guardians
            .write()
            .await
            .insert(record.id.clone(), guardian.clone());
        guardian
    }

    /// Stop and drop a guardian after its resource lock has been acquired.
    ///
    /// Server deletion uses this while holding the lock so an in-flight file
    /// operation completes before the guardian is removed. The record should
    /// already have been removed from the store before this method is called.
    /// Returns an error if the guardian cannot be settled, so callers do not
    /// silently report deletion success while a managed process remains alive.
    #[allow(dead_code)]
    pub async fn remove_guardian_locked(
        &self,
        id: &str,
        guardian: Arc<Guardian>,
    ) -> anyhow::Result<()> {
        let status = guardian.status().await;
        let needs_stop = status.is_running() || status == guardian::ServerStatus::Preparing;
        if needs_stop {
            guardian
                .stop()
                .await
                .map_err(|e| anyhow::anyhow!("failed to stop server {id}: {e}"))?;
        }
        if !guardian
            .wait_for_settled(std::time::Duration::from_secs(10))
            .await
        {
            anyhow::bail!(
                "server {id} process did not exit; retaining guardian to avoid an orphan"
            );
        }
        self.guardians.write().await.remove(id);
        Ok(())
    }

    /// Remove a guardian that has already been stopped and verified settled.
    ///
    /// Used by server deletion after the caller has explicitly stopped the
    /// guardian and verified `wait_for_settled`. This avoids a second `stop`
    /// and makes the error path explicit.
    pub async fn remove_guardian_verified(
        &self,
        id: &str,
        guardian: Arc<Guardian>,
    ) -> anyhow::Result<()> {
        if !guardian
            .wait_for_settled(std::time::Duration::from_secs(10))
            .await
        {
            anyhow::bail!("server {id} did not settle; retaining guardian");
        }
        // Final verification that no child/preparation remains.
        let snapshot = guardian.snapshot().await;
        if snapshot.pid.is_some()
            || snapshot.status.is_running()
            || snapshot.status == guardian::ServerStatus::Preparing
        {
            anyhow::bail!("server {id} still has active process/preparation");
        }
        self.guardians.write().await.remove(id);
        Ok(())
    }

    /// Gracefully stop every managed server before the panel and its sandbox
    /// parents exit.  A hard kill is used only when the configured graceful
    /// stop takes longer than the panel shutdown budget.
    pub async fn shutdown_servers(&self) {
        let guardians = self
            .guardians
            .read()
            .await
            .iter()
            .map(|(id, guardian)| (id.clone(), Arc::clone(guardian)))
            .collect::<Vec<_>>();

        futures_util::future::join_all(guardians.into_iter().map(|(id, guardian)| async move {
            match tokio::time::timeout(std::time::Duration::from_secs(30), guardian.shutdown())
                .await
            {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    tracing::error!(server = %id, error = ?error, "server shutdown failed");
                }
                Err(_) => {
                    tracing::error!(server = %id, "server shutdown timed out; killing process");
                    let _ = guardian.kill().await;
                }
            }

            if !guardian
                .wait_for_settled(std::time::Duration::from_secs(10))
                .await
            {
                tracing::error!(server = %id, "server did not settle before panel exit");
            }
        }))
        .await;
    }

    /// The on-disk directory for `id`, whether or not it exists yet.
    pub fn server_dir(&self, id: &str) -> PathBuf {
        self.data_dir.join("servers").join(id)
    }

    /// Where `id`'s backups live.
    pub fn backup_dir(&self, id: &str) -> PathBuf {
        self.data_dir.join("backups").join(id)
    }

    /// Remove a just-created server directory through its parent capability.
    /// This is used only when a concurrent store update rejects a new record.
    pub async fn remove_uncommitted_server_dir(&self, id: &str) {
        let root = self.data_dir.join("servers");
        let id = id.to_owned();
        let _ = tokio::task::spawn_blocking(move || {
            guardian::ScopedFs::open(root).and_then(|fs| fs.remove(id))
        })
        .await;
    }
}

fn spawn_guardian(
    record: &ServerRecord,
    data_dir: &Path,
    sandbox_policy: guardian::sandbox::SandboxPolicy,
) -> Arc<Guardian> {
    Guardian::new_with_sandbox_policy(
        record.config.clone(),
        record.policy.clone(),
        data_dir,
        sandbox_policy,
    )
}

/// Rewrite server directories to the panel-managed roots and validate the
/// complete persisted state before any guardian is made runnable.
///
/// Earlier versions saved whatever `--data-dir` was given, so an install
/// started with the default `./data` has unusable paths on disk. Rewriting them
/// once at startup repairs those installs instead of leaving them permanently
/// unable to launch.
async fn normalize_server_records(
    store: &Store,
    data_dir: &Path,
    max_memory_mb: u32,
) -> Result<()> {
    let servers_root = data_dir.join("servers");
    store
        .try_update(|data| -> Result<()> {
            validate_persisted_invariants(data, max_memory_mb)?;
            for record in &mut data.servers {
                let repaired = servers_root.join(&record.id);
                if record.config.directory != repaired {
                    tracing::info!(
                        server = %record.id,
                        from = %record.config.directory.display(),
                        to = %repaired.display(),
                        "rewriting relative server directory"
                    );
                    record.config.directory = repaired;
                }
            }
            Ok(())
        })
        .await??;

    for record in store.read().await.servers {
        secure_directory(&record.config.directory).await?;
    }

    Ok(())
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id.chars().all(|character| {
            character.is_ascii_alphanumeric() || character == '-' || character == '_'
        })
}

fn validate_persisted_invariants(data: &PanelData, max_memory_mb: u32) -> Result<()> {
    let mut usernames = std::collections::HashSet::new();
    if !data.users.is_empty() && !data.users.iter().any(|user| user.admin) {
        anyhow::bail!("panel state has no administrator account");
    }
    for user in &data.users {
        if user.username.trim().is_empty()
            || user.username.len() > 128
            || user.username.contains(['\0', '\r', '\n'])
            || !usernames.insert(&user.username)
        {
            anyhow::bail!("panel state contains an invalid or duplicate username");
        }
    }

    let mut ids = std::collections::HashSet::new();
    let mut ports = std::collections::HashSet::new();
    let mut total_memory_mb = 0_u64;
    for record in &data.servers {
        if !valid_id(&record.id) || !ids.insert(&record.id) {
            anyhow::bail!("panel state contains an invalid or duplicate server id");
        }
        if record.name.trim().is_empty()
            || record.name.len() > 128
            || record.name.contains(['\0', '\r', '\n'])
        {
            anyhow::bail!("panel state contains an invalid server name");
        }
        record
            .config
            .validate()
            .map_err(|error| anyhow::anyhow!("invalid server {}: {error}", record.id))?;
        if record.config.memory.max_mb > max_memory_mb {
            anyhow::bail!("server {} exceeds the configured memory maximum", record.id);
        }
        total_memory_mb = total_memory_mb
            .checked_add(record.config.memory.max_mb as u64)
            .ok_or_else(|| anyhow::anyhow!("panel state exceeds the host memory budget"))?;
        if total_memory_mb > max_memory_mb as u64 {
            anyhow::bail!("panel state exceeds the aggregate host memory budget");
        }
        record
            .policy
            .validate()
            .map_err(|error| anyhow::anyhow!("invalid policy for server {}: {error}", record.id))?;
        if !ports.insert(record.config.port) {
            anyhow::bail!("panel state assigns one port to multiple servers");
        }
        if let Some(binding) = &record.playit {
            if binding.local_port == 0
                || (binding.local_address != "127.0.0.1" && binding.local_address != "::1")
            {
                anyhow::bail!("panel state contains an unsafe Playit destination");
            }
        }
    }

    for user in &data.users {
        let mut assigned = std::collections::HashSet::new();
        if user.servers.len() > 1024 {
            anyhow::bail!("panel state contains too many server permissions");
        }
        for id in &user.servers {
            if !ids.contains(id) || !assigned.insert(id) {
                anyhow::bail!("panel state contains an invalid server permission");
            }
        }
    }
    Ok(())
}

pub(crate) async fn secure_directory(path: &Path) -> Result<()> {
    tokio::fs::create_dir_all(path)
        .await
        .with_context(|| format!("creating {}", path.display()))?;
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || -> Result<()> {
        let fs = guardian::ScopedFs::open(&path)
            .map_err(|error| anyhow::anyhow!("opening {}: {error}", path.display()))?;
        fs.set_private()
            .map_err(|error| anyhow::anyhow!("protecting {}: {error}", path.display()))?;
        Ok(())
    })
    .await
    .map_err(|error| anyhow::anyhow!("protecting directory: {error}"))??;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{AppState, PlayitMode};
    use crate::store::ServerRecord;
    use guardian::{GuardianConfig, ServerConfig};
    use playit_integration::{PlayitConnectionState, PlayitManager};
    use std::sync::Arc;
    use std::time::Duration;

    #[tokio::test]
    async fn embedded_bootstrap_starts_without_an_external_daemon() {
        let data_dir = tempfile::tempdir().unwrap();
        let state = AppState::bootstrap(data_dir.path(), PlayitMode::Embedded)
            .await
            .unwrap();

        let status = state.playit.status().await.unwrap();
        assert!(matches!(
            status.status,
            PlayitConnectionState::NeedsClaim | PlayitConnectionState::Starting
        ));
        assert_eq!(
            state.data_dir.join("playit").join("secret.toml"),
            data_dir
                .path()
                .canonicalize()
                .unwrap()
                .join("playit")
                .join("secret.toml")
        );

        state.playit.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn external_bootstrap_succeeds_without_a_daemon() {
        let data_dir = tempfile::tempdir().unwrap();
        let state = AppState::bootstrap(data_dir.path(), PlayitMode::External)
            .await
            .unwrap();

        let error = tokio::time::timeout(Duration::from_secs(2), state.playit.status())
            .await
            .unwrap()
            .unwrap_err();
        assert_eq!(
            PlayitManager::status_from_error(&error).status,
            PlayitConnectionState::Unavailable
        );

        state.playit.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn removing_a_server_drops_its_lifecycle_owned_resource_lock() {
        let data_dir = tempfile::tempdir().unwrap();
        let state = AppState::bootstrap(data_dir.path(), PlayitMode::External)
            .await
            .unwrap();
        let record = ServerRecord {
            id: "server-1".into(),
            name: "Server".into(),
            config: ServerConfig::paper(state.server_dir("server-1"), "1.21.8"),
            policy: GuardianConfig::default(),
            playit: None,
            created_at: "2026-08-28T00:00:00Z".into(),
        };
        let guardian = state.insert_guardian(&record).await;
        let weak = Arc::downgrade(&guardian);
        let resource_lock = guardian.lock_resources().await;

        state
            .remove_guardian_locked("server-1", Arc::clone(&guardian))
            .await
            .unwrap();
        assert!(state.guardian("server-1").await.is_err());

        drop(resource_lock);
        drop(guardian);
        assert!(weak.upgrade().is_none());
        state.playit.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn deleting_server_removes_stale_user_permissions() {
        let data_dir = tempfile::tempdir().unwrap();
        let state = AppState::bootstrap(data_dir.path(), PlayitMode::External)
            .await
            .unwrap();
        let record = ServerRecord {
            id: "srv-1".into(),
            name: "Server".into(),
            config: ServerConfig::paper(state.server_dir("srv-1"), "1.21.8"),
            policy: GuardianConfig::default(),
            playit: None,
            created_at: "2026-08-28T00:00:00Z".into(),
        };
        state
            .store
            .update(|data| {
                // Ensure an admin exists for invariant validation
                data.users.push(crate::store::User {
                    username: "admin".into(),
                    password_hash: "$argon2id$fake".into(),
                    admin: true,
                    servers: vec![],
                });
                data.servers.push(record.clone())
            })
            .await
            .unwrap();
        // Single user with permission
        state
            .store
            .update(|data| {
                data.users.push(crate::store::User {
                    username: "bob".into(),
                    password_hash: "$argon2id$fake".into(),
                    admin: false,
                    servers: vec!["srv-1".into()],
                })
            })
            .await
            .unwrap();
        // Simulate atomic deletion that also cleans user perms (as fixed)
        state
            .store
            .update(|data| {
                data.servers.retain(|s| s.id != "srv-1");
                for user in &mut data.users {
                    user.servers.retain(|sid| sid != "srv-1");
                }
            })
            .await
            .unwrap();
        let data = state.store.read().await;
        assert!(!data.users[0].servers.contains(&"srv-1".to_string()));
        assert!(data.servers.is_empty());
        // Persist/reload must validate
        drop(data);
        let reloaded = crate::store::Store::load(data_dir.path()).await.unwrap();
        let reloaded_data = reloaded.read().await;
        assert!(reloaded_data.servers.is_empty());
        assert!(reloaded_data.users[0].servers.is_empty());
        // Invariants must hold
        super::validate_persisted_invariants(&reloaded_data, state.limits.max_server_memory_mb)
            .unwrap();
        state.playit.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn deleting_server_cleans_multiple_users_and_admin() {
        let data_dir = tempfile::tempdir().unwrap();
        let state = AppState::bootstrap(data_dir.path(), PlayitMode::External)
            .await
            .unwrap();
        let record = ServerRecord {
            id: "srv-1".into(),
            name: "Server".into(),
            config: ServerConfig::paper(state.server_dir("srv-1"), "1.21.8"),
            policy: GuardianConfig::default(),
            playit: None,
            created_at: "2026-08-28T00:00:00Z".into(),
        };
        state
            .store
            .update(|data| data.servers.push(record.clone()))
            .await
            .unwrap();
        state
            .store
            .update(|data| {
                data.users.push(crate::store::User {
                    username: "alice".into(),
                    password_hash: "$argon2id$fake".into(),
                    admin: false,
                    servers: vec!["srv-1".into()],
                });
                data.users.push(crate::store::User {
                    username: "bob".into(),
                    password_hash: "$argon2id$fake".into(),
                    admin: false,
                    servers: vec!["srv-1".into()],
                });
                data.users.push(crate::store::User {
                    username: "admin".into(),
                    password_hash: "$argon2id$fake".into(),
                    admin: true,
                    servers: vec!["srv-1".into()],
                });
            })
            .await
            .unwrap();
        state
            .store
            .update(|data| {
                data.servers.retain(|s| s.id != "srv-1");
                for user in &mut data.users {
                    user.servers.retain(|sid| sid != "srv-1");
                }
            })
            .await
            .unwrap();
        let data = state.store.read().await;
        for user in &data.users {
            assert!(!user.servers.contains(&"srv-1".to_string()));
        }
        state.playit.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn deleting_unassigned_server_leaves_permissions_untouched() {
        let data_dir = tempfile::tempdir().unwrap();
        let state = AppState::bootstrap(data_dir.path(), PlayitMode::External)
            .await
            .unwrap();
        let record = ServerRecord {
            id: "srv-1".into(),
            name: "Server".into(),
            config: ServerConfig::paper(state.server_dir("srv-1"), "1.21.8"),
            policy: GuardianConfig::default(),
            playit: None,
            created_at: "2026-08-28T00:00:00Z".into(),
        };
        let other = ServerRecord {
            id: "srv-2".into(),
            name: "Other".into(),
            config: ServerConfig::paper(state.server_dir("srv-2"), "1.21.8"),
            policy: GuardianConfig::default(),
            playit: None,
            created_at: "2026-08-28T00:00:00Z".into(),
        };
        state
            .store
            .update(|data| {
                data.servers.push(record.clone());
                data.servers.push(other.clone());
                data.users.push(crate::store::User {
                    username: "bob".into(),
                    password_hash: "$argon2id$fake".into(),
                    admin: false,
                    servers: vec!["srv-2".into()],
                });
            })
            .await
            .unwrap();
        state
            .store
            .update(|data| {
                data.servers.retain(|s| s.id != "srv-1");
                for user in &mut data.users {
                    user.servers.retain(|sid| sid != "srv-1");
                }
            })
            .await
            .unwrap();
        let data = state.store.read().await;
        assert_eq!(data.servers.len(), 1);
        assert_eq!(data.servers[0].id, "srv-2");
        assert_eq!(data.users[0].servers, vec!["srv-2"]);
        state.playit.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn failed_server_creation_leaves_no_orphan_directory() {
        let data_dir = tempfile::tempdir().unwrap();
        let state = AppState::bootstrap(data_dir.path(), PlayitMode::External)
            .await
            .unwrap();
        let id = "orphan-test-server";
        let dir = state.server_dir(id);
        crate::state::secure_directory(&dir).await.unwrap();
        assert!(dir.exists());
        // Simulate persistence failure cleanup as in create()
        state.remove_uncommitted_server_dir(id).await;
        assert!(!dir.exists());
        // Ensure second removal is idempotent
        state.remove_uncommitted_server_dir(id).await;
        assert!(!dir.exists());
        state.playit.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn shutdown_waits_for_preparation_and_prevents_new_starts() {
        let data_dir = tempfile::tempdir().unwrap();
        let state = AppState::bootstrap(data_dir.path(), PlayitMode::External)
            .await
            .unwrap();
        let record = ServerRecord {
            id: "server-shutdown".into(),
            name: "ShutdownTest".into(),
            config: ServerConfig::paper(state.server_dir("server-shutdown"), "1.21.8"),
            policy: GuardianConfig::default(),
            playit: None,
            created_at: "2026-08-28T00:00:00Z".into(),
        };
        let guardian = state.insert_guardian(&record).await;
        // Mark shutting down
        guardian.shutdown().await.unwrap();
        // New starts must be rejected
        assert!(guardian.start().await.is_err());
        assert!(guardian.reinstall().await.is_err());
        // shutdown_servers should settle quickly for offline server
        state.shutdown_servers().await;
        assert!(
            guardian
                .wait_for_settled(std::time::Duration::from_secs(1))
                .await
        );
        state.playit.shutdown().await.unwrap();
    }
}
