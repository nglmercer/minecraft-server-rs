//! Android CPU sampling when SELinux blocks /proc/stat.
//!
//! Kernel cpuidle counters measure microseconds spent in each idle state.
//! Their deltas provide an estimated host utilization. Per-process utime and
//! stime are sampled against monotonic wall time, without a host CPU baseline.
//! https://www.kernel.org/doc/html/latest/admin-guide/pm/cpuidle.html

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use sysinfo::MINIMUM_CPU_UPDATE_INTERVAL;

#[derive(Debug)]
struct HostSample {
    at: Instant,
    idle: BTreeMap<(u32, String), u64>,
}

#[derive(Debug, Clone, Copy)]
struct ProcessSample {
    at: Instant,
    ticks: u64,
    start: u64,
    percent: Option<f32>,
}

pub(super) struct AndroidCpu {
    sys_cpu: PathBuf,
    proc_root: PathBuf,
    ticks_per_second: u64,
    host: Option<HostSample>,
    host_percent: Option<f32>,
    processes: HashMap<u32, ProcessSample>,
}

impl AndroidCpu {
    #[cfg(target_os = "android")]
    pub(super) fn new() -> Self {
        Self::with_roots(
            PathBuf::from("/sys/devices/system/cpu"),
            PathBuf::from("/proc"),
            rustix::param::clock_ticks_per_second(),
        )
    }

    fn with_roots(sys_cpu: PathBuf, proc_root: PathBuf, ticks_per_second: u64) -> Self {
        Self {
            sys_cpu,
            proc_root,
            ticks_per_second,
            host: None,
            host_percent: None,
            processes: HashMap::new(),
        }
    }

    pub(super) fn host(&mut self, now: Instant) -> Option<f32> {
        if self.host.as_ref().is_some_and(|last| {
            now.saturating_duration_since(last.at) < MINIMUM_CPU_UPDATE_INTERVAL
        }) {
            return self.host_percent;
        }
        let Some(idle) = idle_counters(&self.sys_cpu) else {
            self.host = None;
            self.host_percent = None;
            return None;
        };
        let next = HostSample { at: now, idle };
        self.host_percent = self.host.as_ref().and_then(|previous| {
            // A changed CPU/state set, counter reset, or delayed idle-accounting
            // jump invalidates this interval. Never turn partial data into load.
            if !previous.idle.keys().eq(next.idle.keys()) {
                return None;
            }
            let idle_delta = next.idle.iter().try_fold(0u64, |sum, (key, value)| {
                sum.checked_add(value.checked_sub(*previous.idle.get(key)?)?)
            })?;
            let cores = next
                .idle
                .keys()
                .map(|(cpu, _)| cpu)
                .collect::<std::collections::BTreeSet<_>>()
                .len();
            let capacity =
                now.checked_duration_since(previous.at)?.as_secs_f64() * 1_000_000.0 * cores as f64;
            if capacity <= 0.0 || idle_delta as f64 > capacity * 1.05 {
                return None;
            }
            Some((100.0 * (1.0 - idle_delta as f64 / capacity)).clamp(0.0, 100.0) as f32)
        });
        self.host = Some(next);
        self.host_percent
    }

    pub(super) fn process(&mut self, pid: u32, now: Instant) -> Option<f32> {
        let counters = std::fs::read_to_string(self.proc_root.join(pid.to_string()).join("stat"))
            .ok()
            .and_then(|text| process_counters(&text));
        let Some((ticks, start)) = counters else {
            self.processes.remove(&pid);
            return None;
        };
        self.processes.retain(|_, sample| {
            now.saturating_duration_since(sample.at) < Duration::from_secs(300)
        });
        if let Some(previous) = self.processes.get(&pid) {
            if previous.start == start
                && ticks >= previous.ticks
                && now.saturating_duration_since(previous.at) < MINIMUM_CPU_UPDATE_INTERVAL
            {
                return previous.percent;
            }
        }
        let percent = self.processes.get(&pid).and_then(|previous| {
            if previous.start != start || self.ticks_per_second == 0 {
                return None;
            }
            let elapsed = now.checked_duration_since(previous.at)?.as_secs_f64();
            let delta = ticks.checked_sub(previous.ticks)?;
            (elapsed > 0.0)
                .then(|| (100.0 * delta as f64 / self.ticks_per_second as f64 / elapsed) as f32)
        });
        // Keep the cache bounded even under continuous process churn.
        if self.processes.len() >= 512 && !self.processes.contains_key(&pid) {
            if let Some(oldest) = self
                .processes
                .iter()
                .min_by_key(|(_, sample)| sample.at)
                .map(|(pid, _)| *pid)
            {
                self.processes.remove(&oldest);
            }
        }
        self.processes.insert(
            pid,
            ProcessSample {
                at: now,
                ticks,
                start,
                percent,
            },
        );
        percent
    }
}

fn online_cpus(text: &str) -> Option<Vec<u32>> {
    let mut cpus = std::collections::BTreeSet::new();
    for item in text.trim().split(',') {
        let (first, last) = match item.split_once('-') {
            Some((first, last)) => (first.parse::<u32>().ok()?, last.parse::<u32>().ok()?),
            None => {
                let cpu = item.parse::<u32>().ok()?;
                (cpu, cpu)
            }
        };
        if first > last || last >= 4096 {
            return None;
        }
        cpus.extend(first..=last);
    }
    (!cpus.is_empty()).then(|| cpus.into_iter().collect())
}

fn idle_counters(root: &Path) -> Option<BTreeMap<(u32, String), u64>> {
    let online = std::fs::read_to_string(root.join("online")).ok()?;
    let cpus = online_cpus(&online)?;
    let mut counters = BTreeMap::new();
    for cpu in cpus {
        let mut states = 0;
        for entry in std::fs::read_dir(root.join(format!("cpu{cpu}/cpuidle"))).ok()? {
            let entry = entry.ok()?;
            let name = entry.file_name().into_string().ok()?;
            if !name.strip_prefix("state").is_some_and(|suffix| {
                !suffix.is_empty() && suffix.bytes().all(|b| b.is_ascii_digit())
            }) {
                continue;
            }
            let value = std::fs::read_to_string(entry.path().join("time"))
                .ok()?
                .trim()
                .parse()
                .ok()?;
            counters.insert((cpu, name), value);
            states += 1;
        }
        if states == 0 {
            return None;
        }
    }
    // Hotplug during the read invalidates the whole snapshot.
    (std::fs::read_to_string(root.join("online")).ok()? == online).then_some(counters)
}

fn process_counters(text: &str) -> Option<(u64, u64)> {
    // comm may contain spaces and closing parentheses. The final ')' closes it.
    let (_, fields) = text.rsplit_once(')')?;
    let fields: Vec<_> = fields.split_whitespace().collect();
    let user: u64 = fields.get(11)?.parse().ok()?;
    let system: u64 = fields.get(12)?.parse().ok()?;
    let start = fields.get(19)?.parse().ok()?;
    Some((user.checked_add(system)?, start))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (tempfile::TempDir, AndroidCpu) {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("online"), "0-1\n").unwrap();
        for cpu in 0..2 {
            for state in 0..2 {
                let path = root.path().join(format!("cpu{cpu}/cpuidle/state{state}"));
                std::fs::create_dir_all(&path).unwrap();
                std::fs::write(path.join("time"), "1000000\n").unwrap();
            }
        }
        let sampler = AndroidCpu::with_roots(root.path().into(), root.path().into(), 100);
        (root, sampler)
    }

    fn set_idle(root: &Path, cpu: u32, state: u32, value: u64) {
        std::fs::write(
            root.join(format!("cpu{cpu}/cpuidle/state{state}/time")),
            value.to_string(),
        )
        .unwrap();
    }

    fn set_process(root: &Path, user: u64, system: u64, start: u64) {
        let mut fields = vec!["0".to_owned(); 20];
        fields[0] = "R".into();
        fields[11] = user.to_string();
        fields[12] = system.to_string();
        fields[19] = start.to_string();
        std::fs::create_dir_all(root.join("123")).unwrap();
        std::fs::write(
            root.join("123/stat"),
            format!("123 (worker ) with spaces) {}", fields.join(" ")),
        )
        .unwrap();
    }

    #[test]
    fn host_uses_all_online_cores_and_idle_states() {
        let (root, mut sampler) = fixture();
        let now = Instant::now();
        assert_eq!(sampler.host(now), None);
        set_idle(root.path(), 0, 0, 2_000_000);
        set_idle(root.path(), 0, 1, 1_500_000);
        set_idle(root.path(), 1, 0, 2_000_000);
        set_idle(root.path(), 1, 1, 1_500_000);
        assert_eq!(sampler.host(now + Duration::from_secs(2)), Some(25.0));
        assert_eq!(sampler.host(now + Duration::from_millis(2100)), Some(25.0));
    }

    #[test]
    fn missing_counter_never_reports_partial_host_usage() {
        let (root, mut sampler) = fixture();
        let now = Instant::now();
        sampler.host(now);
        std::fs::remove_file(root.path().join("cpu1/cpuidle/state1/time")).unwrap();
        assert_eq!(sampler.host(now + Duration::from_secs(2)), None);
        assert!(sampler.host.is_none());
    }

    #[test]
    fn hotplug_and_counter_resets_require_a_new_baseline() {
        let (root, mut sampler) = fixture();
        let now = Instant::now();
        sampler.host(now);
        std::fs::write(root.path().join("online"), "0\n").unwrap();
        assert_eq!(sampler.host(now + Duration::from_secs(2)), None);
        set_idle(root.path(), 0, 0, 0);
        assert_eq!(sampler.host(now + Duration::from_secs(4)), None);
        set_idle(root.path(), 0, 0, 1_000_000);
        assert_eq!(sampler.host(now + Duration::from_secs(6)), Some(50.0));
    }

    #[test]
    fn delayed_idle_accounting_cannot_produce_negative_utilization() {
        let (root, mut sampler) = fixture();
        let now = Instant::now();
        sampler.host(now);
        set_idle(root.path(), 0, 0, 100_000_000);
        assert_eq!(sampler.host(now + Duration::from_secs(2)), None);
    }

    #[test]
    fn process_cpu_uses_tick_deltas_not_host_cpu_or_lifetime_average() {
        let (root, mut sampler) = fixture();
        let now = Instant::now();
        set_process(root.path(), 100, 50, 500);
        assert_eq!(sampler.process(123, now), None);
        set_process(root.path(), 150, 100, 500);
        assert_eq!(
            sampler.process(123, now + Duration::from_secs(2)),
            Some(50.0)
        );
        assert_eq!(
            sampler.process(123, now + Duration::from_millis(2100)),
            Some(50.0)
        );
        set_process(root.path(), 550, 100, 500);
        assert_eq!(
            sampler.process(123, now + Duration::from_secs(4)),
            Some(200.0)
        );
    }

    #[test]
    fn reused_pids_resets_and_dead_processes_do_not_reuse_old_cpu() {
        let (root, mut sampler) = fixture();
        let now = Instant::now();
        set_process(root.path(), 100, 50, 500);
        sampler.process(123, now);
        set_process(root.path(), 1, 0, 600);
        assert_eq!(sampler.process(123, now + Duration::from_secs(2)), None);
        set_process(root.path(), 0, 0, 600);
        assert_eq!(sampler.process(123, now + Duration::from_secs(4)), None);
        std::fs::remove_file(root.path().join("123/stat")).unwrap();
        assert_eq!(sampler.process(123, now + Duration::from_secs(6)), None);
        assert!(sampler.processes.is_empty());
    }

    #[cfg(target_os = "android")]
    #[test]
    fn real_android_host_sampling_uses_readable_kernel_counters() {
        let mut sampler = AndroidCpu::new();
        let available = idle_counters(&sampler.sys_cpu).is_some();
        assert_eq!(sampler.host(Instant::now()), None);
        std::thread::sleep(Duration::from_millis(400));
        let sample = sampler.host(Instant::now());
        if let Some(value) = sample {
            assert!(available);
            assert!(value.is_finite() && (0.0..=100.0).contains(&value));
        } else {
            // Unreadable counters and deferred accounting jumps remain unknown.
            assert_eq!(sampler.host_percent, None);
        }
    }

    #[cfg(target_os = "android")]
    #[test]
    fn real_android_process_cpu_responds_to_work_without_proc_stat() {
        let mut sampler = AndroidCpu::new();
        let pid = std::process::id();
        assert_eq!(sampler.process(pid, Instant::now()), None);
        let start = Instant::now();
        let mut work = 0u64;
        while start.elapsed() < Duration::from_millis(400) {
            work = std::hint::black_box(work.wrapping_add(1));
        }
        let cpu = sampler
            .process(pid, Instant::now())
            .expect("own CPU ticks are readable");
        assert!(cpu > 5.0, "busy process reported {cpu}% CPU");
    }

    #[test]
    fn malformed_cpu_lists_and_stat_files_are_rejected() {
        assert_eq!(online_cpus("0-2,4\n"), Some(vec![0, 1, 2, 4]));
        for text in ["", "2-1", "0-99999999", "x", "-1"] {
            assert_eq!(online_cpus(text), None);
        }
        assert_eq!(process_counters("123 (broken)"), None);
    }
}
