//! Per-process resource usage.
//!
//! Host-level numbers say nothing useful when several servers share a box, so
//! the panel reports each JVM's own CPU and memory.

use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, MINIMUM_CPU_UPDATE_INTERVAL};

/// How long a directory size is trusted before it is measured again.
///
/// Walking a large world costs real I/O, and the dashboard polls every few
/// seconds; without this the panel would spend more effort measuring the server
/// than running it.
const DISK_TTL: Duration = Duration::from_secs(60);

/// What the machine as a whole is doing.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct HostMetrics {
    /// Busy percentage across all cores, 0..=100.
    pub cpu_percent: f32,
    /// Physical memory in use, in mebibytes.
    pub memory_used_mb: u64,
    /// Physical memory installed, in mebibytes.
    pub memory_total_mb: u64,
}

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
    // the same System instance has to survive between calls. Building a fresh
    // one per request is the classic way to get a number that never moves.
    system: Mutex<System>,
    disk: Mutex<HashMap<PathBuf, (Instant, u64)>>,
    /// Last host CPU reading, and when it was taken.
    host_cpu: Mutex<(Instant, f32)>,
}

impl Default for Metrics {
    fn default() -> Self {
        let mut system = System::new();

        // Establish the baseline the first real reading is measured against.
        system.refresh_cpu_usage();

        Metrics {
            system: Mutex::new(system),
            disk: Mutex::new(HashMap::new()),
            host_cpu: Mutex::new((Instant::now(), 0.0)),
        }
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

    /// Whole-machine CPU and memory.
    ///
    /// CPU is a delta against the previous sample, so two refreshes closer
    /// together than sysinfo's minimum interval would report nonsense. Inside
    /// that window the last real reading is repeated rather than recomputed.
    pub fn host(&self) -> HostMetrics {
        let mut system = match self.system.lock() {
            Ok(system) => system,
            Err(poisoned) => poisoned.into_inner(),
        };

        system.refresh_memory();

        let cpu_percent = match self.host_cpu.lock() {
            Ok(mut last) => {
                if last.0.elapsed() >= MINIMUM_CPU_UPDATE_INTERVAL {
                    system.refresh_cpu_usage();
                    *last = (Instant::now(), system.global_cpu_usage());
                }
                last.1
            }
            Err(_) => system.global_cpu_usage(),
        };

        HostMetrics {
            cpu_percent,
            memory_used_mb: system.used_memory() / 1024 / 1024,
            memory_total_mb: system.total_memory() / 1024 / 1024,
        }
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
    guardian::ScopedFs::open(root)
        .and_then(|fs| fs.directory_size("."))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Occupy every core for `duration`.
    fn saturate(duration: Duration) {
        let threads: Vec<_> = (0..std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(2))
            .map(|_| {
                std::thread::spawn(move || {
                    let deadline = Instant::now() + duration;
                    let mut spin: u64 = 0;
                    while Instant::now() < deadline {
                        spin = spin.wrapping_add(1);
                    }
                    std::hint::black_box(spin);
                })
            })
            .collect();

        for thread in threads {
            let _ = thread.join();
        }
    }

    #[test]
    fn host_cpu_responds_to_load() {
        let metrics = Metrics::default();

        // Establish a baseline, then read again after an idle gap.
        metrics.host();
        std::thread::sleep(MINIMUM_CPU_UPDATE_INTERVAL * 2);
        let idle = metrics.host().cpu_percent;

        saturate(MINIMUM_CPU_UPDATE_INTERVAL * 3);
        let busy = metrics.host().cpu_percent;

        // Asserting `busy > 0` would not catch anything: a freshly built System
        // reports an average since boot, which is non-zero and never moves. What
        // was broken is that the number did not *respond*, so that is the
        // assertion — saturating every core has to show up.
        assert!(
            busy > idle + 10.0,
            "idle {idle}% then saturated {busy}%: the reading is not tracking load"
        );
    }

    #[test]
    fn host_memory_is_reported_and_sane() {
        let host = Metrics::default().host();

        assert!(host.memory_total_mb > 0);
        assert!(host.memory_used_mb > 0);
        assert!(host.memory_used_mb <= host.memory_total_mb);
    }

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
