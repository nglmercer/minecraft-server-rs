//! Centralized Java discovery and trust.
//!
//! Every Java question the panel asks — the catalog endpoint, first-install
//! provisioning, reinstalls — goes through [`JavaResolver`], which caches one
//! successful discovery pass for [`DISCOVERY_TTL`]. Java installations do not
//! change every second, and a server restart must never pay for six registry
//! reads again.
//!
//! The warm-start path ([`trusted_installation`](crate::environment)) never
//! touches the resolver at all: it validates the recorded Java against the
//! local filesystem only, so an ordinary Start spawns exactly one process —
//! the Minecraft JVM.

use java_path::JavaInstallation;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How long a successful discovery pass is reused before a refresh.
///
/// Explicit [`JavaResolver::invalidate`] (operator refresh, reinstall) always
/// wins over the TTL; the TTL only bounds how stale an untouched cache gets.
pub const DISCOVERY_TTL: Duration = Duration::from_secs(60);

/// One full machine-wide Java discovery pass.
///
/// The trait exists so tests can prove how often discovery runs: production
/// uses [`SystemDiscovery`], tests inject a counting fake. A backend must
/// never spawn a *visible* helper on Windows — `java-path` routes its own
/// probes through its hidden-command helper — but backends are still expected
/// to be silent by construction.
pub trait DiscoveryBackend: Send + Sync + std::fmt::Debug {
    /// Discover every Java installation on this machine.
    fn discover(&self) -> Result<Vec<JavaInstallation>, java_path::Error>;
}

/// The production backend: a real `java-path` discovery pass.
#[derive(Debug, Default)]
pub struct SystemDiscovery;

impl DiscoveryBackend for SystemDiscovery {
    fn discover(&self) -> Result<Vec<JavaInstallation>, java_path::Error> {
        system_discovery(
            java_path::Platform::current(),
            java_path::discovery::termux::prefix(),
        )
        .search()
    }
}

fn system_discovery(
    platform: java_path::Platform,
    termux_prefix: Option<PathBuf>,
) -> java_path::Discovery {
    let discovery = java_path::Discovery::new();
    // Recent Termux packages keep versioned JDKs under lib/jvm. Include
    // all of them, even when PATH/JAVA_HOME selects a different major.
    if platform == java_path::Platform::Termux {
        let prefix =
            termux_prefix.unwrap_or_else(|| PathBuf::from("/data/data/com.termux/files/usr"));
        discovery.root(prefix.join("lib/jvm"))
    } else {
        discovery
    }
}

#[derive(Debug)]
struct CacheState {
    at: Option<Instant>,
    installs: Vec<JavaInstallation>,
}

impl CacheState {
    fn empty() -> Self {
        CacheState {
            at: None,
            installs: Vec::new(),
        }
    }
}

/// Shared, cached Java discovery for one panel process.
///
/// Cheap to clone behind an `Arc`: the catalog handler, every guardian and
/// every reinstall share a single instance owned by the panel state, so
/// opening the Java catalog and starting a server never triggers two
/// independent machine-wide scans.
#[derive(Debug)]
pub struct JavaResolver {
    backend: Arc<dyn DiscoveryBackend>,
    data_dir: PathBuf,
    cache: Mutex<CacheState>,
    ttl: Duration,
    passes: AtomicU64,
}

impl JavaResolver {
    /// A resolver over real system discovery, rooted at `data_dir`.
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self::with_backend(data_dir, Arc::new(SystemDiscovery))
    }

    /// A resolver over an injected backend. Production always uses [`Self::new`];
    /// this exists so tests can count discovery passes deterministically.
    pub fn with_backend(data_dir: impl Into<PathBuf>, backend: Arc<dyn DiscoveryBackend>) -> Self {
        JavaResolver {
            backend,
            data_dir: data_dir.into(),
            cache: Mutex::new(CacheState::empty()),
            ttl: DISCOVERY_TTL,
            passes: AtomicU64::new(0),
        }
    }

    /// Override the cache TTL (tests use a long TTL to freeze the cache).
    pub fn with_ttl(mut self, ttl: Duration) -> Self {
        self.ttl = ttl;
        self
    }

    /// The panel data directory this resolver provisions managed JDKs under.
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// Every known Java installation, reusing the cached pass while fresh.
    ///
    /// At most one backend pass runs per TTL window no matter how many
    /// callers ask: a burst of catalog refreshes and server starts shares a
    /// single scan. Failed passes are never cached.
    pub fn javas(&self) -> Result<Vec<JavaInstallation>, java_path::Error> {
        {
            let cache = self
                .cache
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(at) = cache.at {
                if at.elapsed() < self.ttl {
                    return Ok(cache.installs.clone());
                }
            }
        }
        let installs = self.backend.discover()?;
        self.passes.fetch_add(1, Ordering::Relaxed);
        {
            let mut cache = self
                .cache
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            cache.at = Some(Instant::now());
            cache.installs = installs.clone();
        }
        Ok(installs)
    }

    /// Drop the cached pass. The next [`Self::javas`] scans again.
    ///
    /// Called for explicit operator refreshes and before reinstalls, which
    /// must see a JDK that arrived after the cache was filled. Ordinary
    /// server Starts must never call this.
    pub fn invalidate(&self) {
        let mut cache = self
            .cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        cache.at = None;
        cache.installs = Vec::new();
    }

    /// How many successful backend discovery passes have run.
    ///
    /// The regression invariant for a warm Start is `0` new passes; a burst
    /// of catalog reads is `1`.
    pub fn discovery_passes(&self) -> u64 {
        self.passes.load(Ordering::Relaxed)
    }
}

/// A [`DiscoveryBackend`] that replays a fixed list and counts invocations.
///
/// Test-only: lets the suite assert that warm Starts perform zero discovery
/// passes and that catalog bursts share one.
#[cfg(test)]
pub(crate) struct CountingDiscovery {
    installs: Vec<JavaInstallation>,
    calls: AtomicU64,
}

#[cfg(test)]
impl CountingDiscovery {
    pub(crate) fn new(installs: Vec<JavaInstallation>) -> Self {
        CountingDiscovery {
            installs,
            calls: AtomicU64::new(0),
        }
    }

    pub(crate) fn calls(&self) -> u64 {
        self.calls.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
impl std::fmt::Debug for CountingDiscovery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CountingDiscovery")
            .field("installs", &self.installs.len())
            .field("calls", &self.calls())
            .finish()
    }
}

#[cfg(test)]
impl DiscoveryBackend for CountingDiscovery {
    fn discover(&self) -> Result<Vec<JavaInstallation>, java_path::Error> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        Ok(self.installs.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use java_path::{Architecture, DiscoverySource, JavaKind, JavaVersion, Platform};
    use std::time::Duration;

    pub(crate) fn installation(major: u32) -> JavaInstallation {
        JavaInstallation {
            home: PathBuf::from(format!("/host/jdks/java-{major}")),
            java: PathBuf::from(format!("/host/jdks/java-{major}/bin/java")),
            javac: Some(PathBuf::from(format!("/host/jdks/java-{major}/bin/javac"))),
            version: JavaVersion::parse(&format!("{major}.0.1")).unwrap(),
            vendor: Some("Test Vendor".into()),
            architecture: Architecture::X86_64,
            platform: Platform::Linux,
            kind: JavaKind::Jdk,
            source: DiscoverySource::UserDirectory,
        }
    }

    fn resolver_with(installs: Vec<JavaInstallation>) -> (JavaResolver, Arc<CountingDiscovery>) {
        let backend = Arc::new(CountingDiscovery::new(installs));
        let resolver =
            JavaResolver::with_backend("/data", backend.clone()).with_ttl(Duration::from_secs(60));
        (resolver, backend)
    }

    #[test]
    fn termux_discovers_all_versioned_jdks_without_path_or_java_home() {
        let tmp = tempfile::tempdir().unwrap();
        for major in [17, 21] {
            let home = tmp.path().join(format!("lib/jvm/java-{major}-openjdk"));
            std::fs::create_dir_all(home.join("bin")).unwrap();
            let launcher = if cfg!(windows) { "java.exe" } else { "java" };
            std::fs::write(home.join("bin").join(launcher), b"not executed").unwrap();
            std::fs::write(
                home.join("release"),
                format!("JAVA_VERSION=\"{major}.0.1\"\nOS_ARCH=\"aarch64\"\nOS_NAME=\"Linux\"\n"),
            )
            .unwrap();
        }
        let mut installs = system_discovery(java_path::Platform::Termux, Some(tmp.path().into()))
            .java_home(false)
            .path(false)
            .system_dirs(false)
            .tool_stores(false)
            .inspect_options(java_path::InspectOptions::strict())
            .search()
            .unwrap();
        installs.sort_by_key(JavaInstallation::major);
        assert_eq!(
            installs
                .iter()
                .map(JavaInstallation::major)
                .collect::<Vec<_>>(),
            [17, 21]
        );
    }

    #[test]
    fn a_burst_of_reads_shares_a_single_discovery_pass() {
        let (resolver, backend) = resolver_with(vec![installation(21)]);

        for _ in 0..5 {
            let installs = resolver.javas().unwrap();
            assert_eq!(installs.len(), 1);
        }

        assert_eq!(backend.calls(), 1);
        assert_eq!(resolver.discovery_passes(), 1);
    }

    #[test]
    fn explicit_invalidation_forces_exactly_one_refresh() {
        let (resolver, backend) = resolver_with(vec![installation(21)]);

        resolver.javas().unwrap();
        resolver.javas().unwrap();
        assert_eq!(backend.calls(), 1);

        resolver.invalidate();
        resolver.javas().unwrap();
        assert_eq!(backend.calls(), 2);
        assert_eq!(resolver.discovery_passes(), 2);
    }

    #[test]
    fn an_expired_entry_is_refreshed() {
        let backend = Arc::new(CountingDiscovery::new(vec![installation(21)]));
        let resolver =
            JavaResolver::with_backend("/data", backend.clone()).with_ttl(Duration::from_nanos(1));

        resolver.javas().unwrap();
        std::thread::sleep(Duration::from_millis(5));
        resolver.javas().unwrap();

        assert_eq!(backend.calls(), 2);
    }
}
