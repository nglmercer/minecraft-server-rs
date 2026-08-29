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

/// Body of the user creation endpoint.
#[derive(Deserialize)]
pub struct CreateUser {
    username: String,
    password: String,
    #[serde(default)]
    admin: bool,
    #[serde(default)]
    servers: Vec<String>,
}

fn validate_username(input: &str) -> ApiResult<String> {
    let username = input.trim();
    if username.is_empty() || username.len() > 128 || username.contains(['\0', '\r', '\n']) {
        return Err(ApiError::BadRequest(
            "username must be 1-128 bytes and contain no control characters".into(),
        ));
    }
    Ok(username.to_owned())
}

fn validate_server_ids(
    servers: &[String],
    available: &[crate::store::ServerRecord],
) -> ApiResult<()> {
    if servers.len() > 1024 {
        return Err(ApiError::BadRequest("too many server permissions".into()));
    }
    let mut unique = std::collections::HashSet::new();
    for server in servers {
        if server.len() > 128
            || server.contains(['\0', '\r', '\n'])
            || !unique.insert(server)
            || !available.iter().any(|record| record.id == *server)
        {
            return Err(ApiError::BadRequest(
                "invalid or unknown server permission".into(),
            ));
        }
    }
    Ok(())
}

async fn create(
    State(state): State<Arc<AppState>>,
    AdminIdentity(_): AdminIdentity,
    Json(body): Json<CreateUser>,
) -> ApiResult<Json<UserView>> {
    let username = validate_username(&body.username)?;
    if body.password.len() < 8 || body.password.len() > 1024 {
        return Err(ApiError::BadRequest(
            "password must be between 8 and 1024 characters".into(),
        ));
    }
    let password_hash = hash_password(&body.password)
        .map_err(|error| ApiError::Internal(anyhow::anyhow!("password hashing failed: {error}")))?;
    let user = User {
        username: username.clone(),
        password_hash,
        admin: body.admin,
        servers: body.servers,
    };
    let stored = user.clone();
    let inserted = state
        .store
        .try_update(move |data| -> ApiResult<bool> {
            if data
                .users
                .iter()
                .any(|user| user.username == stored.username)
            {
                return Ok(false);
            }
            validate_server_ids(&stored.servers, &data.servers)?;
            data.users.push(stored);
            Ok(true)
        })
        .await??;
    if !inserted {
        return Err(ApiError::Conflict(format!("{username} already exists")));
    }
    tracing::info!(user = %username, "account created");
    Ok(Json(UserView::from(&user)))
}

/// Body of the user update endpoint.
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
    let password_hash = match body.password {
        Some(password) => {
            if password.len() < 8 || password.len() > 1024 {
                return Err(ApiError::BadRequest(
                    "password must be between 8 and 1024 characters".into(),
                ));
            }
            Some(hash_password(&password).map_err(|error| {
                ApiError::Internal(anyhow::anyhow!("password hashing failed: {error}"))
            })?)
        }
        None => None,
    };
    let name = username.clone();
    let updated = state
        .store
        .try_update(move |data| -> ApiResult<User> {
            let Some(index) = data.users.iter().position(|user| user.username == name) else {
                return Err(ApiError::NotFound("user".into()));
            };
            if let Some(servers) = body.servers.as_ref() {
                validate_server_ids(servers, &data.servers)?;
            }
            let current = &data.users[index];
            if body.admin == Some(false)
                && current.admin
                && data.users.iter().filter(|user| user.admin).count() == 1
            {
                return Err(ApiError::BadRequest(
                    "this is the only administrator; promote someone else first".into(),
                ));
            }
            let mut next = current.clone();
            if let Some(hash) = password_hash {
                next.password_hash = hash;
            }
            if let Some(is_admin) = body.admin {
                next.admin = is_admin;
            }
            if let Some(servers) = body.servers {
                next.servers = servers;
            }
            data.users[index] = next.clone();
            Ok(next)
        })
        .await??;

    state.sessions.revoke_user(&username).await;
    tracing::info!(user = %username, by = %admin.username, "account updated");
    Ok(Json(UserView::from(&updated)))
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
    let name = username.clone();
    state
        .store
        .try_update(move |data| -> ApiResult<()> {
            let Some(index) = data.users.iter().position(|user| user.username == name) else {
                return Err(ApiError::NotFound("user".into()));
            };
            if data.users[index].admin && data.users.iter().filter(|user| user.admin).count() == 1 {
                return Err(ApiError::BadRequest(
                    "this is the only administrator; promote someone else first".into(),
                ));
            }
            data.users.remove(index);
            Ok(())
        })
        .await??;
    state.sessions.revoke_user(&username).await;
    tracing::info!(user = %username, by = %admin.username, "account deleted");
    Ok(Json(serde_json::json!({ "ok": true })))
}

/// Routes under the users prefix.
pub fn router() -> Router<Arc<AppState>> {
    Router::new().route("/users", get(list).post(create)).route(
        "/users/{username}",
        axum::routing::patch(update).delete(delete),
    )
}
