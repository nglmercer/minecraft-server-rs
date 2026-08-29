//! Installing plugins and mods from Modrinth.
//!
//! The panel already knows the server's flavour and Minecraft version, so it
//! scopes every search and version lookup to what will actually load. Browsing
//! results that cannot run on your server is worse than no browser at all.

use axum::extract::{Path, Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::{Stream, StreamExt};
use serde::{Deserialize, Serialize};
use sha1::Sha1;
use sha2::{Digest, Sha512};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncWrite, AsyncWriteExt};

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
        // Do not follow a metadata-controlled redirect.  The URL is checked
        // and DNS-filtered immediately before the download; disabling
        // redirects prevents a later hop from bypassing that check.
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("http client: {e}")))
}

fn is_trusted_download_url(url: &reqwest::Url) -> bool {
    if url.scheme() != "https" {
        return false;
    }
    let Some(host) = url.host_str() else {
        return false;
    };
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    // File URLs are expected to come from Modrinth's API/CDN. Accepting an
    // arbitrary subdomain would turn a compromised metadata response into a
    // redirectable SSRF primitive.
    if !matches!(
        host.as_str(),
        "api.modrinth.com" | "cdn.modrinth.com" | "cdn-raw.modrinth.com" | "modrinth.com"
    ) {
        return false;
    }
    if let Ok(address) = host.parse::<std::net::IpAddr>() {
        return !is_private_address(address);
    }
    true
}

fn is_private_address(address: std::net::IpAddr) -> bool {
    match address {
        std::net::IpAddr::V4(address) => {
            address.is_private()
                || address.is_loopback()
                || address.is_link_local()
                || address.is_unspecified()
                || address.octets() == [169, 254, 169, 254]
        }
        std::net::IpAddr::V6(address) => {
            address.is_loopback()
                || address.is_unspecified()
                || address.is_unique_local()
                || address.segments()[0] & 0xffc0 == 0xfe80
        }
    }
}

async fn validate_download_url(raw: &str) -> ApiResult<reqwest::Url> {
    let url = reqwest::Url::parse(raw)
        .map_err(|_| ApiError::BadRequest("Modrinth returned an invalid download URL".into()))?;
    if !is_trusted_download_url(&url) {
        return Err(ApiError::BadRequest(
            "Modrinth download URL is not an allowed HTTPS endpoint".into(),
        ));
    }
    let host = url
        .host_str()
        .ok_or_else(|| ApiError::BadRequest("Modrinth download URL has no host".into()))?;
    let port = url.port_or_known_default().unwrap_or(443);
    let resolves_publicly = {
        let addresses = tokio::net::lookup_host((host, port)).await.map_err(|_| {
            ApiError::BadRequest("Modrinth download host could not be resolved".into())
        })?;
        addresses
            .into_iter()
            .any(|address| !is_private_address(address.ip()))
    };
    if !resolves_publicly {
        return Err(ApiError::BadRequest(
            "Modrinth download URL resolves to a private network".into(),
        ));
    }
    Ok(url)
}

fn digest_hex(bytes: impl AsRef<[u8]>) -> String {
    bytes
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

async fn authorized(state: &AppState, identity: &Identity, id: &str) -> ApiResult<ServerRecord> {
    if !identity.may_access(id) {
        return Err(ApiError::NotFound("server".into()));
    }
    state
        .store
        .server(id)
        .await
        .ok_or_else(|| ApiError::NotFound("server".into()))
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
    if query.q.len() > 256 || query.q.contains(['\0', '\r', '\n']) {
        return Err(ApiError::BadRequest(
            "search query is too long or invalid".into(),
        ));
    }

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
    #[serde(default)]
    hashes: Hashes,
}

#[derive(Default, Deserialize)]
struct Hashes {
    #[serde(default)]
    sha512: Option<String>,
    #[serde(default)]
    sha1: Option<String>,
}

/// Body of `POST /api/servers/:id/mods/install`.
#[derive(Deserialize)]
pub struct InstallRequest {
    /// Modrinth project id or slug.
    project: String,
}

fn project_id(input: &str) -> ApiResult<&str> {
    if input.is_empty()
        || input.len() > 128
        || input.contains(['/', '\\', '?', '#', '\0', '\r', '\n'])
        || !input
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(ApiError::BadRequest("invalid Modrinth project id".into()));
    }
    Ok(input)
}

fn expected_hash(value: &str, bytes: usize) -> ApiResult<String> {
    let value = value.trim();
    if value.len() != bytes * 2 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(ApiError::BadRequest(
            "Modrinth returned an invalid file hash".into(),
        ));
    }
    Ok(value.to_ascii_lowercase())
}

struct DownloadVerification<'a> {
    content_length: Option<u64>,
    expected_size: u64,
    max_download_bytes: u64,
    base_usage: u64,
    max_server_disk_bytes: u64,
    expected_sha512: Option<&'a str>,
    expected_sha1: Option<&'a str>,
}

async fn stream_and_verify<S, E, B, W>(
    mut stream: S,
    verification: DownloadVerification<'_>,
    output: &mut W,
) -> ApiResult<u64>
where
    S: Stream<Item = Result<B, E>> + Unpin,
    E: std::fmt::Display,
    B: AsRef<[u8]>,
    W: AsyncWrite + Unpin,
{
    let mut sha512 = verification.expected_sha512.map(|_| Sha512::new());
    let mut sha1 = verification.expected_sha1.map(|_| Sha1::new());
    let mut written = 0_u64;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| {
            ApiError::Internal(anyhow::anyhow!("Modrinth download failed: {error}"))
        })?;
        let bytes = chunk.as_ref();
        let next = written
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| ApiError::Conflict("Modrinth file exceeds the download limit".into()))?;
        if next > verification.max_download_bytes
            || verification.base_usage.saturating_add(next) > verification.max_server_disk_bytes
        {
            return Err(ApiError::Conflict("server storage quota exceeded".into()));
        }
        if let Some(hasher) = sha512.as_mut() {
            hasher.update(bytes);
        }
        if let Some(hasher) = sha1.as_mut() {
            hasher.update(bytes);
        }
        output
            .write_all(bytes)
            .await
            .map_err(|error| ApiError::Internal(error.into()))?;
        written = next;
    }
    if let Some(length) = verification.content_length {
        if length != written {
            return Err(ApiError::BadRequest(
                "Modrinth download was truncated".into(),
            ));
        }
    }
    if verification.expected_size > 0 && verification.expected_size != written {
        return Err(ApiError::BadRequest(
            "Modrinth file size did not match its metadata".into(),
        ));
    }
    if let Some(expected) = verification.expected_sha512 {
        let actual = digest_hex(sha512.expect("sha512 hasher is present").finalize());
        if !actual.eq_ignore_ascii_case(expected) {
            return Err(ApiError::Conflict(
                "Modrinth checksum verification failed".into(),
            ));
        }
    } else if let Some(expected) = verification.expected_sha1 {
        let actual = digest_hex(sha1.expect("sha1 hasher is present").finalize());
        if !actual.eq_ignore_ascii_case(expected) {
            return Err(ApiError::Conflict(
                "Modrinth checksum verification failed".into(),
            ));
        }
    }
    output
        .flush()
        .await
        .map_err(|error| ApiError::Internal(error.into()))?;
    Ok(written)
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
    let project = project_id(body.project.trim())?;

    let loaders = loader
        .names
        .iter()
        .map(|name| format!("\"{name}\""))
        .collect::<Vec<_>>()
        .join(",");

    let response = http
        .get(format!("{MODRINTH}/project/{project}/version"))
        .query(&[
            ("game_versions", format!("[\"{}\"]", record.config.version)),
            ("loaders", format!("[{loaders}]")),
        ])
        .send()
        .await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("modrinth versions: {e}")))?;

    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Err(ApiError::NotFound(format!("modrinth project {}", project)));
    }
    if !response.status().is_success() {
        return Err(ApiError::BadRequest(
            "Modrinth version lookup was not successful".into(),
        ));
    }

    let mut versions: Vec<Version> = response
        .json()
        .await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("modrinth versions: {e}")))?;

    if versions.is_empty() {
        return Err(ApiError::BadRequest(format!(
            "{} has no release for {} on {}",
            project, record.config.version, record.config.core
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

    let filename = crate::filesystem::filename(&file.filename)?.to_owned();
    let url = validate_download_url(&file.url).await?;
    let expected_size = file.size;
    if expected_size > state.limits.max_download_bytes {
        return Err(ApiError::Conflict(
            "Modrinth file exceeds the download limit".into(),
        ));
    }
    let expected_sha512 = file
        .hashes
        .sha512
        .as_deref()
        .map(|hash| expected_hash(hash, 64))
        .transpose()?;
    let expected_sha1 = file
        .hashes
        .sha1
        .as_deref()
        .map(|hash| expected_hash(hash, 20))
        .transpose()?;
    if expected_sha512.is_none() && expected_sha1.is_none() {
        return Err(ApiError::BadRequest(
            "Modrinth did not provide a checksum for this file".into(),
        ));
    }
    let _resource_lock = state.resource_lock.lock().await;
    let directory = std::path::PathBuf::from(loader.directory);
    let target = directory.join(&filename);
    let fs = crate::filesystem::open(record.config.directory.clone()).await?;
    fs.create_dir_all(&directory)
        .map_err(|error| ApiError::Internal(error.into()))?;
    let existing = fs
        .directory_size(".")
        .map_err(|error| ApiError::Internal(error.into()))?;
    let replaced_size = fs
        .metadata(&target)
        .ok()
        .filter(|metadata| metadata.is_file)
        .map(|metadata| metadata.len)
        .unwrap_or(0);
    let base_usage = existing.saturating_sub(replaced_size);
    if base_usage > state.limits.max_server_disk_bytes {
        return Err(ApiError::Conflict("server storage quota exceeded".into()));
    }

    let temporary = directory.join(format!(".mcpanel-mod-{}", uuid::Uuid::new_v4().simple()));
    let output = fs
        .create_new_file(&temporary)
        .map_err(|error| ApiError::Internal(error.into()))?;
    let mut output = tokio::fs::File::from_std(output);
    let result: ApiResult<u64> = async {
        let response = http.get(url).send().await.map_err(|error| {
            ApiError::Internal(anyhow::anyhow!("Modrinth download failed: {error}"))
        })?;
        if !response.status().is_success() {
            return Err(ApiError::BadRequest(
                "Modrinth download was not successful".into(),
            ));
        }
        let content_length = response.content_length();
        if content_length.is_some_and(|length| length > state.limits.max_download_bytes) {
            return Err(ApiError::Conflict(
                "Modrinth file exceeds the download limit".into(),
            ));
        }

        let written = stream_and_verify(
            response.bytes_stream(),
            DownloadVerification {
                content_length,
                expected_size,
                max_download_bytes: state.limits.max_download_bytes,
                base_usage,
                max_server_disk_bytes: state.limits.max_server_disk_bytes,
                expected_sha512: expected_sha512.as_deref(),
                expected_sha1: expected_sha1.as_deref(),
            },
            &mut output,
        )
        .await?;
        output
            .sync_data()
            .await
            .map_err(|error| ApiError::Internal(error.into()))?;
        Ok(written)
    }
    .await;
    drop(output);
    let size = match result {
        Ok(size) => size,
        Err(error) => {
            let _ = fs.remove(&temporary);
            return Err(error);
        }
    };
    if let Err(error) = fs.replace_file(&temporary, &target) {
        let _ = fs.remove(&temporary);
        return Err(ApiError::Internal(error.into()));
    }

    tracing::info!(server = %id, file = %filename, "add-on installed");

    Ok(Json(InstallResult {
        name: version.name,
        version: version.version_number,
        path: format!("{}/{}", loader.directory, filename),
        size,
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
    let fs = crate::filesystem::open(record.config.directory.clone()).await?;
    let directory = std::path::PathBuf::from(loader.directory);
    let entries = tokio::task::spawn_blocking(move || fs.read_dir(&directory))
        .await
        .map_err(|error| ApiError::Internal(anyhow::anyhow!("installed add-ons task: {error}")))?;
    let entries = match entries {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Json(Vec::new())),
        Err(error) => return Err(ApiError::Internal(error.into())),
    };

    let mut out = entries
        .into_iter()
        .filter_map(|(name, metadata)| {
            if !metadata.is_file {
                return None;
            }
            let name = name.to_str()?.to_owned();
            if !name.ends_with(".jar") {
                return None;
            }
            Some(InstalledView {
                path: format!("{}/{}", loader.directory, name),
                filename: name,
                size: metadata.len,
            })
        })
        .collect::<Vec<_>>();

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

    #[test]
    fn download_urls_are_restricted_to_https_modrinth_hosts() {
        for raw in [
            "http://cdn.modrinth.com/file.jar",
            "file:///etc/passwd",
            "https://127.0.0.1/file.jar",
            "https://evil.example/file.jar",
            "https://evil.modrinth.com/file.jar",
        ] {
            let url = reqwest::Url::parse(raw).unwrap();
            assert!(!is_trusted_download_url(&url), "{raw} must be rejected");
        }
        for raw in [
            "https://cdn.modrinth.com/data/a/file.jar",
            "https://api.modrinth.com/v2/file.jar",
        ] {
            let url = reqwest::Url::parse(raw).unwrap();
            assert!(is_trusted_download_url(&url), "{raw} should be accepted");
        }
    }

    #[test]
    fn project_ids_and_hashes_are_bounded_and_well_formed() {
        assert_eq!(project_id("sodium").unwrap(), "sodium");
        assert!(project_id("../panel.json").is_err());
        assert!(project_id(&"x".repeat(129)).is_err());
        assert!(expected_hash(&"a".repeat(128), 64).is_ok());
        assert!(expected_hash(&"a".repeat(40), 20).is_ok());
        assert!(expected_hash("not-a-hash", 20).is_err());
    }

    #[tokio::test]
    async fn streamed_download_validation_rejects_bad_size_hash_failure_and_overflow() {
        async fn attempt(
            chunks: Vec<Result<Vec<u8>, std::io::Error>>,
            content_length: Option<u64>,
            expected_size: u64,
            max_download_bytes: u64,
            expected_sha512: Option<&str>,
        ) -> ApiResult<u64> {
            let tmp = tempfile::tempdir().unwrap();
            let fs = guardian::ScopedFs::open(tmp.path()).unwrap();
            let output = fs.create_new_file("download.tmp").unwrap();
            let mut output = tokio::fs::File::from_std(output);
            stream_and_verify(
                futures_util::stream::iter(chunks),
                DownloadVerification {
                    content_length,
                    expected_size,
                    max_download_bytes,
                    base_usage: 0,
                    max_server_disk_bytes: u64::MAX,
                    expected_sha512,
                    expected_sha1: None,
                },
                &mut output,
            )
            .await
        }

        let body = b"streamed plugin bytes".to_vec();
        let hash = digest_hex(Sha512::digest(&body));
        let bad_hash = "00".repeat(64);
        assert_eq!(
            attempt(
                vec![Ok(body.clone())],
                Some(body.len() as u64),
                body.len() as u64,
                body.len() as u64,
                Some(&hash),
            )
            .await
            .unwrap(),
            body.len() as u64
        );

        assert!(matches!(
            attempt(
                vec![Ok(body.clone())],
                Some(body.len() as u64),
                body.len() as u64 + 1,
                u64::MAX,
                Some(&hash),
            )
            .await,
            Err(ApiError::BadRequest(_))
        ));
        assert!(matches!(
            attempt(
                vec![Ok(body.clone())],
                Some(body.len() as u64 + 1),
                0,
                u64::MAX,
                Some(&hash),
            )
            .await,
            Err(ApiError::BadRequest(_))
        ));
        assert!(matches!(
            attempt(
                vec![Ok(body.clone())],
                None,
                body.len() as u64,
                u64::MAX,
                Some(&bad_hash),
            )
            .await,
            Err(ApiError::Conflict(_))
        ));
        assert!(matches!(
            attempt(
                vec![Ok(body.clone())],
                None,
                0,
                (body.len() - 1) as u64,
                Some(&hash),
            )
            .await,
            Err(ApiError::Conflict(_))
        ));
        assert!(matches!(
            attempt(
                vec![Err(std::io::Error::other("connection reset"))],
                None,
                0,
                u64::MAX,
                Some(&hash),
            )
            .await,
            Err(ApiError::Internal(_))
        ));
    }

    #[tokio::test]
    async fn failed_streamed_download_does_not_replace_the_existing_addon() {
        let tmp = tempfile::tempdir().unwrap();
        let fs = guardian::ScopedFs::open(tmp.path()).unwrap();
        fs.create_dir_all("plugins").unwrap();
        fs.write_atomic("plugins/example.jar", b"old addon")
            .unwrap();
        let bad_hash = "00".repeat(64);
        let output = fs.create_new_file("plugins/.download.tmp").unwrap();
        let mut output = tokio::fs::File::from_std(output);
        let result = stream_and_verify(
            futures_util::stream::iter(vec![Ok::<_, std::io::Error>(b"new addon".to_vec())]),
            DownloadVerification {
                content_length: None,
                expected_size: 0,
                max_download_bytes: u64::MAX,
                base_usage: 0,
                max_server_disk_bytes: u64::MAX,
                expected_sha512: Some(&bad_hash),
                expected_sha1: None,
            },
            &mut output,
        )
        .await;
        drop(output);
        fs.remove("plugins/.download.tmp").unwrap();

        assert!(matches!(result, Err(ApiError::Conflict(_))));
        assert_eq!(fs.read_file("plugins/example.jar").unwrap(), b"old addon");
    }
}
