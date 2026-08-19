//! The live console WebSocket.
//!
//! One socket carries both directions: guardian events stream down as JSON,
//! and the client sends `{"type":"command","command":"..."}` up.

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::response::Response;
use axum::routing::get;
use axum::Router;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use std::sync::Arc;
use tokio::sync::broadcast::error::RecvError;

use crate::auth::Identity;
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
    identity: Identity,
    Path(id): Path<String>,
) -> ApiResult<Response> {
    if !identity.may_access(&id) {
        return Err(ApiError::Forbidden);
    }
    // Resolve before upgrading, so a bad id fails as a clean 404 rather than a
    // socket that opens and immediately closes.
    let guardian = state.guardian(&id).await?;
    Ok(ws.on_upgrade(move |socket| pump(socket, guardian)))
}

async fn pump(socket: WebSocket, guardian: Arc<guardian::Guardian>) {
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
    Router::new().route("/{id}/ws", get(upgrade))
}
