//! On-disk panel state.
//!
//! Everything the panel knows lives in one JSON file. There is no database:
//! a panel that manages a handful of servers on one box does not need one, and
//! not needing one is most of why this is easier to run than Pterodactyl.

use anyhow::{Context, Result};
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
        let data = match tokio::fs::read(&path).await {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .with_context(|| format!("{} is not valid panel state", path.display()))?,
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
        let out = f(&mut guard);
        let json = serde_json::to_vec_pretty(&*guard)?;

        // Unique per write, so an unrelated process writing beside us — or a
        // leftover file from an earlier crash — cannot be mistaken for ours.
        let tmp = self
            .path
            .with_extension(format!("json.{}.tmp", std::process::id()));

        tokio::fs::write(&tmp, &json)
            .await
            .with_context(|| format!("writing {}", tmp.display()))?;
        tokio::fs::rename(&tmp, &self.path)
            .await
            .with_context(|| format!("replacing {}", self.path.display()))?;

        drop(guard);
        Ok(out)
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
        };
        let mut document = serde_json::to_value(PanelData {
            users: vec![],
            servers: vec![record],
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
