//! The HTTP API.

pub mod auth;
pub mod backups;
pub mod catalog;
pub mod console;
pub mod files;
pub mod mods;
pub mod servers;
pub mod users;

use axum::Router;
use std::sync::Arc;

use crate::state::AppState;

/// Everything under `/api`.
///
/// The per-server routers are merged into one before nesting: nesting several
/// routers at the same prefix silently drops all but the last.
pub fn router() -> Router<Arc<AppState>> {
    let per_server = servers::router()
        .merge(console::router())
        .merge(files::router())
        .merge(backups::router())
        .merge(mods::router());

    Router::new()
        .nest("/auth", auth::router())
        .merge(servers::collection_router())
        .nest("/servers", per_server)
        .merge(users::router())
        .merge(catalog::router())
}
