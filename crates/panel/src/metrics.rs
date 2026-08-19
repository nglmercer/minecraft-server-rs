//! Per-process resource usage.
//!
//! Host-level numbers say nothing useful when several servers share a box, so
//! the panel reports each JVM's own CPU and memory.

use serde::Serialize;
use std::sync::Mutex;
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

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
}

impl Default for Metrics {
    fn default() -> Self {
        Metrics { system: Mutex::new(System::new()) }
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
}
