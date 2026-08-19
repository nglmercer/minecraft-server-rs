//! Serving the frontend out of the binary.
//!
//! The Vite build is embedded at compile time, so deploying the panel is one
//! file: no nginx, no static directory to keep in sync, no Node on the host.

use axum::body::Body;
use axum::extract::Request;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "$CARGO_MANIFEST_DIR/../../web/dist"]
struct Assets;

/// Shown when the binary was built without running the frontend build first.
const NO_FRONTEND: &str = r#"<!doctype html>
<meta charset="utf-8">
<title>Panel — frontend not built</title>
<style>body{font:16px/1.6 system-ui;margin:8vh auto;max-width:44rem;padding:0 1.5rem;background:#0b0f14;color:#e6edf3}
code{background:#151b23;padding:.15em .4em;border-radius:4px}</style>
<h1>Frontend not built</h1>
<p>The API is running, but no compiled frontend was embedded in this binary.</p>
<pre><code>cd web &amp;&amp; npm install &amp;&amp; npm run build
cargo build --release</code></pre>
<p>The REST API is available under <code>/api</code> regardless.</p>
"#;

/// Serve an embedded asset, falling back to `index.html` so client-side routes
/// survive a page reload.
pub async fn serve(request: Request) -> Response {
    let path = request.uri().path().trim_start_matches('/');

    if let Some(response) = asset(path) {
        return response;
    }

    // Anything that is not a file is a frontend route; hand it the app shell.
    match asset("index.html") {
        Some(response) => response,
        None => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
            NO_FRONTEND,
        )
            .into_response(),
    }
}

fn asset(path: &str) -> Option<Response> {
    let file = Assets::get(path)?;
    let mime = mime_from(path);

    // Hashed Vite bundles are immutable; index.html must never be cached or a
    // deploy leaves clients on the old app forever.
    let cache = if path == "index.html" {
        "no-cache"
    } else {
        "public, max-age=31536000, immutable"
    };

    Some(
        (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, mime),
                (header::CACHE_CONTROL, cache.to_string()),
            ],
            Body::from(file.data.into_owned()),
        )
            .into_response(),
    )
}

fn mime_from(path: &str) -> String {
    mime_guess::from_path(path)
        .first_or_octet_stream()
        .to_string()
}
