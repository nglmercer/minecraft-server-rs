//! First-run setup: create the initial administrator when no users exist.

use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::auth::hash_password;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

#[derive(Serialize)]
struct StatusResponse {
    needs_setup: bool,
}

async fn status(State(state): State<Arc<AppState>>) -> Json<StatusResponse> {
    let needs = state.store.read().await.users.is_empty();
    Json(StatusResponse { needs_setup: needs })
}

#[derive(Deserialize)]
pub struct SetupRequest {
    username: String,
    password: String,
    confirm: String,
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

async fn create_setup(
    State(state): State<Arc<AppState>>,
    Json(body): Json<SetupRequest>,
) -> ApiResult<Json<serde_json::Value>> {
    let username = validate_username(&body.username)?;
    if body.password != body.confirm {
        return Err(ApiError::BadRequest("passwords do not match".into()));
    }
    if body.password.len() < 8 || body.password.len() > 1024 {
        return Err(ApiError::BadRequest(
            "password must be between 8 and 1024 characters".into(),
        ));
    }

    // Copy needed for closure
    let provided = body.password.clone();
    let username_clone = username.clone();

    // Hash outside lock? Hash is expensive; but we need atomic check. We can hash first
    // then try_update, but atomicity still holds because try_update checks emptiness.
    let hash = tokio::task::spawn_blocking(move || hash_password(&provided))
        .await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("hashing failed: {e}")))?
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("hashing failed: {e}")))?;

    let username_for_closure = username_clone.clone();
    let result = state
        .store
        .try_update(move |data| -> ApiResult<()> {
            if !data.users.is_empty() {
                return Err(ApiError::Forbidden);
            }
            data.users.push(crate::store::User {
                username: username_for_closure.clone(),
                password_hash: hash.clone(),
                admin: true,
                servers: Vec::new(),
            });
            Ok(())
        })
        .await
        .map_err(|e| ApiError::Internal(e))?;

    match result {
        Ok(()) => {
            tracing::info!(user = %username_clone, "initial administrator created via setup");
            Ok(Json(serde_json::json!({ "ok": true })))
        }
        Err(ApiError::Forbidden) => Err(ApiError::Forbidden),
        Err(e) => Err(e),
    }
}

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/setup/status", get(status))
        .route("/setup", post(create_setup))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::PlayitMode;

    async fn state_with_no_users() -> std::sync::Arc<crate::state::AppState> {
        let tmp = tempfile::tempdir().unwrap();
        let s = crate::state::AppState::bootstrap(tmp.path(), PlayitMode::External)
            .await
            .unwrap();
        // Keep tempdir alive by leaking? Use state data_dir canonicalized tmp; tempdir will be dropped but data_dir persists.
        // Instead use Box::leak for dir
        std::mem::forget(tmp);
        s
    }

    #[tokio::test]
    async fn needs_setup_when_empty() {
        let state = state_with_no_users().await;
        let needs = state.store.read().await.users.is_empty();
        assert!(needs);
        state.playit.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn setup_creates_first_admin_and_hashes_password() {
        let state = state_with_no_users().await;
        let req = SetupRequest {
            username: "admin".into(),
            password: "correct-horse-batterystaple".into(),
            confirm: "correct-horse-batterystaple".into(),
        };
        let resp = create_setup(State(state.clone()), Json(req)).await.unwrap();
        assert_eq!(resp.0["ok"], true);
        let user = state.store.user("admin").await.unwrap();
        assert!(user.admin);
        assert!(crate::auth::verify_password(
            "correct-horse-batterystaple",
            &user.password_hash
        ));
        assert!(!crate::auth::verify_password("wrong", &user.password_hash));
        state.playit.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn setup_cannot_be_repeated_after_initialization() {
        let state = state_with_no_users().await;
        let req = || SetupRequest {
            username: "admin".into(),
            password: "password123".into(),
            confirm: "password123".into(),
        };
        let _ = create_setup(State(state.clone()), Json(req()))
            .await
            .unwrap();
        let err = create_setup(State(state.clone()), Json(req()))
            .await
            .unwrap_err();
        assert!(matches!(err, ApiError::Forbidden));
        // ensure still one user
        assert_eq!(state.store.read().await.users.len(), 1);
        state.playit.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn simultaneous_setup_cannot_create_multiple_admins() {
        let state = state_with_no_users().await;
        let mut handles = Vec::new();
        for i in 0..10 {
            let s = state.clone();
            handles.push(tokio::spawn(async move {
                let req = SetupRequest {
                    username: format!("admin{i}"),
                    password: "password123".into(),
                    confirm: "password123".into(),
                };
                create_setup(State(s), Json(req)).await
            }));
        }
        let mut successes = 0;
        for h in handles {
            if h.await.unwrap().is_ok() {
                successes += 1;
            }
        }
        assert_eq!(successes, 1, "only one setup should succeed");
        assert_eq!(state.store.read().await.users.len(), 1);
        state.playit.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn invalid_password_rejected() {
        let state = state_with_no_users().await;
        let req = SetupRequest {
            username: "admin".into(),
            password: "short".into(),
            confirm: "short".into(),
        };
        let err = create_setup(State(state.clone()), Json(req))
            .await
            .unwrap_err();
        assert!(matches!(err, ApiError::BadRequest(_)));
        assert!(state.store.read().await.users.is_empty());
        state.playit.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn password_mismatch_rejected() {
        let state = state_with_no_users().await;
        let req = SetupRequest {
            username: "admin".into(),
            password: "password123".into(),
            confirm: "different123".into(),
        };
        let err = create_setup(State(state.clone()), Json(req))
            .await
            .unwrap_err();
        assert!(matches!(err, ApiError::BadRequest(_)));
        state.playit.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn custom_username_allowed() {
        let state = state_with_no_users().await;
        let req = SetupRequest {
            username: "alice".into(),
            password: "password123".into(),
            confirm: "password123".into(),
        };
        let _ = create_setup(State(state.clone()), Json(req)).await.unwrap();
        assert!(state.store.user("alice").await.is_some());
        state.playit.shutdown().await.unwrap();
    }
}
