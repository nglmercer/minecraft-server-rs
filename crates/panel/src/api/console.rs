//! The live console WebSocket.
//!
//! One socket carries both directions: guardian events stream down as JSON,
//! and the client sends `{"type":"command","command":"..."}` up.

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::response::Response;
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use std::sync::Arc;
use tokio::sync::{broadcast::error::RecvError, OwnedSemaphorePermit};

use crate::auth::{token_from_headers, Identity};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

/// What a client may send up the socket.
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientMessage {
    /// Forward a line to the server's stdin.
    Command {
        /// The console line, without a trailing newline.
        command: String,
    },
    /// Keepalive; the panel replies with nothing.
    Ping,
}

async fn upgrade(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(query): Query<TicketQuery>,
) -> ApiResult<Response> {
    let ws = ws
        .max_message_size(guardian::process::MAX_COMMAND_BYTES + 1024)
        .max_frame_size(guardian::process::MAX_COMMAND_BYTES + 1024);
    let Some((server, session_token)) = state.tickets.redeem_console(&query.ticket) else {
        return Err(ApiError::Unauthorized);
    };
    if server != id {
        return Err(ApiError::Unauthorized);
    }
    let identity = state
        .sessions
        .resolve(&session_token)
        .await
        .ok_or(ApiError::Unauthorized)?;
    if !identity.may_access(&id) {
        return Err(ApiError::NotFound("server".into()));
    }
    let websocket_slot = state
        .websocket_slots
        .clone()
        .try_acquire_owned()
        .map_err(|_| ApiError::TooManyRequests)?;
    // Resolve before upgrading, so a bad id fails as a clean 404 rather than a
    // socket that opens and immediately closes.
    let guardian = state.guardian(&id).await?;
    Ok(ws.on_upgrade(move |socket| pump(socket, guardian, websocket_slot)))
}

#[derive(serde::Deserialize)]
struct TicketQuery {
    ticket: String,
}

/// Issue the one-use ticket used by the browser WebSocket handshake.
async fn issue_ticket(
    State(state): State<Arc<AppState>>,
    identity: Identity,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> ApiResult<Json<serde_json::Value>> {
    if !identity.may_access(&id) {
        return Err(ApiError::NotFound("server".into()));
    }
    state.guardian(&id).await?;
    let token = token_from_headers(&headers).ok_or(ApiError::Unauthorized)?;
    let ticket = state.tickets.issue_console(id, token);
    Ok(Json(serde_json::json!({ "ticket": ticket })))
}

async fn pump(
    socket: WebSocket,
    guardian: Arc<guardian::Guardian>,
    _websocket_slot: OwnedSemaphorePermit,
) {
    let (mut tx, mut rx) = socket.split();

    // Subscribe before backfilling, so a line logged during the backfill is
    // delivered late rather than dropped.
    let mut events = guardian.subscribe();

    let backfill = serde_json::json!({
        "type": "backfill",
        "status": guardian.snapshot().await,
        "lines": guardian.console().await,
    });
    if tx
        .send(Message::Text(backfill.to_string().into()))
        .await
        .is_err()
    {
        return;
    }

    let sender = tokio::spawn(async move {
        loop {
            match events.recv().await {
                Ok(event) => {
                    let Ok(text) = serde_json::to_string(&event) else {
                        continue;
                    };
                    if tx.send(Message::Text(text.into())).await.is_err() {
                        return;
                    }
                }
                // A slow client that fell behind gets told, not disconnected.
                Err(RecvError::Lagged(n)) => {
                    let notice = serde_json::json!({ "type": "lagged", "skipped": n });
                    if tx
                        .send(Message::Text(notice.to_string().into()))
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
                Err(RecvError::Closed) => return,
            }
        }
    });

    while let Some(Ok(message)) = rx.next().await {
        let Message::Text(text) = message else {
            continue;
        };
        if text.len() > guardian::process::MAX_COMMAND_BYTES + 1024 {
            continue;
        }
        match serde_json::from_str::<ClientMessage>(&text) {
            Ok(ClientMessage::Command { command }) => {
                let _ = guardian.command(command.trim()).await;
            }
            Ok(ClientMessage::Ping) | Err(_) => {}
        }
    }

    sender.abort();
}

/// The `/api/servers/{id}/ws` route.
pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/{id}/ws", get(upgrade))
        .route("/{id}/ws/ticket", post(issue_ticket))
}
