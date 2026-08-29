//! Login, logout, password changes, and the current identity endpoint.

use axum::extract::{ConnectInfo, Extension, State};
use axum::http::{header, HeaderMap, HeaderValue};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use crate::auth::{
    generate_token, hash_password, token_from_headers, verify_password, Identity, CSRF_COOKIE,
    SESSION_COOKIE,
};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

/// Credentials submitted to the login endpoint.
#[derive(Deserialize)]
pub struct LoginRequest {
    username: String,
    password: String,
}

/// A successful login. The session credential is delivered only as an
/// HttpOnly cookie and is never included in JSON.
#[derive(Serialize)]
pub struct LoginResponse {
    user: UserView,
}

/// The public shape of an account.
#[derive(Serialize)]
pub struct UserView {
    username: String,
    admin: bool,
}

impl From<&Identity> for UserView {
    fn from(identity: &Identity) -> Self {
        UserView {
            username: identity.username.clone(),
            admin: identity.admin,
        }
    }
}

fn client_key(
    peer: Option<&Extension<ConnectInfo<SocketAddr>>>,
    headers: &HeaderMap,
    trusted_proxies: &HashSet<IpAddr>,
) -> String {
    let Some(peer_ip) = peer_ip(peer) else {
        return "unknown".into();
    };

    if trusted_proxies.contains(&peer_ip) {
        if let Some(forwarded) = forwarded_client_ip(headers, trusted_proxies) {
            return forwarded.to_string();
        }
    }
    peer_ip.to_string()
}

fn peer_ip(peer: Option<&Extension<ConnectInfo<SocketAddr>>>) -> Option<IpAddr> {
    peer.map(|Extension(ConnectInfo(address))| address.ip())
}

/// Resolve the nearest non-proxy address from standard proxy headers.
///
/// The immediate TCP peer must already be in the trusted-proxy set before this
/// function is called. Walking from right to left also avoids accepting an
/// address a client prepended when a proxy appends its own address.
fn forwarded_client_ip(headers: &HeaderMap, trusted_proxies: &HashSet<IpAddr>) -> Option<IpAddr> {
    let x_forwarded_for = headers
        .get_all("x-forwarded-for")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .filter_map(parse_ip_token)
        .collect::<Vec<_>>();
    if let Some(address) = nearest_untrusted(x_forwarded_for, trusted_proxies) {
        return Some(address);
    }

    let forwarded = headers
        .get_all("forwarded")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .filter_map(|element| {
            element.split(';').find_map(|parameter| {
                let (name, value) = parameter.split_once('=')?;
                name.trim()
                    .eq_ignore_ascii_case("for")
                    .then(|| parse_ip_token(value))
                    .flatten()
            })
        })
        .collect::<Vec<_>>();
    nearest_untrusted(forwarded, trusted_proxies)
}

fn nearest_untrusted(candidates: Vec<IpAddr>, trusted_proxies: &HashSet<IpAddr>) -> Option<IpAddr> {
    candidates
        .into_iter()
        .rev()
        .find(|address| !trusted_proxies.contains(address))
}

fn parse_ip_token(token: &str) -> Option<IpAddr> {
    let token = token.trim().trim_matches('"');
    if let Some(bracketed) = token.strip_prefix('[') {
        let end = bracketed.find(']')?;
        return bracketed[..end].parse().ok();
    }
    if let Ok(address) = token.parse() {
        return Some(address);
    }
    let (host, port) = token.rsplit_once(':')?;
    port.parse::<u16>().ok()?;
    host.parse().ok()
}

async fn login(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    peer: Option<Extension<ConnectInfo<SocketAddr>>>,
    Json(body): Json<LoginRequest>,
) -> ApiResult<Response> {
    // Forwarded client headers are only considered when the immediate TCP peer
    // is explicitly configured as trusted. The proxy must overwrite, rather
    // than append to, these headers at its public boundary.
    let ip = client_key(peer.as_ref(), &headers, &state.trusted_proxies);
    if state
        .login_limiter
        .retry_after(&ip, &body.username)
        .is_some()
    {
        return Err(ApiError::TooManyRequests);
    }
    let Some(_permit) = state.login_limiter.try_verification() else {
        return Err(ApiError::TooManyRequests);
    };

    let user = state.store.user(&body.username).await;
    let hash = user
        .as_ref()
        .map(|user| user.password_hash.clone())
        .unwrap_or_else(crate::auth::dummy_hash_for_login);
    let password = body.password;
    let valid = tokio::task::spawn_blocking(move || verify_password(&password, &hash))
        .await
        .map_err(|error| {
            ApiError::Internal(anyhow::anyhow!("password verification failed: {error}"))
        })?;

    let Some(user) = user.filter(|_| valid) else {
        state.login_limiter.failure(&ip, &body.username);
        return Err(ApiError::Unauthorized);
    };
    state.login_limiter.success(&ip, &body.username);

    let identity = Identity {
        username: user.username,
        admin: user.admin,
        servers: user.servers,
    };
    let token = state.sessions.create(identity.clone()).await;
    let mut response = (
        axum::http::StatusCode::OK,
        Json(LoginResponse {
            user: UserView::from(&identity),
        }),
    )
        .into_response();
    append_login_cookies(
        &mut response,
        &headers,
        peer.as_ref(),
        &state.trusted_proxies,
        &token,
    )?;
    Ok(response)
}

fn secure_cookie(
    headers: &HeaderMap,
    peer: Option<&Extension<ConnectInfo<SocketAddr>>>,
    trusted_proxies: &HashSet<IpAddr>,
) -> bool {
    if !peer_ip(peer).is_some_and(|address| trusted_proxies.contains(&address)) {
        return false;
    }
    headers
        .get("x-forwarded-proto")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value
                .split(',')
                .next()
                .is_some_and(|scheme| scheme.trim() == "https")
        })
}

fn cookie_header(name: &str, value: &str, http_only: bool, secure: bool) -> ApiResult<HeaderValue> {
    let mut cookie = format!("{name}={value}; Path=/; SameSite=Lax");
    if http_only {
        cookie.push_str("; HttpOnly");
    }
    if secure {
        cookie.push_str("; Secure");
    }
    HeaderValue::from_str(&cookie)
        .map_err(|_| ApiError::Internal(anyhow::anyhow!("failed to construct session cookie")))
}

fn append_login_cookies(
    response: &mut Response,
    headers: &HeaderMap,
    peer: Option<&Extension<ConnectInfo<SocketAddr>>>,
    trusted_proxies: &HashSet<IpAddr>,
    token: &str,
) -> ApiResult<()> {
    let secure = secure_cookie(headers, peer, trusted_proxies);
    let csrf = generate_token();
    response.headers_mut().append(
        header::SET_COOKIE,
        cookie_header(SESSION_COOKIE, token, true, secure)?,
    );
    response.headers_mut().append(
        header::SET_COOKIE,
        cookie_header(CSRF_COOKIE, &csrf, false, secure)?,
    );
    Ok(())
}

async fn me(identity: Identity) -> Json<UserView> {
    Json(UserView::from(&identity))
}

fn clear_cookie(name: &str, secure: bool) -> ApiResult<HeaderValue> {
    cookie_header(name, "", false, secure).map(|value| {
        let mut text = value.to_str().unwrap_or_default().to_owned();
        text.push_str("; Max-Age=0");
        HeaderValue::from_str(&text).expect("the fixed cookie name is valid")
    })
}

/// Revoke the presented session and clear browser credentials.
async fn logout(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    peer: Option<Extension<ConnectInfo<SocketAddr>>>,
) -> ApiResult<Response> {
    if let Some(token) = token_from_headers(&headers) {
        state.sessions.revoke(&token).await;
    }
    let mut response = Json(serde_json::json!({ "ok": true })).into_response();
    let secure = secure_cookie(&headers, peer.as_ref(), &state.trusted_proxies);
    response
        .headers_mut()
        .append(header::SET_COOKIE, clear_cookie(SESSION_COOKIE, secure)?);
    response
        .headers_mut()
        .append(header::SET_COOKIE, clear_cookie(CSRF_COOKIE, secure)?);
    Ok(response)
}

/// Body of the password change endpoint.
#[derive(Deserialize)]
pub struct ChangePassword {
    current: String,
    new: String,
}

async fn change_password(
    State(state): State<Arc<AppState>>,
    identity: Identity,
    Json(body): Json<ChangePassword>,
) -> ApiResult<Json<serde_json::Value>> {
    if body.new.len() < 8 || body.new.len() > 1024 {
        return Err(ApiError::BadRequest(
            "password must be between 8 and 1024 characters".into(),
        ));
    }
    let Some(_permit) = state.login_limiter.try_verification() else {
        return Err(ApiError::TooManyRequests);
    };

    let user = state
        .store
        .user(&identity.username)
        .await
        .ok_or(ApiError::Unauthorized)?;
    let current = body.current;
    let old_hash = user.password_hash;
    let verify_hash = old_hash.clone();
    let valid = tokio::task::spawn_blocking(move || verify_password(&current, &verify_hash))
        .await
        .map_err(|error| {
            ApiError::Internal(anyhow::anyhow!("password verification failed: {error}"))
        })?;
    if !valid {
        return Err(ApiError::Unauthorized);
    }

    let next_password = body.new;
    let hash = tokio::task::spawn_blocking(move || hash_password(&next_password))
        .await
        .map_err(|error| ApiError::Internal(anyhow::anyhow!("password hashing failed: {error}")))?
        .map_err(|error| ApiError::Internal(anyhow::anyhow!("password hashing failed: {error}")))?;
    let username = identity.username.clone();
    let changed = state
        .store
        .update(move |data| {
            let Some(user) = data.users.iter_mut().find(|user| user.username == username) else {
                return false;
            };
            if user.password_hash != old_hash {
                return false;
            }
            user.password_hash = hash;
            true
        })
        .await?;
    if !changed {
        return Err(ApiError::Conflict(
            "password changed concurrently; please retry".into(),
        ));
    }

    state.sessions.revoke_user(&identity.username).await;
    Ok(Json(serde_json::json!({ "ok": true })))
}

/// Routes under the auth prefix.
pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/login", post(login))
        .route("/me", get(me))
        .route("/logout", post(logout))
        .route("/password", post(change_password))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::LoginLimiter;

    fn peer(address: SocketAddr) -> Extension<ConnectInfo<SocketAddr>> {
        Extension(ConnectInfo(address))
    }

    #[test]
    fn limiter_keys_use_only_the_peer_ip() {
        let headers = HeaderMap::new();
        let trusted = HashSet::new();
        let first = client_key(
            Some(&peer("203.0.113.10:50101".parse().unwrap())),
            &headers,
            &trusted,
        );
        let second = client_key(
            Some(&peer("203.0.113.10:50102".parse().unwrap())),
            &headers,
            &trusted,
        );
        let other = client_key(
            Some(&peer("203.0.113.11:50101".parse().unwrap())),
            &headers,
            &trusted,
        );

        assert_eq!(first, "203.0.113.10");
        assert_eq!(first, second);
        assert_ne!(first, other);
    }

    #[test]
    fn failed_attempts_on_different_ports_share_the_same_ip_cooldown() {
        let limiter = LoginLimiter::default();
        let headers = HeaderMap::new();
        let trusted = HashSet::new();
        let first = client_key(
            Some(&peer("203.0.113.10:50101".parse().unwrap())),
            &headers,
            &trusted,
        );
        let second = client_key(
            Some(&peer("203.0.113.10:50102".parse().unwrap())),
            &headers,
            &trusted,
        );
        let third = client_key(
            Some(&peer("203.0.113.10:50103".parse().unwrap())),
            &headers,
            &trusted,
        );

        // Use different usernames so the assertion specifically exercises the
        // IP bucket rather than the username bucket.
        limiter.failure(&first, "alice");
        limiter.failure(&second, "bob");

        assert!(
            limiter.retry_after(&third, "unrelated-user").is_some(),
            "failures from one IP must accumulate across source ports"
        );
    }

    #[test]
    fn trusted_proxy_headers_identify_the_real_client() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            "198.51.100.10, 127.0.0.1".parse().unwrap(),
        );
        let trusted = ["127.0.0.1".parse().unwrap()].into_iter().collect();

        assert_eq!(
            client_key(
                Some(&peer("127.0.0.1:8080".parse().unwrap())),
                &headers,
                &trusted,
            ),
            "198.51.100.10"
        );
    }

    #[test]
    fn untrusted_peers_cannot_forge_forwarded_client_ips() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "198.51.100.10".parse().unwrap());

        assert_eq!(
            client_key(
                Some(&peer("203.0.113.10:8080".parse().unwrap())),
                &headers,
                &HashSet::new(),
            ),
            "203.0.113.10"
        );
    }

    #[test]
    fn forwarded_header_supports_ipv6_and_ports() {
        let mut headers = HeaderMap::new();
        headers.insert("forwarded", "for=\"[2001:db8::10]:443\"".parse().unwrap());
        let trusted = ["127.0.0.1".parse().unwrap()].into_iter().collect();

        assert_eq!(
            client_key(
                Some(&peer("127.0.0.1:8080".parse().unwrap())),
                &headers,
                &trusted,
            ),
            "2001:db8::10"
        );
    }

    #[test]
    fn forwarded_protocol_is_used_only_for_a_trusted_proxy() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-proto", "https".parse().unwrap());
        let trusted = ["127.0.0.1".parse().unwrap()].into_iter().collect();

        assert!(!secure_cookie(
            &headers,
            Some(&peer("203.0.113.10:8080".parse().unwrap())),
            &trusted,
        ));
        assert!(secure_cookie(
            &headers,
            Some(&peer("127.0.0.1:8080".parse().unwrap())),
            &trusted,
        ));
    }
}
