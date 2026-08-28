//! Playit account and daemon endpoints.

use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};
use playit_integration::{ClaimInfo, PlayitAccount, PlayitStatus};
use std::sync::Arc;

use crate::auth::{AdminIdentity, Identity};
use crate::error::ApiResult;
use crate::state::AppState;

/// GET `/api/playit/status`.
///
/// A missing or stopped daemon is a normal deployment state, so this endpoint
/// returns a usable status document instead of preventing the panel from
/// starting or turning the status check into a generic HTTP 500.
async fn status(State(state): State<Arc<AppState>>, _: Identity) -> Json<PlayitStatus> {
    let status = match state.playit.status().await {
        Ok(status) => status,
        Err(error) => playit_integration::PlayitManager::status_from_error(&error),
    };
    Json(status)
}

/// GET `/api/playit/account`.
async fn account(
    State(state): State<Arc<AppState>>,
    _: Identity,
) -> ApiResult<Json<PlayitAccount>> {
    Ok(Json(state.playit.account().await?))
}

/// POST `/api/playit/claim`.
async fn claim(
    State(state): State<Arc<AppState>>,
    AdminIdentity(_admin): AdminIdentity,
) -> ApiResult<Json<ClaimInfo>> {
    Ok(Json(state.playit.start_claim().await?))
}

/// Routes under `/api`.
pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/playit/status", get(status))
        .route("/playit/account", get(account))
        .route("/playit/claim", post(claim))
}
