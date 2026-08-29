//! The HTTP API.

pub mod auth;
pub mod backups;
pub mod catalog;
pub mod console;
pub mod files;
pub mod mods;
pub mod playit;
pub mod servers;
pub mod users;

use axum::extract::DefaultBodyLimit;
use axum::Router;
use std::sync::Arc;

use crate::error::ApiError;
use crate::limits::ResourceLimits;
use crate::state::AppState;

/// Everything under `/api`.
///
/// The per-server routers are merged into one before nesting: nesting several
/// routers at the same prefix silently drops all but the last.
#[allow(dead_code)]
pub fn router() -> Router<Arc<AppState>> {
    router_with_limits(ResourceLimits::default())
}

/// Build the API with the installation's request limits.
pub fn router_with_limits(limits: ResourceLimits) -> Router<Arc<AppState>> {
    let per_server = servers::router()
        .merge(console::router())
        .merge(files::router_with_limits(limits))
        .merge(backups::router())
        .merge(mods::router());

    Router::new()
        .nest("/auth", auth::router())
        .merge(servers::collection_router())
        .nest("/servers", per_server)
        .merge(users::router())
        .merge(catalog::router())
        .merge(playit::router())
        .layer(axum::middleware::from_fn(crate::auth::csrf_protect))
        .layer(DefaultBodyLimit::max(crate::limits::MAX_JSON_BODY_BYTES))
        // Without this, a mistyped API path falls through to the SPA and
        // answers 200 with HTML, which reads as success to any client.
        .fallback(|| async { ApiError::NotFound("endpoint".into()) })
}
