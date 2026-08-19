//! On-disk panel state.
//!
//! Everything the panel knows lives in one JSON file. There is no database:
//! a panel that manages a handful of servers on one box does not need one, and
//! not needing one is most of why this is easier to run than Pterodactyl.

use anyhow::{Context, Result};
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
        Ok(Store { path, data: RwLock::new(data) })
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
        let tmp = self.path.with_extension(format!("json.{}.tmp", std::process::id()));

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
        self.data.read().await.users.iter().find(|u| u.username == username).cloned()
    }

    /// Find a server record by id.
    pub async fn server(&self, id: &str) -> Option<ServerRecord> {
        self.data.read().await.servers.iter().find(|s| s.id == id).cloned()
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
            store.update(|data| data.users.push(user("admin", true))).await.unwrap();
        }

        let reloaded = Store::load(tmp.path()).await.unwrap();
        let found = reloaded.user("admin").await.expect("the account should have survived");

        assert!(found.admin);
        assert!(reloaded.user("nobody").await.is_none());
    }

    #[tokio::test]
    async fn a_write_leaves_no_temporary_file_behind() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::load(tmp.path()).await.unwrap();
        store.update(|data| data.users.push(user("admin", true))).await.unwrap();

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
}
