//! Password hashing, session tokens and the auth extractor.
//!
//! Sessions are opaque random tokens held in memory. Restarting the panel logs
//! everyone out, which for a self-hosted single-box panel is an acceptable
//! trade for not having to manage a signing key.

use argon2::password_hash::{
    rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString,
};
use argon2::Argon2;
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use rand::RngCore;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

use crate::error::ApiError;
use crate::state::AppState;

/// How long a session survives without being used.
const SESSION_TTL: Duration = Duration::from_secs(60 * 60 * 24 * 7);

/// Hash a password for storage.
pub fn hash_password(password: &str) -> Result<String, argon2::password_hash::Error> {
    let salt = SaltString::generate(&mut OsRng);
    Ok(Argon2::default()
        .hash_password(password.as_bytes(), &salt)?
        .to_string())
}

/// Check a password against a stored PHC hash.
pub fn verify_password(password: &str, hash: &str) -> bool {
    match PasswordHash::new(hash) {
        Ok(parsed) => Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok(),
        Err(_) => false,
    }
}

/// Generate a 256-bit token, hex encoded.
pub fn generate_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// A generated password that is readable enough to retype once.
pub fn generate_password() -> String {
    const ALPHABET: &[u8] = b"abcdefghijkmnpqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let mut bytes = [0u8; 16];
    rand::rng().fill_bytes(&mut bytes);
    bytes
        .iter()
        .map(|b| ALPHABET[*b as usize % ALPHABET.len()] as char)
        .collect()
}

/// A logged-in user, as carried by a session.
#[derive(Debug, Clone)]
pub struct Identity {
    /// Login name.
    pub username: String,
    /// Whether this account may manage servers and users.
    pub admin: bool,
    /// Server ids this account may touch, when not an admin.
    pub servers: Vec<String>,
}

impl Identity {
    /// Whether this identity may act on `server_id`.
    pub fn may_access(&self, server_id: &str) -> bool {
        self.admin || self.servers.iter().any(|s| s == server_id)
    }
}

struct Session {
    identity: Identity,
    last_seen: Instant,
}

/// In-memory session table.
#[derive(Default)]
pub struct Sessions {
    inner: RwLock<HashMap<String, Session>>,
}

impl Sessions {
    /// Issue a token for `identity`.
    pub async fn create(&self, identity: Identity) -> String {
        let token = generate_token();
        let mut sessions = self.inner.write().await;

        // Swept on login rather than on a timer. Expired entries are otherwise
        // only noticed when someone presents them, so a panel that runs for
        // months accumulates sessions nobody will ever resolve again.
        sessions.retain(|_, session| session.last_seen.elapsed() < SESSION_TTL);

        sessions.insert(
            token.clone(),
            Session {
                identity,
                last_seen: Instant::now(),
            },
        );
        token
    }

    /// Resolve a token, refreshing its idle timer.
    pub async fn resolve(&self, token: &str) -> Option<Identity> {
        let mut sessions = self.inner.write().await;
        let session = sessions.get_mut(token)?;
        if session.last_seen.elapsed() > SESSION_TTL {
            sessions.remove(token);
            return None;
        }
        session.last_seen = Instant::now();
        Some(session.identity.clone())
    }

    /// Invalidate a token.
    pub async fn revoke(&self, token: &str) {
        self.inner.write().await.remove(token);
    }

    /// Drop every session belonging to `username`, used when a password changes.
    pub async fn revoke_user(&self, username: &str) {
        self.inner
            .write()
            .await
            .retain(|_, s| s.identity.username != username);
    }
}

/// Pull the bearer token out of the `Authorization` header.
///
/// A `?token=` fallback exists for the WebSocket handshake alone, because
/// browsers cannot set headers on one. It is restricted to that route on
/// purpose: a token in a query string is copied into browser history, into
/// proxy logs, and into this panel's own request log, so it must not become the
/// convenient way to authenticate anything else.
fn extract_token(parts: &Parts) -> Option<String> {
    if let Some(value) = parts.headers.get(axum::http::header::AUTHORIZATION) {
        if let Ok(text) = value.to_str() {
            if let Some(token) = text.strip_prefix("Bearer ") {
                return Some(token.trim().to_string());
            }
        }
    }

    if !parts.uri.path().ends_with("/ws") {
        return None;
    }

    let query = parts.uri.query()?;
    query.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        (key == "token").then(|| value.to_string())
    })
}

impl FromRequestParts<Arc<AppState>> for Identity {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<AppState>,
    ) -> Result<Self, Self::Rejection> {
        let token = extract_token(parts).ok_or(ApiError::Unauthorized)?;
        state
            .sessions
            .resolve(&token)
            .await
            .ok_or(ApiError::Unauthorized)
    }
}

/// An [`Identity`] that additionally proved it is an admin.
#[derive(Debug, Clone)]
pub struct AdminIdentity(pub Identity);

impl FromRequestParts<Arc<AppState>> for AdminIdentity {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<AppState>,
    ) -> Result<Self, Self::Rejection> {
        let identity = Identity::from_request_parts(parts, state).await?;
        if !identity.admin {
            return Err(ApiError::Forbidden);
        }
        Ok(AdminIdentity(identity))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::Request;

    fn identity(name: &str) -> Identity {
        Identity {
            username: name.into(),
            admin: false,
            servers: vec!["allowed".into()],
        }
    }

    #[test]
    fn a_password_verifies_against_its_own_hash_and_nothing_else() {
        let hash = hash_password("correct horse battery").unwrap();

        assert!(verify_password("correct horse battery", &hash));
        assert!(!verify_password("wrong", &hash));
        assert!(!verify_password("", &hash));
    }

    #[test]
    fn hashing_is_salted_so_equal_passwords_differ_on_disk() {
        let a = hash_password("same").unwrap();
        let b = hash_password("same").unwrap();

        assert_ne!(
            a, b,
            "identical hashes would leak that two accounts share a password"
        );
        assert!(verify_password("same", &a));
        assert!(verify_password("same", &b));
    }

    #[test]
    fn a_corrupt_hash_rejects_rather_than_panicking() {
        assert!(!verify_password("anything", "not-a-phc-string"));
        assert!(!verify_password("anything", ""));
    }

    #[test]
    fn tokens_are_long_and_unique() {
        let a = generate_token();
        let b = generate_token();

        assert_eq!(a.len(), 64, "256 bits, hex encoded");
        assert_ne!(a, b);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn generated_passwords_avoid_visually_ambiguous_characters() {
        let password = generate_password();

        assert_eq!(password.len(), 16);
        // Someone has to retype this from a terminal, so no 0/O or 1/l/I.
        assert!(!password.contains(['0', 'O', 'l', 'I', '1']));
    }

    #[test]
    fn non_admins_reach_only_their_own_servers() {
        let user = identity("bob");

        assert!(user.may_access("allowed"));
        assert!(!user.may_access("someone-elses"));

        let admin = Identity {
            admin: true,
            servers: vec![],
            ..identity("root")
        };
        assert!(admin.may_access("anything at all"));
    }

    #[tokio::test]
    async fn sessions_resolve_until_they_are_revoked() {
        let sessions = Sessions::default();
        let token = sessions.create(identity("bob")).await;

        assert_eq!(sessions.resolve(&token).await.unwrap().username, "bob");

        sessions.revoke(&token).await;
        assert!(sessions.resolve(&token).await.is_none());
    }

    #[tokio::test]
    async fn an_unknown_token_never_resolves() {
        let sessions = Sessions::default();
        assert!(sessions.resolve("deadbeef").await.is_none());
        assert!(sessions.resolve("").await.is_none());
    }

    #[tokio::test]
    async fn revoking_a_user_drops_every_one_of_their_sessions() {
        let sessions = Sessions::default();
        let first = sessions.create(identity("bob")).await;
        let second = sessions.create(identity("bob")).await;
        let other = sessions.create(identity("alice")).await;

        sessions.revoke_user("bob").await;

        assert!(sessions.resolve(&first).await.is_none());
        assert!(sessions.resolve(&second).await.is_none());
        assert!(
            sessions.resolve(&other).await.is_some(),
            "alice was not involved"
        );
    }

    fn parts_for(uri: &str, header: Option<&str>) -> axum::http::request::Parts {
        let mut builder = Request::builder().uri(uri);
        if let Some(value) = header {
            builder = builder.header(axum::http::header::AUTHORIZATION, value);
        }
        builder.body(()).unwrap().into_parts().0
    }

    #[test]
    fn the_bearer_header_is_the_primary_source_of_a_token() {
        let parts = parts_for("/api/servers", Some("Bearer abc123"));
        assert_eq!(extract_token(&parts).as_deref(), Some("abc123"));
    }

    #[test]
    fn websockets_may_pass_the_token_in_the_query_string() {
        // Browsers cannot set headers on a WebSocket handshake, so this path
        // has to work — but only as a fallback.
        let parts = parts_for("/api/servers/x/ws?token=abc123", None);
        assert_eq!(extract_token(&parts).as_deref(), Some("abc123"));

        let with_others = parts_for("/api/servers/x/ws?foo=1&token=abc123&bar=2", None);
        assert_eq!(extract_token(&with_others).as_deref(), Some("abc123"));
    }

    #[test]
    fn no_other_route_accepts_a_token_in_the_query_string() {
        // Otherwise every download link would carry a session credential into
        // browser history and the access log.
        for uri in [
            "/api/servers/x/files/download?path=a&token=abc123",
            "/api/servers/x/backups/b/download?token=abc123",
            "/api/servers?token=abc123",
        ] {
            assert!(
                extract_token(&parts_for(uri, None)).is_none(),
                "{uri} should not authenticate from the query string"
            );
        }

        // The header still works on those routes.
        let parts = parts_for(
            "/api/servers/x/files/download?path=a",
            Some("Bearer abc123"),
        );
        assert_eq!(extract_token(&parts).as_deref(), Some("abc123"));
    }

    #[test]
    fn a_request_without_credentials_yields_no_token() {
        assert!(extract_token(&parts_for("/api/servers", None)).is_none());
        // A non-bearer scheme is not a token the panel understands.
        assert!(extract_token(&parts_for("/api/servers", Some("Basic abc"))).is_none());
        assert!(extract_token(&parts_for("/api/servers?tokenish=x", None)).is_none());
    }
}
