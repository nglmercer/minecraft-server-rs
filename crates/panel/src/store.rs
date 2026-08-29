//! On-disk panel state.
//!
//! Everything the panel knows lives in one JSON file. There is no database:
//! a panel that manages a handful of servers on one box does not need one, and
//! not needing one is most of why this is easier to run than Pterodactyl.

use anyhow::{Context, Result};
use guardian::ScopedFs;
use playit_integration::PlayitProtocol;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio::sync::RwLock;

/// A panel account.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    /// Login name, unique and case-sensitive.
    pub username: String,
    /// PHC-format Argon2 hash. Never leaves the backend.
    #[serde(rename = "password_hash")]
    pub password_hash: String,
    /// Admins may create and delete servers and manage users.
    pub admin: bool,
    /// Server ids a non-admin may see. Ignored for admins.
    #[serde(default)]
    pub servers: Vec<String>,
}

/// A panel-owned association with one Playit tunnel.
///
/// The Playit service remains the source of truth for the tunnel's public
/// address and operational state. The panel only persists the stable id and
/// the local destination it asked Playit to expose.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayitBinding {
    /// Stable id assigned by Playit.
    pub tunnel_id: String,
    /// Transport requested for this binding.
    pub protocol: PlayitProtocol,
    /// Local address exposed through the tunnel.
    pub local_address: String,
    /// Local port exposed through the tunnel.
    pub local_port: u16,
}

/// How many and how long to keep successful backups.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupRetentionPolicy {
    /// Keep at most this many successful backups.
    pub max_backups: usize,
    /// Delete backups older than this many days, if Some.
    pub max_age_days: Option<u32>,
}

impl Default for BackupRetentionPolicy {
    fn default() -> Self {
        Self {
            max_backups: 1,
            max_age_days: None,
        }
    }
}

impl BackupRetentionPolicy {
    pub fn validate(&self) -> Result<(), String> {
        if self.max_backups == 0 {
            return Err("max_backups must be at least 1".into());
        }
        if self.max_backups > 1000 {
            return Err("max_backups must not exceed 1000".into());
        }
        if let Some(days) = self.max_age_days {
            if days == 0 {
                return Err("max_age_days must be at least 1 when present".into());
            }
        }
        Ok(())
    }
}

/// Kind of backup provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum BackupProviderKind {
    #[default]
    Local,
    GoogleDrive,
}

impl std::fmt::Display for BackupProviderKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Local => write!(f, "local"),
            Self::GoogleDrive => write!(f, "google_drive"),
        }
    }
}

/// Google Drive provider configuration (no secrets here, only folder + ref).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoogleDriveConfig {
    pub folder_id: String,
    pub credential_ref: String,
}

impl GoogleDriveConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.folder_id.trim().is_empty() || self.folder_id.len() > 512 {
            return Err("folder_id must be 1-512 chars".into());
        }
        if self.credential_ref.trim().is_empty()
            || self.credential_ref.len() > 64
            || !self
                .credential_ref
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err("credential_ref must be 1-64 alphanumeric/_/-".into());
        }
        Ok(())
    }
}

/// Global backup storage settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupStorageSettings {
    pub provider: BackupProviderKind,
    pub retention: BackupRetentionPolicy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub google_drive: Option<GoogleDriveConfig>,
}

impl Default for BackupStorageSettings {
    fn default() -> Self {
        Self {
            provider: BackupProviderKind::Local,
            retention: BackupRetentionPolicy::default(),
            google_drive: None,
        }
    }
}

/// Per-server override for backup settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerBackupPolicy {
    #[serde(default)]
    pub provider: BackupProviderKind,
    #[serde(default)]
    pub retention: BackupRetentionPolicy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub google_drive: Option<GoogleDriveConfig>,
}

/// Provider-neutral backup metadata, persisted in panel.json.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredBackup {
    pub id: String,
    pub server_id: String,
    pub provider: BackupProviderKind,
    pub remote_id: String,
    pub created_at: String,
    pub size_bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checksum_sha256: Option<String>,
    #[serde(default)]
    pub note: String,
}

/// A server as the panel stores it: identity plus the two guardian configs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerRecord {
    /// Stable identifier, also the directory name under `servers/`.
    pub id: String,
    /// Human-facing name.
    pub name: String,
    /// What to run.
    pub config: guardian::ServerConfig,
    /// How to supervise it.
    pub policy: guardian::GuardianConfig,
    /// Optional panel-managed Playit tunnel.
    #[serde(default)]
    pub playit: Option<PlayitBinding>,
    /// RFC 3339 creation timestamp.
    pub created_at: String,
    /// Optional per-server backup override. None means inherit global.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backup_policy: Option<ServerBackupPolicy>,
}

/// The entire persisted document.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PanelData {
    /// Every account.
    #[serde(default)]
    pub users: Vec<User>,
    /// Every configured server.
    #[serde(default)]
    pub servers: Vec<ServerRecord>,
    /// Global backup storage settings.
    #[serde(default)]
    pub backup_storage: BackupStorageSettings,
    /// All successful backups across all servers and providers.
    #[serde(default)]
    pub backups: Vec<StoredBackup>,
}

/// Reads and writes [`PanelData`], serialising concurrent writers.
pub struct Store {
    path: PathBuf,
    data: RwLock<PanelData>,
}

impl Store {
    /// Load `panel.json` from `data_dir`, starting empty if it does not exist.
    pub async fn load(data_dir: &Path) -> Result<Self> {
        let path = data_dir.join("panel.json");
        let root = ScopedFs::open(data_dir)
            .with_context(|| format!("opening data directory {}", data_dir.display()))?;
        let data = match root.read_file("panel.json") {
            Ok(bytes) => {
                root.set_file_private("panel.json")
                    .with_context(|| format!("protecting {}", path.display()))?;
                serde_json::from_slice(&bytes)
                    .with_context(|| format!("{} is not valid panel state", path.display()))?
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => PanelData::default(),
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        Ok(Store {
            path,
            data: RwLock::new(data),
        })
    }

    /// A snapshot of the current state.
    pub async fn read(&self) -> PanelData {
        self.data.read().await.clone()
    }

    /// Mutate the state and persist the result atomically.
    ///
    /// The write goes to a temporary file and is renamed into place, so a crash
    /// mid-write cannot leave a truncated `panel.json` behind.
    ///
    /// The lock is deliberately held across the write rather than released once
    /// the document has been serialised. Releasing it early lets two writers
    /// serialise in one order and reach the filesystem in the other, so the disk
    /// ends up holding a state that memory has already moved past — and with a
    /// shared temporary name, one writer renames the file out from under the
    /// other and that update fails outright.
    pub async fn update<T>(&self, f: impl FnOnce(&mut PanelData) -> T) -> Result<T> {
        let mut guard = self.data.write().await;
        // Work on a private snapshot until the new document is safely on
        // disk. A failed serialization, write, or rename must not leave the
        // in-memory state ahead of the only durable state.
        let mut next = guard.clone();
        let out = f(&mut next);
        let json = serde_json::to_vec_pretty(&next)?;

        let root = self
            .path
            .parent()
            .map(Path::to_path_buf)
            .ok_or_else(|| anyhow::anyhow!("panel state has no parent directory"))?;
        let path = self.path.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let fs = ScopedFs::open(&root)
                .with_context(|| format!("opening state directory {}", root.display()))?;
            fs.write_atomic("panel.json", &json)
                .with_context(|| format!("replacing {}", path.display()))?;
            Ok(())
        })
        .await
        .map_err(|error| anyhow::anyhow!("state write task failed: {error}"))??;

        *guard = next;
        drop(guard);
        Ok(out)
    }

    /// Validate and mutate a state snapshot transactionally.
    ///
    /// A closure that returns an error must not accidentally persist the
    /// partially edited snapshot.  Handlers that enforce an invariant inside
    /// the closure use this method rather than `update`, so a rejected request
    /// leaves both memory and disk unchanged.
    pub async fn try_update<T, E>(
        &self,
        f: impl FnOnce(&mut PanelData) -> std::result::Result<T, E>,
    ) -> Result<std::result::Result<T, E>>
    where
        T: Send + 'static,
        E: Send + 'static,
    {
        let mut guard = self.data.write().await;
        let mut next = guard.clone();
        let value = match f(&mut next) {
            Ok(value) => value,
            Err(error) => return Ok(Err(error)),
        };
        let json = serde_json::to_vec_pretty(&next)?;
        let root = self
            .path
            .parent()
            .map(Path::to_path_buf)
            .ok_or_else(|| anyhow::anyhow!("panel state has no parent directory"))?;
        let path = self.path.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let fs = ScopedFs::open(&root)
                .with_context(|| format!("opening state directory {}", root.display()))?;
            fs.write_atomic("panel.json", &json)
                .with_context(|| format!("replacing {}", path.display()))?;
            Ok(())
        })
        .await
        .map_err(|error| anyhow::anyhow!("state write task failed: {error}"))??;

        *guard = next;
        Ok(Ok(value))
    }

    /// Find a user by name.
    pub async fn user(&self, username: &str) -> Option<User> {
        self.data
            .read()
            .await
            .users
            .iter()
            .find(|u| u.username == username)
            .cloned()
    }

    /// Find a server record by id.
    pub async fn server(&self, id: &str) -> Option<ServerRecord> {
        self.data
            .read()
            .await
            .servers
            .iter()
            .find(|s| s.id == id)
            .cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(name: &str, admin: bool) -> User {
        User {
            username: name.into(),
            password_hash: "$argon2id$fake".into(),
            admin,
            servers: vec![],
        }
    }

    #[tokio::test]
    async fn a_missing_file_loads_as_empty_state() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::load(tmp.path()).await.unwrap();

        let data = store.read().await;
        assert!(data.users.is_empty());
        assert!(data.servers.is_empty());
    }

    #[tokio::test]
    async fn updates_persist_across_a_reload() {
        let tmp = tempfile::tempdir().unwrap();

        {
            let store = Store::load(tmp.path()).await.unwrap();
            store
                .update(|data| data.users.push(user("admin", true)))
                .await
                .unwrap();
        }

        let reloaded = Store::load(tmp.path()).await.unwrap();
        let found = reloaded
            .user("admin")
            .await
            .expect("the account should have survived");

        assert!(found.admin);
        assert!(reloaded.user("nobody").await.is_none());
    }

    #[tokio::test]
    async fn a_write_leaves_no_temporary_file_behind() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::load(tmp.path()).await.unwrap();
        store
            .update(|data| data.users.push(user("admin", true)))
            .await
            .unwrap();

        assert!(tmp.path().join("panel.json").exists());
        assert!(
            !tmp.path().join("panel.json.tmp").exists(),
            "the atomic rename should have consumed the temporary file"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn panel_state_is_owner_only_after_atomic_replacement() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::tempdir().unwrap();
        let store = Store::load(tmp.path()).await.unwrap();
        store
            .update(|data| data.users.push(user("admin", true)))
            .await
            .unwrap();

        let mode = std::fs::metadata(tmp.path().join("panel.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_panel_state_symlink_is_rejected_without_touching_its_target() {
        let tmp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let target = outside.path().join("real-panel.json");
        std::fs::write(&target, b"{}\n").unwrap();
        std::os::unix::fs::symlink(&target, tmp.path().join("panel.json")).unwrap();

        assert!(Store::load(tmp.path()).await.is_err());
        assert_eq!(std::fs::read_to_string(target).unwrap(), "{}\n");
    }

    #[tokio::test]
    async fn update_returns_the_closure_result() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::load(tmp.path()).await.unwrap();

        let count = store
            .update(|data| {
                data.users.push(user("a", true));
                data.users.push(user("b", false));
                data.users.len()
            })
            .await
            .unwrap();

        assert_eq!(count, 2);
    }

    #[tokio::test]
    async fn a_rejected_transaction_does_not_persist_partial_mutations() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::load(tmp.path()).await.unwrap();

        let result = store
            .try_update(|data| -> std::result::Result<(), &'static str> {
                data.users.push(user("not-saved", false));
                Err("validation failed")
            })
            .await
            .unwrap();

        assert_eq!(result, Err("validation failed"));
        assert!(store.read().await.users.is_empty());
        assert!(!tmp.path().join("panel.json").exists());
    }

    #[tokio::test]
    async fn concurrent_updates_all_survive() {
        let tmp = tempfile::tempdir().unwrap();
        let store = std::sync::Arc::new(Store::load(tmp.path()).await.unwrap());

        // Several requests mutating panel state at once is ordinary: two admins,
        // or one admin and a background write. None of them may be lost, and the
        // document must never be left unreadable.
        let writers: Vec<_> = (0..25)
            .map(|i| {
                let store = store.clone();
                tokio::spawn(async move {
                    store
                        .update(move |data| data.users.push(user(&format!("user{i}"), false)))
                        .await
                        .unwrap();
                })
            })
            .collect();

        for writer in writers {
            writer.await.unwrap();
        }

        let reloaded = Store::load(tmp.path())
            .await
            .expect("panel.json must still be readable");

        assert_eq!(
            reloaded.read().await.users.len(),
            25,
            "an update was lost between memory and disk"
        );
    }

    #[tokio::test]
    async fn a_corrupt_document_fails_loudly_rather_than_silently_resetting() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("panel.json"), "{ this is not json").unwrap();

        // Starting fresh here would silently orphan every configured server.
        assert!(Store::load(tmp.path()).await.is_err());
    }

    #[tokio::test]
    async fn older_server_records_load_without_a_playit_binding() {
        let tmp = tempfile::tempdir().unwrap();
        let record = ServerRecord {
            id: "server-1".into(),
            name: "Survival".into(),
            config: guardian::ServerConfig::paper(tmp.path().join("server-1"), "1.21.8"),
            policy: guardian::GuardianConfig::default(),
            playit: None,
            created_at: "2026-08-28T00:00:00Z".into(),
            backup_policy: None,
        };
        let mut document = serde_json::to_value(PanelData {
            users: vec![],
            servers: vec![record],
            ..Default::default()
        })
        .unwrap();
        document["servers"][0]
            .as_object_mut()
            .unwrap()
            .remove("playit");
        std::fs::write(
            tmp.path().join("panel.json"),
            serde_json::to_vec_pretty(&document).unwrap(),
        )
        .unwrap();

        let loaded = Store::load(tmp.path()).await.unwrap();
        assert!(loaded.read().await.servers[0].playit.is_none());
    }

    #[tokio::test]
    async fn playit_bindings_survive_a_store_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::load(tmp.path()).await.unwrap();
        let record = ServerRecord {
            id: "server-1".into(),
            name: "Survival".into(),
            config: guardian::ServerConfig::paper(tmp.path().join("server-1"), "1.21.8"),
            policy: guardian::GuardianConfig::default(),
            playit: Some(PlayitBinding {
                tunnel_id: "tunnel-1".into(),
                protocol: PlayitProtocol::Tcp,
                local_address: "127.0.0.1".into(),
                local_port: 25565,
            }),
            created_at: "2026-08-28T00:00:00Z".into(),
            backup_policy: None,
        };

        store
            .update(|data| data.servers.push(record))
            .await
            .unwrap();

        let reloaded = Store::load(tmp.path()).await.unwrap();
        let binding = reloaded.read().await.servers[0].playit.clone().unwrap();
        assert_eq!(binding.tunnel_id, "tunnel-1");
        assert_eq!(binding.protocol, PlayitProtocol::Tcp);
        assert_eq!(binding.local_port, 25565);
    }
}
