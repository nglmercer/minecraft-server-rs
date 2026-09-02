//! Bind address resolution, port-file handling, single-instance lock and fallback.
//!
//! Default is `127.0.0.1:8080`. The effective bind is resolved as:
//!   1. `--bind` / `MCPANEL_BIND` when provided (CLI wins)
//!   2. Optional user-editable file `<data-dir>/port` or `<data-dir>/bind`
//!   3. Fallback to `127.0.0.1:8080`
//!
//! The file may contain `8080` or `127.0.0.1:8080` (also `port = 8080` form).
//! An active instance writes `<data-dir>/.lock` with `pid`/`bind`/`url` for
//! single-instance detection. When the requested port is in use the code probes
//! the lock: if a live panel is found the second launch opens the browser to
//! the existing URL and exits; otherwise the next free port `+1..+20` is tried.

use anyhow::{Context, Result};
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};

const DEFAULT_BIND: &str = "127.0.0.1:8080";

pub fn default_bind_addr() -> SocketAddr {
    DEFAULT_BIND.parse().expect("default bind is valid")
}

/// Optional user-editable port override at `<data-dir>/port` or `<data-dir>/bind`.
pub fn read_bind_file(data_dir: &Path) -> Option<SocketAddr> {
    for name in ["port", "bind", ".port", ".bind"] {
        let path = data_dir.join(name);
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        for raw in content.lines() {
            let trimmed = raw.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let value = if let Some((_, after)) = trimmed.split_once('=') {
                after.trim()
            } else {
                trimmed
            };
            if value.is_empty() {
                continue;
            }
            if let Ok(addr) = value.parse::<SocketAddr>() {
                tracing::info!("using bind from {}: {}", path.display(), addr);
                return Some(addr);
            }
            if let Ok(port) = value.parse::<u16>() {
                let addr = SocketAddr::from((IpAddr::from([127, 0, 0, 1]), port));
                tracing::info!("using bind from {}: {}", path.display(), addr);
                return Some(addr);
            }
            tracing::warn!(
                "ignoring invalid bind in {}: {:?} (expected PORT or IP:PORT)",
                path.display(),
                trimmed
            );
            break;
        }
    }
    None
}

pub fn resolve_effective_bind(cli_bind: Option<SocketAddr>, data_dir: &Path) -> SocketAddr {
    if let Some(addr) = cli_bind {
        return addr;
    }
    if let Some(addr) = read_bind_file(data_dir) {
        return addr;
    }
    default_bind_addr()
}

pub fn instance_lock_path(data_dir: &Path) -> PathBuf {
    data_dir.join(".lock")
}

pub fn panel_url(address: SocketAddr) -> String {
    match address {
        SocketAddr::V4(addr) if addr.ip().is_unspecified() => {
            format!("http://127.0.0.1:{}", addr.port())
        }
        SocketAddr::V6(addr) if addr.ip().is_unspecified() => {
            format!("http://[::1]:{}", addr.port())
        }
        SocketAddr::V4(addr) => format!("http://{addr}"),
        SocketAddr::V6(addr) => format!("http://[{}]:{}", addr.ip(), addr.port()),
    }
}

pub async fn write_instance_lock(data_dir: &Path, bind: SocketAddr) -> Result<()> {
    let path = instance_lock_path(data_dir);
    let url = panel_url(bind);
    let pid = std::process::id();
    let payload = serde_json::json!({
        "pid": pid,
        "bind": bind.to_string(),
        "url": url,
    });
    let json = serde_json::to_vec_pretty(&payload)?;
    let tmp = path.with_extension("lock.tmp");
    tokio::fs::write(&tmp, &json)
        .await
        .with_context(|| format!("writing {}", tmp.display()))?;
    tokio::fs::rename(&tmp, &path)
        .await
        .with_context(|| format!("activating {}", path.display()))?;
    Ok(())
}

pub async fn remove_instance_lock(data_dir: &Path) {
    let path = instance_lock_path(data_dir);
    let current_pid = std::process::id();
    if let Ok(bytes) = tokio::fs::read(&path).await {
        if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&bytes) {
            if let Some(pid) = v.get("pid").and_then(|p| p.as_u64()) {
                if pid != current_pid as u64 {
                    return;
                }
            }
        }
    }
    let _ = tokio::fs::remove_file(&path).await;
}

pub async fn probe_existing_instance(data_dir: &Path) -> Option<String> {
    let path = instance_lock_path(data_dir);
    let Ok(bytes) = tokio::fs::read(&path).await else {
        return None;
    };
    let v: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    let url = v.get("url").and_then(|u| u.as_str())?.to_string();
    let bind_str = v.get("bind").and_then(|b| b.as_str())?;
    let bind: SocketAddr = bind_str.parse().ok()?;
    let probe = tokio::time::timeout(
        std::time::Duration::from_millis(500),
        tokio::net::TcpStream::connect(bind),
    )
    .await;
    if probe.is_err() {
        return None;
    }
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(700))
        .build()
        .ok()?;
    let resp = tokio::time::timeout(
        std::time::Duration::from_millis(800),
        client.get(&url).send(),
    )
    .await
    .ok()?
    .ok()?;
    if resp.status().is_success() {
        Some(url)
    } else {
        None
    }
}

fn open_browser(url: &str) {
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("rundll32.exe")
            .args(["url.dll,FileProtocolHandler", url])
            .spawn();
        return;
    }
    #[cfg(all(target_os = "linux", not(target_env = "musl")))]
    {
        let candidates: [&[&str]; 4] = [
            &["xdg-open", url],
            &["gio", "open", url],
            &["kde-open5", url],
            &["sensible-browser", url],
        ];
        for candidate in candidates {
            if std::process::Command::new(candidate[0])
                .args(&candidate[1..])
                .spawn()
                .is_ok()
            {
                return;
            }
        }
        tracing::warn!("could not open browser for setup/recovery");
        return;
    }
    #[cfg(not(any(windows, all(target_os = "linux", not(target_env = "musl")))))]
    {
        let _ = url;
        tracing::warn!("browser opening not supported on this platform");
    }
}

pub async fn bind_with_fallback(
    requested: SocketAddr,
    data_dir: &Path,
) -> Result<(tokio::net::TcpListener, SocketAddr)> {
    match tokio::net::TcpListener::bind(requested).await {
        Ok(l) => {
            let actual = l.local_addr().context("reading listener address")?;
            return Ok((l, actual));
        }
        Err(e) if e.kind() != std::io::ErrorKind::AddrInUse => {
            return Err(e).with_context(|| format!("binding {requested}"));
        }
        Err(_) => {
            tracing::warn!("port {} in use", requested);
        }
    }

    if let Some(url) = probe_existing_instance(data_dir).await {
        tracing::info!("another panel appears to be running at {url}");
        open_browser(&url);
        anyhow::bail!(
            "panel already running at {url} (port {} in use); opened browser to existing instance",
            requested.port()
        );
    }

    let ip = requested.ip();
    let start_port = requested.port().wrapping_add(1);
    for offset in 0..20 {
        let port = start_port.wrapping_add(offset);
        if port == 0 {
            continue;
        }
        let candidate = SocketAddr::new(ip, port);
        match tokio::net::TcpListener::bind(candidate).await {
            Ok(l) => {
                let actual = l.local_addr().context("reading listener address")?;
                tracing::warn!(
                    "port {} in use, selected free port {} (override with --bind or {} file)",
                    requested.port(),
                    actual.port(),
                    data_dir.join("port").display()
                );
                return Ok((l, actual));
            }
            Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => continue,
            Err(e) => {
                return Err(e).with_context(|| format!("binding {candidate}"));
            }
        }
    }

    anyhow::bail!(
        "port {} in use and no free port in {}-{}; change {} or pass --bind",
        requested.port(),
        start_port,
        start_port.wrapping_add(19),
        data_dir.join("port").display()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::SocketAddr;

    #[test]
    fn default_bind_is_loopback_8080() {
        assert_eq!(default_bind_addr(), "127.0.0.1:8080".parse().unwrap());
        assert!(default_bind_addr().ip().is_loopback());
    }

    #[test]
    fn effective_bind_prefers_cli_over_file_and_default() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("port"), "9090").unwrap();
        let cli: SocketAddr = "127.0.0.1:7070".parse().unwrap();
        assert_eq!(resolve_effective_bind(Some(cli), tmp.path()), cli);
        assert_eq!(
            resolve_effective_bind(None, tmp.path()),
            "127.0.0.1:9090".parse().unwrap()
        );
        let empty = tempfile::tempdir().unwrap();
        assert_eq!(
            resolve_effective_bind(None, empty.path()),
            default_bind_addr()
        );
    }

    #[test]
    fn port_file_accepts_port_and_socketaddr_and_bind_alias() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("port"), "8081\n").unwrap();
        assert_eq!(
            read_bind_file(tmp.path()).unwrap(),
            "127.0.0.1:8081".parse().unwrap()
        );
        let tmp2 = tempfile::tempdir().unwrap();
        std::fs::write(tmp2.path().join("bind"), "127.0.0.1:9090").unwrap();
        assert_eq!(
            read_bind_file(tmp2.path()).unwrap(),
            "127.0.0.1:9090".parse().unwrap()
        );
        let tmp3 = tempfile::tempdir().unwrap();
        std::fs::write(tmp3.path().join("port"), "port = 7070").unwrap();
        assert_eq!(
            read_bind_file(tmp3.path()).unwrap(),
            "127.0.0.1:7070".parse().unwrap()
        );
        let tmp4 = tempfile::tempdir().unwrap();
        std::fs::write(tmp4.path().join("bind"), "bind = 0.0.0.0:8082").unwrap();
        assert_eq!(
            read_bind_file(tmp4.path()).unwrap(),
            "0.0.0.0:8082".parse().unwrap()
        );
    }

    #[test]
    fn port_file_ignores_comments_and_empty() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("port"), "# comment\n\n  ").unwrap();
        assert!(read_bind_file(tmp.path()).is_none());
        let tmp2 = tempfile::tempdir().unwrap();
        std::fs::write(tmp2.path().join("port"), "# comment\n8085").unwrap();
        assert_eq!(
            read_bind_file(tmp2.path()).unwrap(),
            "127.0.0.1:8085".parse().unwrap()
        );
    }

    #[test]
    fn invalid_port_file_falls_back_to_default() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("port"), "not-a-port").unwrap();
        assert!(read_bind_file(tmp.path()).is_none());
        assert_eq!(
            resolve_effective_bind(None, tmp.path()),
            default_bind_addr()
        );
    }

    #[test]
    fn panel_url_maps_unspecified_to_loopback() {
        assert_eq!(
            panel_url("127.0.0.1:8080".parse().unwrap()),
            "http://127.0.0.1:8080"
        );
        assert_eq!(
            panel_url("0.0.0.0:8080".parse().unwrap()),
            "http://127.0.0.1:8080"
        );
        assert_eq!(
            panel_url("[::]:8080".parse().unwrap()),
            "http://[::1]:8080"
        );
    }

    #[tokio::test]
    async fn bind_fallback_selects_free_port() {
        let occupied = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let occupied_addr = occupied.local_addr().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let (listener, actual) = bind_with_fallback(occupied_addr, tmp.path()).await.unwrap();
        assert_ne!(actual, occupied_addr);
        assert_eq!(actual.ip(), occupied_addr.ip());
        drop(listener);
        drop(occupied);
    }

    #[tokio::test]
    async fn instance_lock_is_written_and_removed() {
        let tmp = tempfile::tempdir().unwrap();
        let bind: SocketAddr = "127.0.0.1:18880".parse().unwrap();
        write_instance_lock(tmp.path(), bind).await.unwrap();
        let path = instance_lock_path(tmp.path());
        assert!(path.exists());
        let data = tokio::fs::read_to_string(&path).await.unwrap();
        let v: serde_json::Value = serde_json::from_str(&data).unwrap();
        assert_eq!(v["bind"], bind.to_string());
        assert_eq!(v["url"], panel_url(bind));
        remove_instance_lock(tmp.path()).await;
        assert!(!path.exists());
    }
}
