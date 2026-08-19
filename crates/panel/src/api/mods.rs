//! Installing plugins and mods from Modrinth.
//!
//! The panel already knows the server's flavour and Minecraft version, so it
//! scopes every search and version lookup to what will actually load. Browsing
//! results that cannot run on your server is worse than no browser at all.

use axum::extract::{Path, Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncWriteExt;

use crate::auth::Identity;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use crate::store::ServerRecord;

const MODRINTH: &str = "https://api.modrinth.com/v2";

/// Modrinth asks that clients identify themselves.
const USER_AGENT: &str = concat!(
    "nglmercer/minecraft-server-rs/",
    env!("CARGO_PKG_VERSION"),
    " (panel)"
);

/// Where a server's add-ons go, and what Modrinth calls its loader.
#[derive(Debug)]
struct Loader {
    /// Modrinth loader facets to filter by.
    names: &'static [&'static str],
    /// Directory under the server root, relative.
    directory: &'static str,
    /// `mod` or `plugin`, as Modrinth's `project_type` facet.
    project_type: &'static str,
}

/// Map a `minecraft-core` provider onto Modrinth's vocabulary.
fn loader_for(core: &str) -> ApiResult<Loader> {
    Ok(match core {
        // The Bukkit family loads the same plugin jars, so accepting all of
        // those facets finds far more than filtering on "paper" alone.
        "paper" | "purpur" | "folia" => Loader {
            names: &["paper", "purpur", "folia", "spigot", "bukkit"],
            directory: "plugins",
            project_type: "plugin",
        },
        "velocity" => Loader {
            names: &["velocity"],
            directory: "plugins",
            project_type: "plugin",
        },
        "waterfall" => Loader {
            names: &["waterfall", "bungeecord"],
            directory: "plugins",
            project_type: "plugin",
        },
        "fabric" => Loader {
            names: &["fabric"],
            directory: "mods",
            project_type: "mod",
        },
        "forge" => Loader {
            names: &["forge"],
            directory: "mods",
            project_type: "mod",
        },
        // Hybrids load Forge mods and Bukkit plugins; mods are the common case.
        "mohist" | "arclight" => Loader {
            names: &["forge"],
            directory: "mods",
            project_type: "mod",
        },
        "vanilla" => {
            return Err(ApiError::BadRequest(
                "a vanilla server cannot load plugins or mods; use Paper or Fabric".into(),
            ))
        }
        other => {
            return Err(ApiError::BadRequest(format!(
                "unknown server flavour {other}"
            )))
        }
    })
}

fn client() -> ApiResult<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("http client: {e}")))
}

async fn authorized(state: &AppState, identity: &Identity, id: &str) -> ApiResult<ServerRecord> {
    if !identity.may_access(id) {
        return Err(ApiError::Forbidden);
    }
    state
        .store
        .server(id)
        .await
        .ok_or_else(|| ApiError::NotFound(format!("server {id}")))
}

/// One search result, trimmed to what the UI shows.
#[derive(Serialize)]
pub struct ProjectView {
    project_id: String,
    slug: String,
    title: String,
    description: String,
    downloads: u64,
    icon_url: Option<String>,
    categories: Vec<String>,
}

#[derive(Deserialize)]
struct SearchResponse {
    hits: Vec<SearchHit>,
}

#[derive(Deserialize)]
struct SearchHit {
    project_id: String,
    slug: String,
    title: String,
    description: String,
    #[serde(default)]
    downloads: u64,
    #[serde(default)]
    icon_url: Option<String>,
    #[serde(default)]
    categories: Vec<String>,
}

/// `?q=` on the search endpoint.
#[derive(Deserialize)]
pub struct SearchQuery {
    #[serde(default)]
    q: String,
}

async fn search(
    State(state): State<Arc<AppState>>,
    identity: Identity,
    Path(id): Path<String>,
    Query(query): Query<SearchQuery>,
) -> ApiResult<Json<Vec<ProjectView>>> {
    let record = authorized(&state, &identity, &id).await?;
    let loader = loader_for(&record.config.core)?;

    // Facets are AND-ed across groups and OR-ed within one, which is exactly
    // the shape needed: this project type, any of these loaders, this version.
    let loaders = loader
        .names
        .iter()
        .map(|name| format!("\"categories:{name}\""))
        .collect::<Vec<_>>()
        .join(",");
    let facets = format!(
        "[[\"project_type:{}\"],[{}],[\"versions:{}\"]]",
        loader.project_type, loaders, record.config.version
    );

    let response = client()?
        .get(format!("{MODRINTH}/search"))
        .query(&[
            ("query", query.q.as_str()),
            ("facets", facets.as_str()),
            ("limit", "40"),
            ("index", "relevance"),
        ])
        .send()
        .await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("modrinth search: {e}")))?;

    if !response.status().is_success() {
        return Err(ApiError::BadRequest(format!(
            "modrinth returned {}",
            response.status()
        )));
    }

    let body: SearchResponse = response
        .json()
        .await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("modrinth response: {e}")))?;

    Ok(Json(
        body.hits
            .into_iter()
            .map(|hit| ProjectView {
                project_id: hit.project_id,
                slug: hit.slug,
                title: hit.title,
                description: hit.description,
                downloads: hit.downloads,
                icon_url: hit.icon_url,
                categories: hit.categories,
            })
            .collect(),
    ))
}

#[derive(Deserialize)]
struct Version {
    name: String,
    version_number: String,
    files: Vec<VersionFile>,
    #[serde(default)]
    date_published: Option<String>,
}

#[derive(Deserialize)]
struct VersionFile {
    url: String,
    filename: String,
    #[serde(default)]
    primary: bool,
    #[serde(default)]
    size: u64,
}

/// Body of `POST /api/servers/:id/mods/install`.
#[derive(Deserialize)]
pub struct InstallRequest {
    /// Modrinth project id or slug.
    project: String,
}

/// What was installed.
#[derive(Serialize)]
pub struct InstallResult {
    name: String,
    version: String,
    filename: String,
    path: String,
    size: u64,
}

async fn install(
    State(state): State<Arc<AppState>>,
    identity: Identity,
    Path(id): Path<String>,
    Json(body): Json<InstallRequest>,
) -> ApiResult<Json<InstallResult>> {
    let record = authorized(&state, &identity, &id).await?;
    let loader = loader_for(&record.config.core)?;
    let http = client()?;

    let loaders = loader
        .names
        .iter()
        .map(|name| format!("\"{name}\""))
        .collect::<Vec<_>>()
        .join(",");

    let response = http
        .get(format!("{MODRINTH}/project/{}/version", body.project))
        .query(&[
            ("game_versions", format!("[\"{}\"]", record.config.version)),
            ("loaders", format!("[{loaders}]")),
        ])
        .send()
        .await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("modrinth versions: {e}")))?;

    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Err(ApiError::NotFound(format!(
            "modrinth project {}",
            body.project
        )));
    }

    let mut versions: Vec<Version> = response
        .json()
        .await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("modrinth versions: {e}")))?;

    if versions.is_empty() {
        return Err(ApiError::BadRequest(format!(
            "{} has no release for {} on {}",
            body.project, record.config.version, record.config.core
        )));
    }

    // Modrinth returns newest first, but the ordering is not contractual.
    versions.sort_by(|a, b| b.date_published.cmp(&a.date_published));
    let version = versions.remove(0);

    let file = version
        .files
        .iter()
        .find(|f| f.primary)
        .or_else(|| version.files.first())
        .ok_or_else(|| ApiError::BadRequest("that release has no downloadable file".into()))?;

    // The filename comes from a third party, so it is sanitised to a bare name
    // before it is ever joined onto a path.
    let filename = std::path::Path::new(&file.filename)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .filter(|n| !n.is_empty() && n != "." && n != "..")
        .ok_or_else(|| ApiError::BadRequest("that release has an unusable filename".into()))?;

    let directory = record.config.directory.join(loader.directory);
    tokio::fs::create_dir_all(&directory)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;

    let bytes = http
        .get(&file.url)
        .send()
        .await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("download: {e}")))?
        .bytes()
        .await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("download: {e}")))?;

    let target = directory.join(&filename);
    let mut out = tokio::fs::File::create(&target)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    out.write_all(&bytes)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    out.flush()
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;

    tracing::info!(server = %id, file = %filename, "add-on installed");

    Ok(Json(InstallResult {
        name: version.name,
        version: version.version_number,
        path: format!("{}/{}", loader.directory, filename),
        size: if file.size > 0 {
            file.size
        } else {
            bytes.len() as u64
        },
        filename,
    }))
}

/// What is already installed, read off disk rather than tracked in state.
#[derive(Serialize)]
pub struct InstalledView {
    filename: String,
    path: String,
    size: u64,
}

async fn installed(
    State(state): State<Arc<AppState>>,
    identity: Identity,
    Path(id): Path<String>,
) -> ApiResult<Json<Vec<InstalledView>>> {
    let record = authorized(&state, &identity, &id).await?;
    let loader = loader_for(&record.config.core)?;
    let directory = record.config.directory.join(loader.directory);

    let mut out = Vec::new();
    let Ok(mut dir) = tokio::fs::read_dir(&directory).await else {
        return Ok(Json(out));
    };

    while let Ok(Some(entry)) = dir.next_entry().await {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.ends_with(".jar") {
            continue;
        }
        let size = entry.metadata().await.map(|m| m.len()).unwrap_or(0);
        out.push(InstalledView {
            path: format!("{}/{}", loader.directory, name),
            filename: name,
            size,
        });
    }

    out.sort_by(|a, b| a.filename.cmp(&b.filename));
    Ok(Json(out))
}

/// Routes nested under `/api/servers`.
pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/{id}/mods", get(installed))
        .route("/{id}/mods/search", get(search))
        .route("/{id}/mods/install", post(install))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bukkit_family_servers_install_plugins() {
        for core in ["paper", "purpur", "folia"] {
            let loader = loader_for(core).unwrap();
            assert_eq!(loader.directory, "plugins");
            assert_eq!(loader.project_type, "plugin");
            assert!(loader.names.contains(&"bukkit") || loader.names.contains(&core));
        }
    }

    #[test]
    fn modded_servers_install_mods() {
        for core in ["fabric", "forge", "mohist", "arclight"] {
            let loader = loader_for(core).unwrap();
            assert_eq!(loader.directory, "mods", "{core} should use mods/");
            assert_eq!(loader.project_type, "mod");
        }
    }

    #[test]
    fn vanilla_is_refused_with_an_explanation() {
        let error = loader_for("vanilla").unwrap_err();
        assert!(matches!(error, ApiError::BadRequest(_)));
        assert!(
            error.to_string().contains("Paper"),
            "should suggest an alternative"
        );
    }

    #[test]
    fn unknown_flavours_are_refused() {
        assert!(loader_for("nonsense").is_err());
    }
}
