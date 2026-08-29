//! Password hashing, session tokens and the auth extractor.
//!
//! Sessions are opaque random tokens held in memory. Restarting the panel logs
//! everyone out, which for a self-hosted single-box panel is an acceptable
//! trade for not having to manage a signing key.

use argon2::password_hash::{
    rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString,
};
use argon2::Argon2;
use axum::body::Body;
use axum::extract::FromRequestParts;
use axum::http::{header, request::Parts, Method, Request};
use axum::middleware::Next;
use axum::response::Response;
use rand::RngCore;
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use tokio::sync::{OwnedSemaphorePermit, RwLock, Semaphore};

use crate::error::ApiError;
use crate::state::AppState;

/// How long a session survives without being used.
const SESSION_TTL: Duration = Duration::from_secs(60 * 60 * 24 * 7);
const MAX_SESSIONS: usize = 4096;
const MAX_LOGIN_KEYS: usize = 4096;
const LOGIN_KEY_TTL: Duration = Duration::from_secs(60 * 30);
const MAX_VERIFICATIONS: usize = 8;
pub(crate) const SESSION_COOKIE: &str = "mcpanel_session";
pub(crate) const CSRF_COOKIE: &str = "mcpanel_csrf";

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

#[derive(Debug)]
struct LoginAttempt {
    failures: u32,
    blocked_until: Instant,
    last_seen: Instant,
}

/// Bounded, in-memory login abuse controls.
pub struct LoginLimiter {
    attempts: std::sync::Mutex<HashMap<String, LoginAttempt>>,
    verifier: Arc<Semaphore>,
}

impl Default for LoginLimiter {
    fn default() -> Self {
        let parallelism = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(2)
            .clamp(1, MAX_VERIFICATIONS);
        Self {
            attempts: std::sync::Mutex::new(HashMap::new()),
            verifier: Arc::new(Semaphore::new(parallelism)),
        }
    }
}

impl LoginLimiter {
    fn keys(ip: &str, username: &str) -> [String; 2] {
        let mut hasher = DefaultHasher::new();
        username.hash(&mut hasher);
        [
            format!("ip:{}", ip.chars().take(128).collect::<String>()),
            format!("user:{:016x}", hasher.finish()),
        ]
    }

    /// Return a retry duration when either the IP or username is cooling down.
    pub fn retry_after(&self, ip: &str, username: &str) -> Option<Duration> {
        let now = Instant::now();
        let keys = Self::keys(ip, username);
        let mut attempts = self.attempts.lock().ok()?;
        attempts.retain(|_, attempt| now.duration_since(attempt.last_seen) < LOGIN_KEY_TTL);
        keys.iter()
            .filter_map(|key| attempts.get(key))
            .filter_map(|attempt| attempt.blocked_until.checked_duration_since(now))
            .max()
    }

    /// Acquire one of the small number of Argon2 workers without queuing an
    /// unbounded attacker-controlled number of expensive requests.
    pub fn try_verification(&self) -> Option<OwnedSemaphorePermit> {
        Arc::clone(&self.verifier).try_acquire_owned().ok()
    }

    pub fn failure(&self, ip: &str, username: &str) {
        let now = Instant::now();
        let keys = Self::keys(ip, username);
        let Ok(mut attempts) = self.attempts.lock() else {
            return;
        };
        attempts.retain(|_, attempt| now.duration_since(attempt.last_seen) < LOGIN_KEY_TTL);
        for key in keys {
            if attempts.len() >= MAX_LOGIN_KEYS && !attempts.contains_key(&key) {
                if let Some(oldest) = attempts
                    .iter()
                    .min_by_key(|(_, attempt)| attempt.last_seen)
                    .map(|(key, _)| key.clone())
                {
                    attempts.remove(&oldest);
                }
            }
            let attempt = attempts.entry(key).or_insert(LoginAttempt {
                failures: 0,
                blocked_until: now,
                last_seen: now,
            });
            attempt.failures = attempt.failures.saturating_add(1);
            let seconds = if attempt.failures < 2 {
                0
            } else {
                1_u64 << attempt.failures.saturating_sub(2).min(5)
            };
            attempt.blocked_until = now + Duration::from_secs(seconds);
            attempt.last_seen = now;
        }
    }

    pub fn success(&self, ip: &str, username: &str) {
        if let Ok(mut attempts) = self.attempts.lock() {
            for key in Self::keys(ip, username) {
                attempts.remove(&key);
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn tracked_keys(&self) -> usize {
        self.attempts
            .lock()
            .map(|attempts| attempts.len())
            .unwrap_or(0)
    }
}

pub(crate) fn dummy_hash_for_login() -> String {
    static HASH: OnceLock<String> = OnceLock::new();
    HASH.get_or_init(|| {
        hash_password("mcpanel-invalid-login-dummy")
            .expect("the built-in dummy password hash must be constructible")
    })
    .clone()
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
        while sessions.len() >= MAX_SESSIONS {
            let Some(oldest) = sessions
                .iter()
                .min_by_key(|(_, session)| session.last_seen)
                .map(|(token, _)| token.clone())
            else {
                break;
            };
            sessions.remove(&oldest);
        }

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

/// Read credentials from the Authorization header or the session cookie.
///
/// WebSocket handshakes use short-lived scoped tickets, not session tokens.
pub(crate) fn cookie_value(headers: &axum::http::HeaderMap, name: &str) -> Option<String> {
    headers
        .get(header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .filter_map(|part| part.trim().split_once('='))
        .find_map(|(key, value)| (key == name).then(|| value.to_string()))
}

/// Pull the session token from the bearer header or HttpOnly session cookie.
pub(crate) fn token_from_headers(headers: &axum::http::HeaderMap) -> Option<String> {
    if let Some(value) = headers.get(header::AUTHORIZATION) {
        if let Ok(text) = value.to_str() {
            if let Some(token) = text.strip_prefix("Bearer ") {
                return Some(token.trim().to_string());
            }
        }
    }
    cookie_value(headers, SESSION_COOKIE)
}

fn extract_token(parts: &Parts) -> Option<String> {
    token_from_headers(&parts.headers)
}

fn csrf_valid(headers: &axum::http::HeaderMap) -> bool {
    let Some(cookie) = cookie_value(headers, CSRF_COOKIE) else {
        return false;
    };
    let Some(header_value) = headers
        .get("x-csrf-token")
        .and_then(|value| value.to_str().ok())
    else {
        return false;
    };
    use subtle::ConstantTimeEq;
    cookie.as_bytes().ct_eq(header_value.as_bytes()).into()
}

/// Require a CSRF token when a state-changing request is authenticated by a
/// browser cookie. Bearer-only API clients remain stateless and do not need a
/// cookie CSRF token.
pub(crate) async fn csrf_protect(request: Request<Body>, next: Next) -> Result<Response, ApiError> {
    let safe_method = matches!(
        request.method(),
        &Method::GET | &Method::HEAD | &Method::OPTIONS | &Method::TRACE
    );
    let is_login = request.uri().path() == "/api/auth/login";
    let has_bearer = request.headers().contains_key(header::AUTHORIZATION);
    let has_session_cookie = cookie_value(request.headers(), SESSION_COOKIE).is_some();
    if !safe_method
        && !is_login
        && has_session_cookie
        && !has_bearer
        && !csrf_valid(request.headers())
    {
        return Err(ApiError::Forbidden);
    }
    Ok(next.run(request).await)
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
    fn query_string_tokens_are_never_authentication_credentials() {
        // WebSockets use a short-lived, one-use ticket. Long-lived session
        // tokens must never be accepted from a URL.
        let parts = parts_for("/api/servers/x/ws?token=abc123", None);
        assert!(extract_token(&parts).is_none());

        let with_others = parts_for("/api/servers/x/ws?foo=1&token=abc123&bar=2", None);
        assert!(extract_token(&with_others).is_none());
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

    #[test]
    fn login_keys_bound_unicode_ip_input_without_panicking() {
        let limiter = LoginLimiter::default();
        limiter.failure(&"é".repeat(200), "alice");
        assert!(limiter.tracked_keys() <= 2);
    }

    #[test]
    fn repeated_failures_activate_a_cooldown_and_success_clears_it() {
        let limiter = LoginLimiter::default();
        limiter.failure("127.0.0.1", "alice");
        assert!(limiter.retry_after("127.0.0.1", "alice").is_none());
        limiter.failure("127.0.0.1", "alice");
        assert!(limiter.retry_after("127.0.0.1", "alice").is_some());

        limiter.success("127.0.0.1", "alice");
        assert!(limiter.retry_after("127.0.0.1", "alice").is_none());
    }
}
