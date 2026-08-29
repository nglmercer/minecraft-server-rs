//! Login, logout, password changes, and the current identity endpoint.

use axum::extract::{ConnectInfo, Extension, State};
use axum::http::{header, HeaderMap, HeaderValue};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
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

fn client_key(peer: Option<&Extension<ConnectInfo<SocketAddr>>>) -> String {
    // ConnectInfo is the address observed by the listener. Forwarded headers
    // are intentionally not consulted because no trusted-proxy policy exists.
    peer.map(|Extension(ConnectInfo(address))| address.ip().to_string())
        .unwrap_or_else(|| "unknown".into())
}

async fn login(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    peer: Option<Extension<ConnectInfo<SocketAddr>>>,
    Json(body): Json<LoginRequest>,
) -> ApiResult<Response> {
    // X-Forwarded-For is intentionally not trusted here: unless a trusted
    // proxy is configured, a caller can forge it and evade an IP limiter.
    // Behind a proxy all requests share the proxy address, while the
    // username limiter and global semaphore still provide a second boundary.
    let ip = client_key(peer.as_ref());
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
    append_login_cookies(&mut response, &headers, &token)?;
    Ok(response)
}

fn secure_cookie(headers: &HeaderMap) -> bool {
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
    token: &str,
) -> ApiResult<()> {
    let secure = secure_cookie(headers);
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
async fn logout(State(state): State<Arc<AppState>>, headers: HeaderMap) -> ApiResult<Response> {
    if let Some(token) = token_from_headers(&headers) {
        state.sessions.revoke(&token).await;
    }
    let mut response = Json(serde_json::json!({ "ok": true })).into_response();
    let secure = secure_cookie(&headers);
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
        let first = client_key(Some(&peer("203.0.113.10:50101".parse().unwrap())));
        let second = client_key(Some(&peer("203.0.113.10:50102".parse().unwrap())));
        let other = client_key(Some(&peer("203.0.113.11:50101".parse().unwrap())));

        assert_eq!(first, "203.0.113.10");
        assert_eq!(first, second);
        assert_ne!(first, other);
    }

    #[test]
    fn failed_attempts_on_different_ports_share_the_same_ip_cooldown() {
        let limiter = LoginLimiter::default();
        let first = client_key(Some(&peer("203.0.113.10:50101".parse().unwrap())));
        let second = client_key(Some(&peer("203.0.113.10:50102".parse().unwrap())));
        let third = client_key(Some(&peer("203.0.113.10:50103".parse().unwrap())));

        // Use different usernames so the assertion specifically exercises the
        // IP bucket rather than the username bucket.
        limiter.failure(&first, "alice");
        limiter.failure(&second, "bob");

        assert!(
            limiter.retry_after(&third, "unrelated-user").is_some(),
            "failures from one IP must accumulate across source ports"
        );
    }
}
