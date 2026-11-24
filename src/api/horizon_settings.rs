//! Per-server Horizon settings that decide which clients may join.
//!
//! These live in the server's own `server.toml` and `server.properties`, not in
//! the panel's `panel.json`: Horizon owns their semantics and an operator may
//! also edit the files directly. The API only reads and writes the three keys
//! the panel exposes, preserving every other setting.

use axum::extract::{Path, State};
use axum::routing::get;
use axum::{Json, Router};
use guardian::ScopedFs;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::auth::Identity;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use crate::store::ServerRecord;

/// Defaults Horizon uses when a key is absent (hosting.md, server.toml).
const DEFAULT_ONLINE_MODE: bool = true;
const DEFAULT_ACCEPT_OLDER_CLIENTS: bool = false;
const DEFAULT_ENFORCE_SECURE_PROFILE: bool = true;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct HorizonSettingsView {
    pub online_mode: bool,
    pub accept_older_clients: bool,
    pub enforce_secure_profile: bool,
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct HorizonSettingsUpdate {
    pub online_mode: Option<bool>,
    pub accept_older_clients: Option<bool>,
    pub enforce_secure_profile: Option<bool>,
}

fn io_error(path: &str, error: std::io::Error) -> ApiError {
    ApiError::Guardian(guardian::Error::io(path, error))
}

#[cfg(test)]
fn defaults() -> HorizonSettingsView {
    HorizonSettingsView {
        online_mode: DEFAULT_ONLINE_MODE,
        accept_older_clients: DEFAULT_ACCEPT_OLDER_CLIENTS,
        enforce_secure_profile: DEFAULT_ENFORCE_SECURE_PROFILE,
    }
}

fn boolean(table: &toml_edit::Table, key: &str, default: bool) -> ApiResult<bool> {
    match table.get(key) {
        None => Ok(default),
        Some(item) => item.as_bool().ok_or_else(|| {
            ApiError::BadRequest(format!("server.toml: server.{key} must be a boolean"))
        }),
    }
}

fn read(fs: &ScopedFs) -> ApiResult<HorizonSettingsView> {
    let toml_bytes = fs
        .read_file("server.toml")
        .map_err(|e| io_error("server.toml", e))?;
    let text = std::str::from_utf8(&toml_bytes)
        .map_err(|_| ApiError::BadRequest("server.toml must be UTF-8".into()))?;
    let document = text
        .parse::<toml_edit::DocumentMut>()
        .map_err(|error| ApiError::BadRequest(format!("invalid server.toml: {error}")))?;
    let server = document
        .get("server")
        .and_then(toml_edit::Item::as_table)
        .ok_or_else(|| ApiError::BadRequest("server.toml: [server] must be a table".into()))?;

    // An absent properties file means vanilla's default: require signed chat.
    let properties = match fs.read_file("server.properties") {
        Ok(bytes) => String::from_utf8(bytes)
            .map_err(|_| ApiError::BadRequest("server.properties must be UTF-8".into()))?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(io_error("server.properties", error)),
    };

    Ok(HorizonSettingsView {
        online_mode: boolean(server, "online_mode", DEFAULT_ONLINE_MODE)?,
        accept_older_clients: boolean(
            server,
            "accept_older_clients",
            DEFAULT_ACCEPT_OLDER_CLIENTS,
        )?,
        enforce_secure_profile: property(&properties, "enforce-secure-profile")
            .map(|value| match value {
                "true" => Ok(true),
                "false" => Ok(false),
                other => Err(ApiError::BadRequest(format!(
                    "server.properties: enforce-secure-profile must be true or false, found {other}"
                ))),
            })
            .transpose()?
            .unwrap_or(DEFAULT_ENFORCE_SECURE_PROFILE),
    })
}

fn write(fs: &ScopedFs, view: HorizonSettingsView) -> ApiResult<()> {
    let toml_bytes = fs
        .read_file("server.toml")
        .map_err(|e| io_error("server.toml", e))?;
    let text = std::str::from_utf8(&toml_bytes)
        .map_err(|_| ApiError::BadRequest("server.toml must be UTF-8".into()))?;
    let mut document = text
        .parse::<toml_edit::DocumentMut>()
        .map_err(|error| ApiError::BadRequest(format!("invalid server.toml: {error}")))?;
    let section = document.entry("server").or_insert(toml_edit::table());
    let server = section
        .as_table_mut()
        .ok_or_else(|| ApiError::BadRequest("server.toml: [server] must be a table".into()))?;
    server.insert("online_mode", toml_edit::value(view.online_mode));
    server.insert(
        "accept_older_clients",
        toml_edit::value(view.accept_older_clients),
    );
    fs.write_atomic("server.toml", document.to_string().as_bytes())
        .map_err(|e| io_error("server.toml", e))?;

    let properties = match fs.read_file("server.properties") {
        Ok(bytes) => String::from_utf8(bytes)
            .map_err(|_| ApiError::BadRequest("server.properties must be UTF-8".into()))?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(io_error("server.properties", error)),
    };
    let updated = set_property(
        &properties,
        "enforce-secure-profile",
        if view.enforce_secure_profile {
            "true"
        } else {
            "false"
        },
    );
    fs.write_atomic("server.properties", updated.as_bytes())
        .map_err(|e| io_error("server.properties", e))?;
    Ok(())
}

/// Read one `.properties` key, ignoring commented-out lines.
fn property<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    text.lines().find_map(|line| {
        if line.trim_start().starts_with('#') {
            return None;
        }
        line.split_once('=')
            .filter(|(name, _)| name.trim() == key)
            .map(|(_, value)| value.trim())
    })
}

/// Replace one key in place, keeping comments, ordering and unrelated keys.
fn set_property(text: &str, key: &str, value: &str) -> String {
    let mut out = String::with_capacity(text.len() + key.len() + value.len() + 2);
    let mut replaced = false;
    for line in text.lines() {
        let is_target = line
            .split_once('=')
            .is_some_and(|(name, _)| name.trim() == key)
            && !line.trim_start().starts_with('#');
        if is_target && !replaced {
            out.push_str(&format!("{key}={value}"));
            replaced = true;
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }
    if !replaced {
        out.push_str(&format!("{key}={value}\n"));
    }
    out
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

fn horizon_only(record: &ServerRecord) -> ApiResult<()> {
    if !record.config.core.eq_ignore_ascii_case("horizon") {
        return Err(ApiError::BadRequest(
            "these settings apply only to Horizon servers".into(),
        ));
    }
    Ok(())
}

async fn get_settings(
    State(state): State<Arc<AppState>>,
    identity: Identity,
    Path(id): Path<String>,
) -> ApiResult<Json<HorizonSettingsView>> {
    let record = authorized(&state, &identity, &id).await?;
    horizon_only(&record)?;
    let fs =
        ScopedFs::open(&record.config.directory).map_err(|e| io_error("server directory", e))?;
    Ok(Json(read(&fs)?))
}

async fn patch_settings(
    State(state): State<Arc<AppState>>,
    identity: Identity,
    Path(id): Path<String>,
    Json(body): Json<HorizonSettingsUpdate>,
) -> ApiResult<Json<HorizonSettingsView>> {
    let record = authorized(&state, &identity, &id).await?;
    horizon_only(&record)?;
    let fs =
        ScopedFs::open(&record.config.directory).map_err(|e| io_error("server directory", e))?;
    let mut view = read(&fs)?;
    if let Some(value) = body.online_mode {
        view.online_mode = value;
    }
    if let Some(value) = body.accept_older_clients {
        view.accept_older_clients = value;
    }
    if let Some(value) = body.enforce_secure_profile {
        view.enforce_secure_profile = value;
    }
    write(&fs, view)?;
    Ok(Json(read(&fs)?))
}

pub fn server_router() -> Router<Arc<AppState>> {
    Router::new().route(
        "/{id}/horizon-settings",
        get(get_settings).patch(patch_settings),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_horizon() {
        assert_eq!(
            defaults(),
            HorizonSettingsView {
                online_mode: true,
                accept_older_clients: false,
                enforce_secure_profile: true,
            }
        );
    }

    #[test]
    fn properties_are_read_without_touching_comments_or_unrelated_keys() {
        let text = "# keep this\nmotd=Hello\nenforce-secure-profile=false\n# enforce-secure-profile=true\n";
        assert_eq!(property(text, "enforce-secure-profile"), Some("false"));
        let updated = set_property(text, "enforce-secure-profile", "true");
        assert!(updated.starts_with("# keep this\nmotd=Hello\n"));
        assert!(updated.contains("enforce-secure-profile=true\n"));
        assert!(updated.contains("# enforce-secure-profile=true\n"));
    }

    #[test]
    fn a_missing_property_is_appended() {
        assert_eq!(
            set_property("motd=Hello\n", "enforce-secure-profile", "false"),
            "motd=Hello\nenforce-secure-profile=false\n"
        );
    }

    #[test]
    fn settings_round_trip_preserves_other_toml_and_properties_keys() {
        let root = tempfile::tempdir().unwrap();
        let fs = ScopedFs::open(root.path()).unwrap();
        fs.write_atomic(
            "server.toml",
            b"[server]\nmotd = 'home'\nonline_mode = false\n[world]\nname = 'existing'\n",
        )
        .unwrap();
        fs.write_atomic(
            "server.properties",
            b"motd=Hello\nenforce-secure-profile=true\n",
        )
        .unwrap();
        let changed = HorizonSettingsView {
            online_mode: true,
            accept_older_clients: true,
            enforce_secure_profile: false,
        };
        write(&fs, changed).unwrap();
        assert_eq!(read(&fs).unwrap(), changed);
        let toml = String::from_utf8(fs.read_file("server.toml").unwrap()).unwrap();
        assert!(toml.contains("motd = 'home'"));
        assert!(toml.contains("[world]"));
        let properties = String::from_utf8(fs.read_file("server.properties").unwrap()).unwrap();
        assert!(properties.contains("motd=Hello\n"));
    }
}
