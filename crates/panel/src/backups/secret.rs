//! Secure secret storage for provider credentials.

use anyhow::{Context, Result};
use guardian::ScopedFs;
use std::path::{Path, PathBuf};

/// Directory for secrets: data/secrets with 0700.
fn secrets_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("secrets")
}

pub struct SecretStorage {
    data_dir: PathBuf,
}

impl SecretStorage {
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            data_dir: data_dir.into(),
        }
    }

    /// Ensure secrets directory exists with 0700.
    pub async fn ensure_dir(&self) -> Result<()> {
        let dir = secrets_dir(&self.data_dir);
        tokio::fs::create_dir_all(&dir)
            .await
            .with_context(|| format!("creating {}", dir.display()))?;
        #[cfg(unix)]
        {
            tokio::fs::set_permissions(&dir, std::os::unix::fs::PermissionsExt::from_mode(0o700))
                .await
                .with_context(|| format!("protecting {}", dir.display()))?;
        }
        let dir_clone = dir.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let fs = ScopedFs::open(&dir_clone)
                .with_context(|| format!("opening {}", dir_clone.display()))?;
            fs.set_private()
                .with_context(|| format!("protecting {}", dir_clone.display()))?;
            Ok(())
        })
        .await
        .with_context(|| format!("protecting {}", dir.display()))??;
        Ok(())
    }

    pub fn path_for(&self, credential_ref: &str) -> Result<PathBuf> {
        if credential_ref.is_empty()
            || credential_ref.len() > 64
            || !credential_ref
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            anyhow::bail!("invalid credential_ref");
        }
        Ok(secrets_dir(&self.data_dir).join(format!("{credential_ref}.json")))
    }

    /// Write credential JSON with 0600. `credential_ref` is like "google-drive".
    pub async fn write_secret(&self, credential_ref: &str, content: &[u8]) -> Result<()> {
        self.ensure_dir().await?;
        let path = self.path_for(credential_ref)?;
        let content = content.to_owned();
        let dir = secrets_dir(&self.data_dir);
        let cref = credential_ref.to_owned();
        let path_for_err = path.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let fs = ScopedFs::open(&dir).with_context(|| format!("opening {}", dir.display()))?;
            // ScopedFs::write_atomic will set 0600 via set_private on file
            fs.write_atomic(format!("{cref}.json"), &content)
                .with_context(|| format!("writing {}", path_for_err.display()))?;
            // Ensure 0600
            fs.set_file_private(&format!("{cref}.json"))
                .with_context(|| format!("protecting {}", path_for_err.display()))?;
            Ok(())
        })
        .await
        .with_context(|| format!("writing secret {}", credential_ref))??;
        Ok(())
    }

    pub async fn read_secret(&self, credential_ref: &str) -> Result<Vec<u8>> {
        let path = self.path_for(credential_ref)?;
        let dir = secrets_dir(&self.data_dir);
        let cref = credential_ref.to_owned();
        let path_for_err = path.clone();
        let bytes = tokio::task::spawn_blocking(move || -> Result<Vec<u8>> {
            let fs = ScopedFs::open(&dir).with_context(|| format!("opening {}", dir.display()))?;
            let bytes = fs
                .read_file(&format!("{cref}.json"))
                .with_context(|| format!("reading {}", path_for_err.display()))?;
            Ok(bytes)
        })
        .await
        .with_context(|| format!("reading secret {}", credential_ref))??;
        Ok(bytes)
    }

    /// Check if secret exists without exposing contents.
    pub async fn exists(&self, credential_ref: &str) -> bool {
        if let Ok(path) = self.path_for(credential_ref) {
            tokio::fs::metadata(&path).await.is_ok()
        } else {
            false
        }
    }

    pub async fn delete_secret(&self, credential_ref: &str) -> Result<()> {
        let path = self.path_for(credential_ref)?;
        let dir = secrets_dir(&self.data_dir);
        let cref = credential_ref.to_owned();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let fs = ScopedFs::open(&dir).with_context(|| format!("opening {}", dir.display()))?;
            let _ = fs.remove(format!("{cref}.json"));
            Ok(())
        })
        .await
        .with_context(|| format!("deleting secret {}", credential_ref))??;
        let _ = tokio::fs::remove_file(&path).await;
        Ok(())
    }
}
