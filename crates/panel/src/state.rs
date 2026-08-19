//! Shared application state: the store, the live guardians and the sessions.

use anyhow::{Context, Result};
use guardian::Guardian;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::auth::Sessions;
use crate::error::ApiError;
use crate::metrics::Metrics;
use crate::store::{ServerRecord, Store};
use crate::tickets::Tickets;

/// Everything the HTTP layer needs.
pub struct AppState {
    /// Persisted panel state.
    pub store: Arc<Store>,
    /// In-memory sessions.
    pub sessions: Sessions,
    /// One live guardian per configured server.
    guardians: RwLock<HashMap<String, Arc<Guardian>>>,
    /// Root for jdks, server directories and `panel.json`.
    pub data_dir: PathBuf,
    /// Per-process CPU and memory sampling.
    pub metrics: Metrics,
    /// Short-lived grants for browser-driven downloads.
    pub tickets: Tickets,
}

impl AppState {
    /// Build the state and instantiate a guardian for every stored server.
    ///
    /// Guardians are created but not started: a panel restart must not silently
    /// bring servers back up that the operator had stopped.
    pub async fn bootstrap(data_dir: impl Into<PathBuf>) -> Result<Arc<Self>> {
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
