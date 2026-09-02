//! Google Drive OAuth2 (user consent) flow.
//!
//! Replaces service-account JSON file upload with a single “Connect Google Drive”
//! button that opens `accounts.google.com`. Tokens are stored at
//! `data/secrets/google-oauth.json` (refresh_token + access_token) with 0600.
//! Service-account JWT remains as fallback when OAuth is not connected.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::backups::secret::SecretStorage;
use crate::error::{ApiError, ApiResult};

const GOOGLE_AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const GOOGLE_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const GOOGLE_DRIVE_SCOPE: &str = "https://www.googleapis.com/auth/drive";
const TOKEN_FILE_REF: &str = "google-oauth";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthTokens {
    pub refresh_token: String,
    #[serde(default)]
    pub access_token: Option<String>,
    #[serde(default)]
    pub expires_at: Option<u64>, // unix secs
    #[serde(default)]
    pub scope: Option<String>,
}

#[derive(Debug, Clone)]
pub struct OAuthPending {
    pub redirect_uri: String,
    pub created_at: Instant,
    pub admin: String,
}

/// Read Google OAuth client credentials from env.
/// Returns None when not configured — endpoint will return 503 with setup instructions.
pub fn client_config() -> Option<(String, String)> {
    let id = std::env::var("MCPANEL_GOOGLE_CLIENT_ID")
        .ok()
        .filter(|s| !s.trim().is_empty());
    let secret = std::env::var("MCPANEL_GOOGLE_CLIENT_SECRET")
        .ok()
        .filter(|s| !s.trim().is_empty());
    match (id, secret) {
        (Some(id), Some(secret)) => Some((id, secret)),
        _ => None,
    }
}

pub fn build_auth_url(client_id: &str, redirect_uri: &str, state: &str) -> String {
    let params = [
        ("client_id", client_id),
        ("redirect_uri", redirect_uri),
        ("response_type", "code"),
        ("scope", GOOGLE_DRIVE_SCOPE),
        ("access_type", "offline"),
        ("prompt", "consent"),
        ("state", state),
        ("include_granted_scopes", "true"),
    ];
    let qs = params
        .iter()
        .map(|(k, v)| format!("{}={}", urlencoding(k), urlencoding(v)))
        .collect::<Vec<_>>()
        .join("&");
    format!("{}?{}", GOOGLE_AUTH_URL, qs)
}

fn urlencoding(s: &str) -> String {
    // Minimal percent-encoding for OAuth params
    let mut out = String::new();
    for b in s.bytes() {
        let c = b as char;
        if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~') {
            out.push(c);
        } else {
            out.push_str(&format!("%{:02X}", b));
        }
    }
    out
}

#[allow(dead_code)]
pub fn token_path(data_dir: &Path) -> PathBuf {
    // SecretStorage path_for will give data/secrets/google-oauth.json
    data_dir.join("secrets").join(format!("{TOKEN_FILE_REF}.json"))
}

pub async fn load_tokens(data_dir: &Path) -> Option<OAuthTokens> {
    let storage = SecretStorage::new(data_dir.to_path_buf());
    // Use path via SecretStorage.exists + read via tokio
    if !storage.exists(TOKEN_FILE_REF).await {
        return None;
    }
    let bytes = storage.read_secret(TOKEN_FILE_REF).await.ok()?;
    serde_json::from_slice::<OAuthTokens>(&bytes).ok()
}

pub async fn save_tokens(data_dir: &Path, tokens: &OAuthTokens) -> Result<()> {
    let storage = SecretStorage::new(data_dir.to_path_buf());
    let bytes = serde_json::to_vec_pretty(tokens)?;
    storage.write_secret(TOKEN_FILE_REF, &bytes).await?;
    Ok(())
}

pub async fn delete_tokens(data_dir: &Path) -> Result<()> {
    let storage = SecretStorage::new(data_dir.to_path_buf());
    storage.delete_secret(TOKEN_FILE_REF).await?;
    Ok(())
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<u64>,
    #[serde(default)]
    scope: Option<String>,
    #[allow(dead_code)]
    #[serde(default)]
    token_type: Option<String>,
}

/// Exchange authorization `code` for tokens.
pub async fn exchange_code(
    code: &str,
    redirect_uri: &str,
    client_id: &str,
    client_secret: &str,
) -> ApiResult<OAuthTokens> {
    let client = reqwest::Client::new();
    let params = [
        ("code", code),
        ("client_id", client_id),
        ("client_secret", client_secret),
        ("redirect_uri", redirect_uri),
        ("grant_type", "authorization_code"),
    ];
    let resp = client
        .post(GOOGLE_TOKEN_URL)
        .form(&params)
        .send()
        .await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("oauth token exchange failed: {e}")))?;
    if !resp.status().is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(ApiError::Internal(anyhow::anyhow!(
            "oauth token exchange failed: {text}"
        )));
    }
    let body: TokenResponse = resp
        .json()
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    let expires_at = body
        .expires_in
        .map(|secs| time::OffsetDateTime::now_utc().unix_timestamp() as u64 + secs);
    Ok(OAuthTokens {
        refresh_token: body
            .refresh_token
            .ok_or_else(|| ApiError::Internal(anyhow::anyhow!("no refresh_token (prompt=consent required)")))?,
        access_token: Some(body.access_token),
        expires_at,
        scope: body.scope,
    })
}

/// Refresh access_token using stored refresh_token.
pub async fn refresh_access_token(
    data_dir: &Path,
    client_id: &str,
    client_secret: &str,
) -> ApiResult<String> {
    let mut tokens = load_tokens(data_dir)
        .await
        .ok_or_else(|| ApiError::Internal(anyhow::anyhow!("not connected")))?;
    // If cached access token still valid (>60s), return it
    if let (Some(at), Some(exp)) = (&tokens.access_token, tokens.expires_at) {
        let now = time::OffsetDateTime::now_utc().unix_timestamp() as u64;
        if now + 60 < exp {
            return Ok(at.clone());
        }
    }
    let client = reqwest::Client::new();
    let params = [
        ("client_id", client_id),
        ("client_secret", client_secret),
        ("refresh_token", tokens.refresh_token.as_str()),
        ("grant_type", "refresh_token"),
    ];
    let resp = client
        .post(GOOGLE_TOKEN_URL)
        .form(&params)
        .send()
        .await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("oauth refresh failed: {e}")))?;
    if !resp.status().is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(ApiError::Internal(anyhow::anyhow!(
            "oauth refresh failed: {text}"
        )));
    }
    let body: TokenResponse = resp
        .json()
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    let expires_at = body
        .expires_in
        .map(|secs| time::OffsetDateTime::now_utc().unix_timestamp() as u64 + secs);
    tokens.access_token = Some(body.access_token.clone());
    tokens.expires_at = expires_at;
    // refresh_token may be omitted on refresh; keep existing
    if let Some(rt) = body.refresh_token {
        tokens.refresh_token = rt;
    }
    save_tokens(data_dir, &tokens)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    Ok(body.access_token)
}

/// Get valid access token, refreshing if needed.
pub async fn get_access_token(data_dir: &Path) -> ApiResult<String> {
    let (client_id, client_secret) =
        client_config().ok_or_else(|| ApiError::Internal(anyhow::anyhow!("oauth not configured")))?;
    refresh_access_token(data_dir, &client_id, &client_secret).await
}

pub async fn is_connected(data_dir: &Path) -> bool {
    load_tokens(data_dir).await.is_some()
}

/// State token generation (32 random hex chars)
pub fn new_state() -> String {
    use rand::Rng;
    let mut rng = rand::rng();
    let bytes: [u8; 16] = rng.random();
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_url_contains_required_params() {
        let url = build_auth_url("cid123", "http://127.0.0.1:8080/callback", "state123");
        assert!(url.contains("client_id=cid123"));
        assert!(url.contains("redirect_uri="));
        assert!(url.contains("scope="));
        assert!(url.contains("state=state123"));
        assert!(url.contains("access_type=offline"));
    }
}
