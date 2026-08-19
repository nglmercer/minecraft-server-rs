//! Per-process resource usage.
//!
//! Host-level numbers say nothing useful when several servers share a box, so
//! the panel reports each JVM's own CPU and memory.

use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

/// How long a directory size is trusted before it is measured again.
///
/// Walking a large world costs real I/O, and the dashboard polls every few
/// seconds; without this the panel would spend more effort measuring the server
/// than running it.
const DISK_TTL: Duration = Duration::from_secs(60);

/// What one JVM is currently costing.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct ProcessMetrics {
    /// Share of a single core, so a 4-thread server can exceed 100.
    pub cpu_percent: f32,
    /// Resident memory in mebibytes.
    pub memory_mb: u64,
}

/// Samples processes, keeping enough history for CPU deltas to be meaningful.
pub struct Metrics {
    // sysinfo computes CPU usage as a delta against the previous refresh, so
    // the same System instance has to survive between calls.
    system: Mutex<System>,
    disk: Mutex<HashMap<PathBuf, (Instant, u64)>>,
}

impl Default for Metrics {
    fn default() -> Self {
        Metrics { system: Mutex::new(System::new()), disk: Mutex::new(HashMap::new()) }
    }
}

impl Metrics {
    /// Sample `pid`, or `None` if the process has gone.
    ///
    /// The first sample for a process always reports 0% CPU: there is no
    /// previous measurement to compare against, and inventing one would lie.
    pub fn of(&self, pid: u32) -> Option<ProcessMetrics> {
        let pid = Pid::from_u32(pid);
        let mut system = self.system.lock().ok()?;

        system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[pid]),
            true,
            ProcessRefreshKind::nothing().with_cpu().with_memory(),
        );

        let process = system.process(pid)?;
        Some(ProcessMetrics {
            cpu_percent: process.cpu_usage(),
            memory_mb: process.memory() / 1024 / 1024,
        })
    }

    /// Total size of everything under `dir`, cached for [`DISK_TTL`].
    ///
    /// Returns the cached value immediately when it is fresh; otherwise the walk
    /// runs on the blocking pool, because a world with tens of thousands of
    /// region files is not something to traverse on the async runtime.
    pub async fn disk_usage(&self, dir: &Path) -> u64 {
        if let Ok(cache) = self.disk.lock() {
            if let Some((measured, bytes)) = cache.get(dir) {
                if measured.elapsed() < DISK_TTL {
                    return *bytes;
                }
            }
        }

        let target = dir.to_path_buf();
        let bytes = tokio::task::spawn_blocking(move || directory_size(&target))
            .await
            .unwrap_or(0);

        if let Ok(mut cache) = self.disk.lock() {
            cache.insert(dir.to_path_buf(), (Instant::now(), bytes));
        }

        bytes
    }
}

/// Sum every regular file under `root`.
fn directory_size(root: &Path) -> u64 {
    let mut total = 0;
    let mut stack = vec![root.to_path_buf()];

    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };

        for entry in entries.flatten() {
            // symlink_metadata, not metadata: following links would count a tree
            // reachable elsewhere twice, and a link back to an ancestor would
            // send this walk round forever.
            let Ok(meta) = std::fs::symlink_metadata(entry.path()) else { continue };

            if meta.is_symlink() {
                continue;
            }
            if meta.is_dir() {
                stack.push(entry.path());
            } else if meta.is_file() {
                total += meta.len();
            }
        }
    }

    total
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_directory_size_is_the_sum_of_its_files() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("a"), vec![0u8; 100]).unwrap();
        std::fs::create_dir(tmp.path().join("nested")).unwrap();
        std::fs::write(tmp.path().join("nested/b"), vec![0u8; 250]).unwrap();

        assert_eq!(directory_size(tmp.path()), 350);
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_cycle_does_not_loop_forever() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("a"), vec![0u8; 40]).unwrap();
        std::fs::create_dir(tmp.path().join("nested")).unwrap();

        // A link back to an ancestor. Following it would recurse without end.
        std::os::unix::fs::symlink(tmp.path(), tmp.path().join("nested/loop")).unwrap();

        assert_eq!(directory_size(tmp.path()), 40);
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_tree_is_not_counted_twice() {
        let tmp = tempfile::tempdir().unwrap();
        let real = tmp.path().join("real");
        std::fs::create_dir(&real).unwrap();
        std::fs::write(real.join("big"), vec![0u8; 500]).unwrap();

        std::os::unix::fs::symlink(&real, tmp.path().join("alias")).unwrap();

        assert_eq!(directory_size(tmp.path()), 500);
    }

    #[test]
    fn an_absent_directory_measures_zero_rather_than_failing() {
        assert_eq!(directory_size(Path::new("/nonexistent/for/sure")), 0);
    }

    #[tokio::test]
    async fn a_measurement_is_cached() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("a"), vec![0u8; 64]).unwrap();

        let metrics = Metrics::default();
        assert_eq!(metrics.disk_usage(tmp.path()).await, 64);

        // Written after the measurement, so a cached read must not see it.
        std::fs::write(tmp.path().join("b"), vec![0u8; 1000]).unwrap();
        assert_eq!(metrics.disk_usage(tmp.path()).await, 64);
    }
}
