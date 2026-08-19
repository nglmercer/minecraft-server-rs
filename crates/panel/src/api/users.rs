//! Account administration. Every route here is admin-only.

use axum::extract::{Path, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::auth::{hash_password, AdminIdentity};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use crate::store::User;

/// An account, without its password hash.
#[derive(Serialize)]
pub struct UserView {
    username: String,
    admin: bool,
    servers: Vec<String>,
}

impl From<&User> for UserView {
    fn from(user: &User) -> Self {
        UserView {
            username: user.username.clone(),
            admin: user.admin,
            servers: user.servers.clone(),
        }
    }
}

async fn list(
    State(state): State<Arc<AppState>>,
    AdminIdentity(_): AdminIdentity,
) -> ApiResult<Json<Vec<UserView>>> {
    let users = state.store.read().await.users;
    Ok(Json(users.iter().map(UserView::from).collect()))
}

/// Body of `POST /api/users`.
#[derive(Deserialize)]
pub struct CreateUser {
    username: String,
    password: String,
    #[serde(default)]
    admin: bool,
    /// Server ids this account may access. Ignored when `admin` is true.
    #[serde(default)]
    servers: Vec<String>,
}

async fn create(
    State(state): State<Arc<AppState>>,
    AdminIdentity(_): AdminIdentity,
    Json(body): Json<CreateUser>,
) -> ApiResult<Json<UserView>> {
    let username = body.username.trim().to_string();
    if username.is_empty() {
        return Err(ApiError::BadRequest("username is required".into()));
    }
    if body.password.len() < 8 {
        return Err(ApiError::BadRequest(
            "password must be at least 8 characters".into(),
        ));
    }
    if state.store.user(&username).await.is_some() {
        return Err(ApiError::BadRequest(format!("{username} already exists")));
    }

    let user = User {
        username: username.clone(),
        password_hash: hash_password(&body.password)
            .map_err(|e| ApiError::Internal(anyhow::anyhow!("hashing failed: {e}")))?,
        admin: body.admin,
        servers: body.servers,
    };

    let stored = user.clone();
    state
        .store
        .update(move |data| data.users.push(stored))
        .await?;
    tracing::info!(user = %username, "account created");

    Ok(Json(UserView::from(&user)))
}

/// Body of `PATCH /api/users/{username}`. Every field is optional.
#[derive(Deserialize)]
pub struct UpdateUser {
    password: Option<String>,
    admin: Option<bool>,
    servers: Option<Vec<String>>,
}

async fn update(
    State(state): State<Arc<AppState>>,
    AdminIdentity(admin): AdminIdentity,
    Path(username): Path<String>,
    Json(body): Json<UpdateUser>,
) -> ApiResult<Json<UserView>> {
    let mut user = state
        .store
        .user(&username)
        .await
        .ok_or_else(|| ApiError::NotFound(format!("user {username}")))?;

    if let Some(password) = &body.password {
        if password.len() < 8 {
            return Err(ApiError::BadRequest(
                "password must be at least 8 characters".into(),
            ));
        }
        user.password_hash = hash_password(password)
            .map_err(|e| ApiError::Internal(anyhow::anyhow!("hashing failed: {e}")))?;
    }

    if let Some(is_admin) = body.admin {
        // Removing the last administrator would lock everyone out of the panel
        // with no way back in short of editing panel.json by hand.
        if !is_admin && user.admin && last_admin(&state, &username).await? {
            return Err(ApiError::BadRequest(
                "this is the only administrator; promote someone else first".into(),
            ));
        }
        user.admin = is_admin;
    }

    if let Some(servers) = body.servers {
        user.servers = servers;
    }

    let stored = user.clone();
    state
        .store
        .update(move |data| {
            if let Some(slot) = data
                .users
                .iter_mut()
                .find(|u| u.username == stored.username)
            {
                *slot = stored;
            }
        })
        .await?;

    // Permissions and passwords only take effect once existing tokens are gone.
    state.sessions.revoke_user(&username).await;
    tracing::info!(user = %username, by = %admin.username, "account updated");

    Ok(Json(UserView::from(&user)))
}

async fn delete(
    State(state): State<Arc<AppState>>,
    AdminIdentity(admin): AdminIdentity,
    Path(username): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    if username == admin.username {
        return Err(ApiError::BadRequest(
            "you cannot delete your own account".into(),
        ));
    }
    if state.store.user(&username).await.is_none() {
        return Err(ApiError::NotFound(format!("user {username}")));
    }
    if last_admin(&state, &username).await? {
        return Err(ApiError::BadRequest(
            "this is the only administrator; promote someone else first".into(),
        ));
    }

    let name = username.clone();
    state
        .store
        .update(move |data| data.users.retain(|u| u.username != name))
        .await?;
    state.sessions.revoke_user(&username).await;
    tracing::info!(user = %username, by = %admin.username, "account deleted");

    Ok(Json(serde_json::json!({ "ok": true })))
}

/// Whether `username` is the only admin left.
async fn last_admin(state: &AppState, username: &str) -> ApiResult<bool> {
    let users = state.store.read().await.users;
    let admins: Vec<&User> = users.iter().filter(|u| u.admin).collect();
    Ok(admins.len() == 1 && admins[0].username == username)
}

/// Routes under `/api/users`.
pub fn router() -> Router<Arc<AppState>> {
    Router::new().route("/users", get(list).post(create)).route(
        "/users/{username}",
        axum::routing::patch(update).delete(delete),
    )
}
