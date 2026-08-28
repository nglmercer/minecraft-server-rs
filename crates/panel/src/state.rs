//! Shared application state: the store, the live guardians and the sessions.

use anyhow::{Context, Result};
use clap::ValueEnum;
use guardian::Guardian;
use playit_integration::PlayitManager;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::auth::Sessions;
use crate::error::ApiError;
use crate::metrics::Metrics;
use crate::store::{ServerRecord, Store};
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
}

impl AppState {
    /// Build the state and instantiate a guardian for every stored server.
    ///
    /// Guardians are created but not started: a panel restart must not silently
    /// bring servers back up that the operator had stopped.
    pub async fn bootstrap(
        data_dir: impl Into<PathBuf>,
        playit_mode: PlayitMode,
    ) -> Result<Arc<Self>> {
        let data_dir = data_dir.into();
        tokio::fs::create_dir_all(data_dir.join("servers"))
            .await
            .with_context(|| format!("creating {}", data_dir.display()))?;

        // Absolute from here on. The JVM is spawned with its working directory
        // set to the server folder, so a data dir like the default `./data`
        // would make every path stored under it unusable at launch time.
        let data_dir = tokio::fs::canonicalize(&data_dir)
            .await
            .with_context(|| format!("resolving {}", data_dir.display()))?;

        let store = Arc::new(Store::load(&data_dir).await?);
        migrate_relative_directories(&store, &data_dir).await?;

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

        let mut guardians = HashMap::new();
        for record in store.read().await.servers {
            guardians.insert(record.id.clone(), spawn_guardian(&record, &data_dir));
        }

        Ok(Arc::new(AppState {
            store,
            sessions: Sessions::default(),
            guardians: RwLock::new(guardians),
            data_dir,
            metrics: Metrics::default(),
            tickets: Tickets::default(),
            playit: Arc::new(playit),
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

    /// Register a guardian for a newly created server.
    pub async fn insert_guardian(&self, record: &ServerRecord) -> Arc<Guardian> {
        let guardian = spawn_guardian(record, &self.data_dir);
        self.guardians
            .write()
            .await
            .insert(record.id.clone(), guardian.clone());
        guardian
    }

    /// Drop a guardian, killing its process if one is running.
    pub async fn remove_guardian(&self, id: &str) {
        if let Some(guardian) = self.guardians.write().await.remove(id) {
            let _ = guardian.kill().await;
        }
    }

    /// The on-disk directory for `id`, whether or not it exists yet.
    pub fn server_dir(&self, id: &str) -> PathBuf {
        self.data_dir.join("servers").join(id)
    }

    /// Where `id`'s backups live.
    pub fn backup_dir(&self, id: &str) -> PathBuf {
        self.data_dir.join("backups").join(id)
    }
}

fn spawn_guardian(record: &ServerRecord, data_dir: &Path) -> Arc<Guardian> {
    Guardian::new(record.config.clone(), record.policy.clone(), data_dir)
}

/// Rewrite server directories that were stored relative to the panel's cwd.
///
/// Earlier versions saved whatever `--data-dir` was given, so an install
/// started with the default `./data` has unusable paths on disk. Rewriting them
/// once at startup repairs those installs instead of leaving them permanently
/// unable to launch.
async fn migrate_relative_directories(store: &Store, data_dir: &Path) -> Result<()> {
    let needs_migration = store
        .read()
        .await
        .servers
        .iter()
        .any(|record| record.config.directory.is_relative());

    if !needs_migration {
        return Ok(());
    }

    let servers_root = data_dir.join("servers");
    store
        .update(|data| {
            for record in data.servers.iter_mut() {
                if record.config.directory.is_relative() {
                    let repaired = servers_root.join(&record.id);
                    tracing::info!(
                        server = %record.id,
                        from = %record.config.directory.display(),
                        to = %repaired.display(),
                        "rewriting relative server directory"
                    );
                    record.config.directory = repaired;
                }
            }
        })
        .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{AppState, PlayitMode};
    use playit_integration::{PlayitConnectionState, PlayitManager};
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
}
