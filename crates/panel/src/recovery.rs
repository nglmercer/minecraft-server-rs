//! Short-lived, single-use recovery tokens for local password reset.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use rand::RngCore;
use sha2::{Digest, Sha256};

const TTL: Duration = Duration::from_secs(600); // ~10 minutes
const MAX_TOKENS: usize = 32;

/// One recovery capability.
struct Entry {
    username: String,
    created: Instant,
    expires: Instant,
    consumed: bool,
}

/// In-memory recovery token store.
pub struct RecoveryManager {
    inner: Mutex<HashMap<String, Entry>>, // key = sha256 hex of token
}

impl Default for RecoveryManager {
    fn default() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
        }
    }
}

fn hash_token(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    hex::encode(hasher.finalize())
}

// Simple hex encode without extra dep? Use manual.
mod hex {
    pub fn encode(bytes: impl AsRef<[u8]>) -> String {
        bytes.as_ref().iter().map(|b| format!("{b:02x}")).collect()
    }
}

fn generate_raw_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

impl RecoveryManager {
    /// Generate a new recovery token for `username`. Returns raw token.
    pub fn generate(&self, username: String) -> String {
        let token = generate_raw_token();
        let hash = hash_token(&token);
        let now = Instant::now();
        if let Ok(mut map) = self.inner.lock() {
            // purge expired/consumed
            map.retain(|_, e| !e.consumed && e.expires > now);
            while map.len() >= MAX_TOKENS {
                if let Some(oldest) = map
                    .iter()
                    .min_by_key(|(_, e)| e.created)
                    .map(|(k, _)| k.clone())
                {
                    map.remove(&oldest);
                } else {
                    break;
                }
            }
            map.insert(
                hash,
                Entry {
                    username,
                    created: now,
                    expires: now + TTL,
                    consumed: false,
                },
            );
        }
        token
    }

    /// Validate token and return associated username if valid, without consuming.
    #[cfg(test)]
    pub fn validate(&self, token: &str) -> Option<String> {
        let hash = hash_token(token);
        let map = self.inner.lock().ok()?;
        let entry = map.get(&hash)?;
        if entry.consumed || entry.expires <= Instant::now() {
            return None;
        }
        Some(entry.username.clone())
    }

    /// Consume token: validates and marks as used. Returns username on success.
    pub fn consume(&self, token: &str) -> Option<String> {
        let hash = hash_token(token);
        let mut map = self.inner.lock().ok()?;
        let entry = map.get_mut(&hash)?;
        if entry.consumed || entry.expires <= Instant::now() {
            return None;
        }
        entry.consumed = true;
        let username = entry.username.clone();
        // Remove immediately to enforce single-use; keep consumed flag for clarity
        // but we remove to free slot.
        map.remove(&hash);
        Some(username)
    }

    /// Invalidate all tokens for username (called after successful reset optional).
    pub fn invalidate_user(&self, username: &str) {
        if let Ok(mut map) = self.inner.lock() {
            map.retain(|_, e| e.username != username);
        }
    }

    #[cfg(test)]
    pub fn force_expire(&self, token: &str) {
        let hash = hash_token(token);
        if let Ok(mut map) = self.inner.lock() {
            if let Some(e) = map.get_mut(&hash) {
                e.expires = Instant::now() - Duration::from_secs(1);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_and_validates() {
        let m = RecoveryManager::default();
        let token = m.generate("admin".into());
        assert_eq!(token.len(), 64);
        assert_eq!(m.validate(&token).as_deref(), Some("admin"));
    }

    #[test]
    fn consume_is_single_use() {
        let m = RecoveryManager::default();
        let token = m.generate("admin".into());
        assert!(m.consume(&token).is_some());
        assert!(m.validate(&token).is_none());
        assert!(m.consume(&token).is_none());
    }

    #[test]
    fn expired_token_rejected() {
        let m = RecoveryManager::default();
        let token = m.generate("admin".into());
        m.force_expire(&token);
        assert!(m.validate(&token).is_none());
    }
}
