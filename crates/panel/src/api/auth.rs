//! Login, logout and "who am I".

use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::auth::{hash_password, verify_password, Identity};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

/// Credentials submitted to `POST /api/auth/login`.
#[derive(Deserialize)]
pub struct LoginRequest {
    username: String,
    password: String,
}

/// A successful login.
#[derive(Serialize)]
pub struct LoginResponse {
    token: String,
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

async fn login(
    State(state): State<Arc<AppState>>,
    Json(body): Json<LoginRequest>,
) -> ApiResult<Json<LoginResponse>> {
    let user = state
        .store
        .user(&body.username)
        .await
        .ok_or(ApiError::Unauthorized)?;

    if !verify_password(&body.password, &user.password_hash) {
        return Err(ApiError::Unauthorized);
    }

    let identity = Identity {
        username: user.username,
        admin: user.admin,
        servers: user.servers,
    };
    let token = state.sessions.create(identity.clone()).await;

    Ok(Json(LoginResponse {
        token,
        user: UserView::from(&identity),
    }))
}

async fn me(identity: Identity) -> Json<UserView> {
    Json(UserView::from(&identity))
}

/// Invalidate the token this request was made with.
async fn logout(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
) -> Json<serde_json::Value> {
    if let Some(token) = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
    {
        state.sessions.revoke(token.trim()).await;
    }
    Json(serde_json::json!({ "ok": true }))
}

/// Body of `POST /api/auth/password`.
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
    if body.new.len() < 8 {
        return Err(ApiError::BadRequest(
            "password must be at least 8 characters".into(),
        ));
    }

    let user = state
        .store
        .user(&identity.username)
        .await
        .ok_or(ApiError::Unauthorized)?;
    if !verify_password(&body.current, &user.password_hash) {
        return Err(ApiError::Unauthorized);
    }

    let hash = hash_password(&body.new)
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("hashing failed: {e}")))?;

    state
        .store
        .update(|data| {
            if let Some(u) = data
                .users
                .iter_mut()
                .find(|u| u.username == identity.username)
            {
                u.password_hash = hash;
            }
        })
        .await?;

    // Force a fresh login everywhere, including whoever may have stolen a token.
    state.sessions.revoke_user(&identity.username).await;

    Ok(Json(serde_json::json!({ "ok": true })))
}

/// Routes under `/api/auth`.
pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/login", post(login))
        .route("/me", get(me))
        .route("/logout", post(logout))
        .route("/password", post(change_password))
}
