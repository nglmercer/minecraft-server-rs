//! What can be installed: server flavours, their versions, and local Javas.
//!
//! These endpoints proxy `minecraft-core` and `java-path` so the browser never
//! talks to PaperMC or Adoptium directly.

use axum::extract::{Path, State};
use axum::routing::get;
use axum::{Json, Router};
use minecraft_core::MinecraftClient;
use serde::Serialize;
use std::sync::Arc;

use crate::auth::Identity;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

/// One installable server flavour.
#[derive(Serialize)]
pub struct ProviderView {
    id: String,
    /// Whether this provider produces a server rather than a proxy.
    server: bool,
}

fn client() -> ApiResult<MinecraftClient> {
    MinecraftClient::builder()
        .build()
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("minecraft client: {e}")))
}

async fn providers(_: Identity) -> ApiResult<Json<Vec<ProviderView>>> {
    let ids = client()?.providers();
    Ok(Json(
        ids.into_iter()
            .map(|id| {
                let id = id.to_string();
                // Velocity and Waterfall are proxies: they have no world and
                // the UI should not offer them a "start the world" flow.
                let server = !matches!(id.as_str(), "velocity" | "waterfall");
                ProviderView { id, server }
            })
            .collect(),
    ))
}

async fn versions(_: Identity, Path(provider): Path<String>) -> ApiResult<Json<Vec<String>>> {
    let versions = client()?.versions(&provider).await.map_err(|error| {
        tracing::warn!(provider = %provider, error = ?error, "failed to load provider versions");
        ApiError::BadRequest("unable to load provider versions".into())
    })?;

    // Newest first: nobody scrolls to the bottom looking for 1.21.
    let mut out: Vec<String> = versions.into_iter().map(|v| v.to_string()).collect();
    out.reverse();
    Ok(Json(out))
}

/// One build of one version.
#[derive(Serialize)]
pub struct BuildView {
    build: String,
    published_at: Option<String>,
}

async fn builds(
    _: Identity,
    Path((provider, version)): Path<(String, String)>,
) -> ApiResult<Json<Vec<BuildView>>> {
    let builds = client()?
        .builds(&provider, &version)
        .await
        .map_err(|error| {
            tracing::warn!(provider = %provider, version = %version, error = ?error, "failed to load provider builds");
            ApiError::BadRequest("unable to load provider builds".into())
        })?;

    Ok(Json(
        builds
            .into_iter()
            .rev()
            .map(|b| BuildView {
                build: b.build_id.to_string(),
                published_at: b.published_at,
            })
            .collect(),
    ))
}

/// A Java installation the host already has.
#[derive(Serialize)]
pub struct JavaView {
    major: u32,
    version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<String>,
    vendor: Option<String>,
    jdk: bool,
}

fn java_view(install: &java_path::JavaInstallation, include_path: bool) -> JavaView {
    JavaView {
        major: install.major(),
        version: install.version.to_string(),
        path: include_path.then(|| install.java.display().to_string()),
        vendor: install.vendor.clone(),
        jdk: install.is_jdk(),
    }
}

async fn javas(identity: Identity) -> Json<Vec<JavaView>> {
    let installs = java_path::discover().unwrap_or_default();
    let mut out: Vec<JavaView> = installs
        .iter()
        .map(|install| java_view(install, identity.admin))
        .collect();
    out.sort_by_key(|java| std::cmp::Reverse(java.major));
    Json(out)
}

/// Host resource usage, for the dashboard header.
#[derive(Serialize)]
pub struct SystemStats {
    #[serde(flatten)]
    host: crate::metrics::HostMetrics,
    servers_online: usize,
}

async fn system(
    State(state): State<Arc<AppState>>,
    identity: Identity,
) -> ApiResult<Json<SystemStats>> {
    let mut online = 0;
    for record in state.store.read().await.servers {
        if !identity.may_access(&record.id) {
            continue;
        }
        if state
            .guardian(&record.id)
            .await?
            .status()
            .await
            .is_running()
        {
            online += 1;
        }
    }

    // Sampled from the long-lived Metrics, which keeps the previous CPU reading
    // to measure against.
    Ok(Json(SystemStats {
        host: state.metrics.host(),
        servers_online: online,
    }))
}

/// Routes under `/api`.
pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/catalog/providers", get(providers))
        .route("/catalog/{provider}/versions", get(versions))
        .route("/catalog/{provider}/{version}/builds", get(builds))
        .route("/catalog/javas", get(javas))
        .route("/system", get(system))
}

#[cfg(test)]
mod tests {
    use super::*;
    use java_path::{Architecture, DiscoverySource, JavaKind, JavaVersion, Platform};
    use std::path::PathBuf;

    fn installation() -> java_path::JavaInstallation {
        java_path::JavaInstallation {
            home: PathBuf::from("/host/jdks/java-21"),
            java: PathBuf::from("/host/jdks/java-21/bin/java"),
            javac: Some(PathBuf::from("/host/jdks/java-21/bin/javac")),
            version: JavaVersion::parse("21.0.1").unwrap(),
            vendor: Some("Test Vendor".into()),
            architecture: Architecture::X86_64,
            platform: Platform::Linux,
            kind: JavaKind::Jdk,
            source: DiscoverySource::UserDirectory,
        }
    }

    #[test]
    fn regular_java_catalog_entries_redact_host_paths() {
        let value = serde_json::to_value(java_view(&installation(), false)).unwrap();

        assert_eq!(value["major"], 21);
        assert!(value.get("path").is_none());
        assert!(!value.to_string().contains("/host/jdks"));
    }

    #[test]
    fn admin_java_catalog_entries_keep_the_path_for_administration() {
        let value = serde_json::to_value(java_view(&installation(), true)).unwrap();

        assert_eq!(value["path"], "/host/jdks/java-21/bin/java");
    }
}
