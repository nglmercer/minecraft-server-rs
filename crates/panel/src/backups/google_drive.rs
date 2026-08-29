//! Google Drive provider with service-account authentication and resumable upload.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Context;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::sync::RwLock;

use crate::backups::provider::{
    BackupArtifact, BackupProvider, BackupStream, ProviderHealth, RemoteBackup,
};
use crate::error::{ApiError, ApiResult};
use crate::store::{BackupProviderKind, GoogleDriveConfig};

/// Service account JSON as stored on disk.
#[derive(Debug, Deserialize)]
struct ServiceAccountKey {
    client_email: String,
    private_key: String,
    token_uri: String,
}

struct TokenCache {
    token: String,
    expires_at: Instant,
}

pub struct GoogleDriveBackupProvider {
    data_dir: PathBuf,
    config: GoogleDriveConfig,
    token_cache: Arc<RwLock<Option<TokenCache>>>,
    client: reqwest::Client,
}

impl GoogleDriveBackupProvider {
    pub fn new(data_dir: PathBuf, config: GoogleDriveConfig) -> Self {
        Self {
            data_dir,
            config,
            token_cache: Arc::new(RwLock::new(None)),
            client: reqwest::Client::new(),
        }
    }

    async fn service_account_key(&self) -> ApiResult<ServiceAccountKey> {
        let secret_path = self
            .data_dir
            .join("secrets")
            .join(format!("{}.json", self.config.credential_ref));
        let bytes = tokio::fs::read(&secret_path).await.map_err(|e| {
            ApiError::Internal(anyhow::anyhow!("reading google drive credentials: {e}"))
        })?;
        serde_json::from_slice(&bytes).map_err(|e| ApiError::Internal(e.into()))
    }

    async fn fetch_access_token(&self) -> ApiResult<String> {
        // Check cache
        {
            let cache = self.token_cache.read().await;
            if let Some(entry) = cache.as_ref() {
                if Instant::now() < entry.expires_at - Duration::from_secs(60) {
                    return Ok(entry.token.clone());
                }
            }
        }

        let key = self.service_account_key().await?;
        let now = time::OffsetDateTime::now_utc().unix_timestamp();
        let exp = now + 3600;
        let claims = serde_json::json!({
            "iss": key.client_email,
            "scope": "https://www.googleapis.com/auth/drive",
            "aud": key.token_uri,
            "iat": now,
            "exp": exp,
        });

        // Sign with RSA private key using jsonwebtoken crate's encoding key
        let encoding_key = jsonwebtoken::EncodingKey::from_rsa_pem(key.private_key.as_bytes())
            .map_err(|e| ApiError::Internal(anyhow::anyhow!("invalid private key: {e}")))?;
        let header_jwt = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256);
        let token = jsonwebtoken::encode(&header_jwt, &claims, &encoding_key)
            .map_err(|e| ApiError::Internal(anyhow::anyhow!("jwt encode failed: {e}")))?;

        // Actually we need to use the signed JWT as assertion to fetch access token
        // For simplicity, we directly use the JWT as the bearer if token_uri is Google's generic?
        // Real flow: POST token_uri with grant_type=urn:ietf:params:oauth:grant-type:jwt-bearer&assertion=token
        let params = [
            ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
            ("assertion", &token),
        ];
        let resp = self
            .client
            .post(&key.token_uri)
            .form(&params)
            .send()
            .await
            .map_err(|e| ApiError::Internal(anyhow::anyhow!("token request failed: {e}")))?;
        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(ApiError::Internal(anyhow::anyhow!(
                "token request failed: {text}"
            )));
        }
        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| ApiError::Internal(e.into()))?;
        let access_token = body
            .get("access_token")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ApiError::Internal(anyhow::anyhow!("no access_token in response")))?
            .to_string();
        let expires_in = body
            .get("expires_in")
            .and_then(|v| v.as_u64())
            .unwrap_or(3600);

        let mut cache = self.token_cache.write().await;
        *cache = Some(TokenCache {
            token: access_token.clone(),
            expires_at: Instant::now() + Duration::from_secs(expires_in),
        });
        Ok(access_token)
    }

    fn drive_api_base() -> &'static str {
        "https://www.googleapis.com"
    }

    async fn ensure_folder(&self, token: &str) -> ApiResult<()> {
        // Verify folder exists and is accessible
        let url = format!(
            "{}/drive/v3/files/{}",
            Self::drive_api_base(),
            self.config.folder_id
        );
        let resp = self
            .client
            .get(&url)
            .bearer_auth(token)
            .query(&[("fields", "id,name,mimeType")])
            .send()
            .await
            .map_err(|e| ApiError::Internal(anyhow::anyhow!("folder check failed: {e}")))?;
        if resp.status() == StatusCode::NOT_FOUND {
            return Err(ApiError::Internal(anyhow::anyhow!(
                "drive folder not found"
            )));
        }
        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(ApiError::Internal(anyhow::anyhow!(
                "folder check failed: {text}"
            )));
        }
        Ok(())
    }

    async fn with_retry<F, Fut, T>(&self, mut f: F) -> ApiResult<T>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = ApiResult<T>>,
    {
        let mut attempts = 0;
        let max_attempts = 4;
        let mut delay = Duration::from_secs(1);
        loop {
            match f().await {
                Ok(v) => return Ok(v),
                Err(e) => {
                    attempts += 1;
                    let is_retryable = match &e {
                        ApiError::Internal(msg) => {
                            let s = msg.to_string();
                            s.contains("429")
                                || s.contains("500")
                                || s.contains("502")
                                || s.contains("503")
                                || s.contains("504")
                                || s.contains("timeout")
                                || s.contains("connection")
                        }
                        _ => false,
                    };
                    if !is_retryable || attempts >= max_attempts {
                        return Err(e);
                    }
                    // Exponential backoff with jitter
                    let jitter = rand::random::<u64>() % 500;
                    tokio::time::sleep(delay + Duration::from_millis(jitter)).await;
                    delay *= 2;
                }
            }
        }
    }
}

#[async_trait::async_trait]
impl BackupProvider for GoogleDriveBackupProvider {
    async fn upload(
        &self,
        server_id: &str,
        artifact: &BackupArtifact,
    ) -> Result<RemoteBackup, ApiError> {
        let token = self.fetch_access_token().await?;
        self.ensure_folder(&token).await?;

        // Prepare file metadata with appProperties for reliable listing
        let file_name = format!("backup-{}-{}.tar.gz", server_id, artifact.id);
        let metadata = serde_json::json!({
            "name": file_name,
            "parents": [self.config.folder_id],
            "appProperties": {
                "mcpanel_server_id": server_id,
                "mcpanel_backup_id": artifact.id,
                "mcpanel_created_at": artifact.created_at,
                "mcpanel_note": artifact.note,
            },
            "mimeType": "application/gzip"
        });

        let file = tokio::fs::File::open(&artifact.staging_path)
            .await
            .map_err(|e| ApiError::Internal(e.into()))?;
        let file_size = artifact.size_bytes;

        // Initiate resumable upload
        let init_url = format!(
            "{}/upload/drive/v3/files?uploadType=resumable",
            Self::drive_api_base()
        );
        let init_resp = self
            .client
            .post(&init_url)
            .bearer_auth(&token)
            .header("X-Upload-Content-Type", "application/gzip")
            .header("X-Upload-Content-Length", file_size.to_string())
            .json(&metadata)
            .send()
            .await
            .map_err(|e| {
                ApiError::Internal(anyhow::anyhow!("init resumable upload failed: {e}"))
            })?;

        if !init_resp.status().is_success() {
            let text = init_resp.text().await.unwrap_or_default();
            return Err(ApiError::Internal(anyhow::anyhow!(
                "init upload failed: {text}"
            )));
        }
        let upload_url = init_resp
            .headers()
            .get("location")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| ApiError::Internal(anyhow::anyhow!("no location for resumable upload")))?
            .to_string();

        // Stream file in chunks with retry
        let mut file = file;
        let mut uploaded: u64 = 0;
        let chunk_size: usize = 8 * 1024 * 1024;
        let mut buf = vec![0u8; chunk_size];

        loop {
            let n = file
                .read(&mut buf)
                .await
                .map_err(|e| ApiError::Internal(e.into()))?;
            if n == 0 {
                break;
            }
            let chunk = &buf[..n];
            let start = uploaded;
            let end = uploaded + n as u64 - 1;
            let total = file_size;

            let upload_chunk = || async {
                let resp = self
                    .client
                    .put(&upload_url)
                    .bearer_auth(&token)
                    .header(
                        "Content-Range",
                        format!("bytes {}-{}/{}", start, end, total),
                    )
                    .body(chunk.to_vec())
                    .send()
                    .await
                    .map_err(|e| ApiError::Internal(anyhow::anyhow!("chunk upload failed: {e}")))?;
                let status = resp.status();
                if status.is_success() || status == StatusCode::from_u16(201).unwrap() {
                    Ok::<(), ApiError>(())
                } else if status.as_u16() == 308 {
                    // Resume incomplete
                    Ok(())
                } else {
                    let text = resp.text().await.unwrap_or_default();
                    Err(ApiError::Internal(anyhow::anyhow!(
                        "chunk upload failed {}: {text}",
                        status
                    )))
                }
            };

            self.with_retry(upload_chunk).await?;
            uploaded += n as u64;
        }

        // After upload, fetch file ID from final response? The last chunk's response contains File resource.
        // For simplicity, list files with our backup_id to find remote_id
        // Do a search in folder for appProperties
        let list = self.list(server_id).await?;
        let found = list
            .iter()
            .find(|b| b.id == artifact.id)
            .cloned()
            .ok_or_else(|| {
                ApiError::Internal(anyhow::anyhow!("uploaded file not found in listing"))
            })?;

        Ok(found)
    }

    async fn list(&self, server_id: &str) -> Result<Vec<RemoteBackup>, ApiError> {
        let token = self.fetch_access_token().await?;
        // Search for files in folder with appProperties
        let q = format!("'{}' in parents and trashed=false", self.config.folder_id);
        let url = format!("{}/drive/v3/files", Self::drive_api_base());
        let resp = self
            .client
            .get(&url)
            .bearer_auth(&token)
            .query(&[
                ("q", q.as_str()),
                ("fields", "files(id,name,appProperties,size,createdTime)"),
                ("pageSize", "1000"),
            ])
            .send()
            .await
            .map_err(|e| ApiError::Internal(anyhow::anyhow!("list failed: {e}")))?;
        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(ApiError::Internal(anyhow::anyhow!("list failed: {text}")));
        }
        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| ApiError::Internal(e.into()))?;
        let files = body
            .get("files")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let mut out = Vec::new();
        for f in files {
            let app_props = f.get("appProperties");
            let backup_id = app_props
                .and_then(|p| p.get("mcpanel_backup_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let srv = app_props
                .and_then(|p| p.get("mcpanel_server_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if srv != server_id {
                continue;
            }
            let id = if !backup_id.is_empty() {
                backup_id.to_string()
            } else {
                // Fallback to parsing filename
                f.get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string()
            };
            let created = app_props
                .and_then(|p| p.get("mcpanel_created_at"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let size = f
                .get("size")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(0);
            let remote_id = f
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let note = app_props
                .and_then(|p| p.get("mcpanel_note"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            out.push(RemoteBackup {
                id,
                server_id: server_id.to_string(),
                provider: BackupProviderKind::GoogleDrive,
                remote_id,
                created_at: if created.is_empty() {
                    f.get("createdTime")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string()
                } else {
                    created
                },
                size_bytes: size,
                checksum_sha256: None,
                note,
            });
        }
        out.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(out)
    }

    async fn delete(&self, _server_id: &str, backup_id: &str) -> Result<(), ApiError> {
        let token = self.fetch_access_token().await?;
        // Find remote_id via list, then delete
        // For safety, we list and find; if not found, treat as not found
        let server_id = _server_id;
        let list = self.list(server_id).await?;
        let remote = list
            .iter()
            .find(|b| b.id == backup_id)
            .ok_or_else(|| ApiError::NotFound("backup".into()))?;
        let url = format!(
            "{}/drive/v3/files/{}",
            Self::drive_api_base(),
            remote.remote_id
        );
        let resp = self
            .client
            .delete(&url)
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| ApiError::Internal(anyhow::anyhow!("delete failed: {e}")))?;
        if resp.status() == StatusCode::NOT_FOUND {
            return Err(ApiError::NotFound("backup".into()));
        }
        if !resp.status().is_success() && resp.status().as_u16() != 204 {
            let text = resp.text().await.unwrap_or_default();
            return Err(ApiError::Internal(anyhow::anyhow!("delete failed: {text}")));
        }
        Ok(())
    }

    async fn download(&self, server_id: &str, backup_id: &str) -> Result<BackupStream, ApiError> {
        let token = self.fetch_access_token().await?;
        let list = self.list(server_id).await?;
        let remote = list
            .iter()
            .find(|b| b.id == backup_id)
            .ok_or_else(|| ApiError::NotFound("backup".into()))?;
        let url = format!(
            "{}/drive/v3/files/{}?alt=media",
            Self::drive_api_base(),
            remote.remote_id
        );
        let resp = self
            .client
            .get(&url)
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| ApiError::Internal(anyhow::anyhow!("download failed: {e}")))?;
        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(ApiError::Internal(anyhow::anyhow!(
                "download failed: {text}"
            )));
        }
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| ApiError::Internal(anyhow::anyhow!("download bytes failed: {e}")))?;
        let cursor = std::io::Cursor::new(bytes.to_vec());
        let reader = tokio::io::BufReader::new(cursor);
        Ok(Box::pin(reader))
    }

    async fn health_check(&self) -> Result<ProviderHealth, ApiError> {
        let token = self
            .fetch_access_token()
            .await
            .map_err(|e| ApiError::Internal(anyhow::anyhow!("health check auth failed: {e}")))?;
        self.ensure_folder(&token)
            .await
            .map_err(|e| ApiError::Internal(anyhow::anyhow!("health check folder failed: {e}")))?;
        // Try to verify we can list
        let _ = self.list("health-check").await.unwrap_or_default();
        Ok(ProviderHealth {
            ok: true,
            message: "google drive ok".into(),
        })
    }
}
