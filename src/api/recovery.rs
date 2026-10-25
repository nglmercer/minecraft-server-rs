//! Local password recovery using a single-use recovery token.

use axum::extract::{ConnectInfo, State};
use axum::routing::post;
use axum::{Extension, Json, Router};
use serde::Deserialize;
use std::net::SocketAddr;
use std::sync::Arc;

use crate::auth::hash_password;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

#[derive(Deserialize)]
pub struct RecoveryRequest {
    token: String,
    password: String,
    confirm: String,
}

fn is_loopback(addr: SocketAddr) -> bool {
    addr.ip().is_loopback()
}

async fn reset_password(
    State(state): State<Arc<AppState>>,
    peer: Option<Extension<ConnectInfo<SocketAddr>>>,
    Json(body): Json<RecoveryRequest>,
) -> ApiResult<Json<serde_json::Value>> {
    // Local-only: require loopback peer.
    let is_local = peer
        .as_ref()
        .map(|Extension(ConnectInfo(addr))| is_loopback(*addr))
        .unwrap_or(false);
    if !is_local {
        return Err(ApiError::Forbidden);
    }

    if body.token.trim().is_empty() {
        return Err(ApiError::BadRequest("missing recovery token".into()));
    }
    if body.password != body.confirm {
        return Err(ApiError::BadRequest("passwords do not match".into()));
    }
    if body.password.len() < 8 || body.password.len() > 1024 {
        return Err(ApiError::BadRequest(
            "password must be between 8 and 1024 characters".into(),
        ));
    }

    // Validate and consume token.
    let username = state
        .recovery
        .consume(&body.token)
        .ok_or(ApiError::BadRequest(
            "invalid or expired recovery token".into(),
        ))?;

    // If token validated but no such user (e.g., deleted after token creation), reject.
    // Find admin to update? Token is bound to specific username.
    let new_password = body.password.clone();
    let hash = tokio::task::spawn_blocking(move || hash_password(&new_password))
        .await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("hashing failed: {e}")))?
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("hashing failed: {e}")))?;

    let username_clone = username.clone();
    let hash_clone = hash.clone();
    let updated = state
        .store
        .update(move |data| {
            if let Some(user) = data.users.iter_mut().find(|u| u.username == username_clone) {
                user.password_hash = hash_clone;
                true
            } else {
                false
            }
        })
        .await
        .map_err(|e| ApiError::Internal(e))?;

    if !updated {
        return Err(ApiError::BadRequest("administrator unavailable".into()));
    }

    state.sessions.revoke_user(&username).await;
    // Invalidate any other tokens for this user (defense in depth)
    state.recovery.invalidate_user(&username);

    tracing::info!(user = %username, "password recovery completed");

    Ok(Json(serde_json::json!({ "ok": true })))
}

pub fn router() -> Router<Arc<AppState>> {
    Router::new().route("/recovery/reset", post(reset_password))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::Identity;
    use crate::state::PlayitMode;

    fn loopback() -> SocketAddr {
        "127.0.0.1:12345".parse().unwrap()
    }
    fn remote() -> SocketAddr {
        "203.0.113.1:12345".parse().unwrap()
    }

    async fn state_with_admin() -> std::sync::Arc<crate::state::AppState> {
        let tmp = tempfile::tempdir().unwrap();
        let s = crate::state::AppState::bootstrap(tmp.path(), PlayitMode::External)
            .await
            .unwrap();
        std::mem::forget(tmp);
        // create admin via store
        let hash = crate::auth::hash_password("oldpassword123").unwrap();
        s.store
            .update(|data| {
                data.users.push(crate::store::User {
                    username: "admin".into(),
                    password_hash: hash,
                    admin: true,
                    servers: vec![],
                })
            })
            .await
            .unwrap();
        s
    }

    async fn call_reset(
        state: std::sync::Arc<crate::state::AppState>,
        peer: SocketAddr,
        token: &str,
        password: &str,
        confirm: &str,
    ) -> Result<Json<serde_json::Value>, ApiError> {
        reset_password(
            State(state),
            Some(Extension(ConnectInfo(peer))),
            Json(RecoveryRequest {
                token: token.to_string(),
                password: password.to_string(),
                confirm: confirm.to_string(),
            }),
        )
        .await
    }

    #[tokio::test]
    async fn valid_token_resets_password_and_revokes_sessions() {
        let state = state_with_admin().await;
        // create session
        let token_session = state
            .sessions
            .create(Identity {
                username: "admin".into(),
                admin: true,
                servers: vec![],
            })
            .await;
        assert!(state.sessions.resolve(&token_session).await.is_some());
        let token = state.recovery.generate("admin".into());
        let _ = call_reset(
            state.clone(),
            loopback(),
            &token,
            "newpass123",
            "newpass123",
        )
        .await
        .unwrap();
        // old password no longer valid
        let user = state.store.user("admin").await.unwrap();
        assert!(crate::auth::verify_password(
            "newpass123",
            &user.password_hash
        ));
        assert!(!crate::auth::verify_password(
            "oldpassword123",
            &user.password_hash
        ));
        // sessions revoked
        assert!(state.sessions.resolve(&token_session).await.is_none());
        // token invalidated
        assert!(state.recovery.validate(&token).is_none());
        state.playit.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn invalid_token_rejected() {
        let state = state_with_admin().await;
        let err = call_reset(
            state.clone(),
            loopback(),
            "deadbeef",
            "newpass123",
            "newpass123",
        )
        .await
        .unwrap_err();
        assert!(matches!(err, ApiError::BadRequest(_)));
        state.playit.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn expired_token_rejected() {
        let state = state_with_admin().await;
        let token = state.recovery.generate("admin".into());
        state.recovery.force_expire(&token);
        let err = call_reset(
            state.clone(),
            loopback(),
            &token,
            "newpass123",
            "newpass123",
        )
        .await
        .unwrap_err();
        assert!(matches!(err, ApiError::BadRequest(_)));
        state.playit.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn token_cannot_be_reused() {
        let state = state_with_admin().await;
        let token = state.recovery.generate("admin".into());
        let _ = call_reset(
            state.clone(),
            loopback(),
            &token,
            "newpass123",
            "newpass123",
        )
        .await
        .unwrap();
        let err = call_reset(
            state.clone(),
            loopback(),
            &token,
            "anotherpass123",
            "anotherpass123",
        )
        .await
        .unwrap_err();
        assert!(matches!(err, ApiError::BadRequest(_)));
        state.playit.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn password_validation_enforced() {
        let state = state_with_admin().await;
        let token = state.recovery.generate("admin".into());
        let err = call_reset(state.clone(), loopback(), &token, "short", "short")
            .await
            .unwrap_err();
        assert!(matches!(err, ApiError::BadRequest(_)));
        // token should have been consumed? No, validation happens before consume? Actually consume before password length? In our code password length checked before consume, so token stays valid.
        // Verify token still valid after bad password length attempt (since we check length before consume)
        assert!(state.recovery.validate(&token).is_some());
        state.playit.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn recovery_is_local_only() {
        let state = state_with_admin().await;
        let token = state.recovery.generate("admin".into());
        let err = call_reset(state.clone(), remote(), &token, "newpass123", "newpass123")
            .await
            .unwrap_err();
        assert!(matches!(err, ApiError::Forbidden));
        // token should still be valid because request was rejected before consume
        assert!(state.recovery.validate(&token).is_some());
        state.playit.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn missing_token_rejected() {
        let state = state_with_admin().await;
        let err = call_reset(state.clone(), loopback(), "", "newpass123", "newpass123")
            .await
            .unwrap_err();
        assert!(matches!(err, ApiError::BadRequest(_)));
        state.playit.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn no_admin_after_token_creation_rejected() {
        let state = state_with_admin().await;
        let token = state.recovery.generate("admin".into());
        // delete admin
        state.store.update(|data| data.users.clear()).await.unwrap();
        let err = call_reset(
            state.clone(),
            loopback(),
            &token,
            "newpass123",
            "newpass123",
        )
        .await
        .unwrap_err();
        assert!(matches!(err, ApiError::BadRequest(_)));
        state.playit.shutdown().await.unwrap();
    }
}
