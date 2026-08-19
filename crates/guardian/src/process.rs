//! The process supervisor.
//!
//! A [`Guardian`] owns one Java process: it provisions the environment, spawns
//! the JVM, pumps its stdio into a broadcast channel, drives a status machine,
//! stops it gracefully, and restarts it when it dies unexpectedly.
//!
//! Every method takes `&self`, so a `Arc<Guardian>` can be shared across HTTP
//! handlers, WebSocket sessions and the supervisor task without coordination.

use crate::config::{GuardianConfig, ServerConfig};
use crate::environment::{prepare, ServerEnvironment};
use crate::error::{Error, Result};
use crate::events::{ConsoleLine, ServerEvent, ServerStatus, Stream};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{broadcast, Mutex, RwLock};

/// How often the supervisor checks whether the child has exited.
const REAP_INTERVAL: Duration = Duration::from_millis(200);

/// Vanilla and every fork log this once the world is loaded and the port is open.
fn line_means_online(line: &str) -> bool {
    line.contains("Done (") && line.contains("For help, type")
}

/// Mutable runtime state, guarded as one unit so status and process cannot disagree.
#[derive(Default)]
struct RunState {
    status: Option<ServerStatus>,
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    pid: Option<u32>,
    started_at: Option<Instant>,
    /// Set by `stop`/`kill` so the supervisor knows the exit was requested.
    intentional: bool,
    /// Incremented on every spawn; a supervisor whose generation is stale exits quietly.
    generation: u64,
}

/// A point-in-time view of a server, cheap enough to serialise on every poll.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Snapshot {
    /// Current lifecycle state.
    pub status: ServerStatus,
    /// Process id, while one exists.
    pub pid: Option<u32>,
    /// Seconds since the current process was spawned.
    pub uptime_secs: Option<u64>,
    /// Consecutive crashes not yet cleared by a successful start.
    pub crashes: u32,
}

/// Supervises exactly one Minecraft server.
pub struct Guardian {
    config: RwLock<ServerConfig>,
    policy: RwLock<GuardianConfig>,
    data_dir: PathBuf,
    state: Mutex<RunState>,
    events: broadcast::Sender<ServerEvent>,
    console: Mutex<VecDeque<ConsoleLine>>,
    environment: RwLock<Option<ServerEnvironment>>,
    seq: AtomicU64,
    crashes: AtomicU32,
}

impl Guardian {
    /// Build a guardian for `config`. Nothing is provisioned or spawned yet.
    pub fn new(config: ServerConfig, policy: GuardianConfig, data_dir: impl Into<PathBuf>) -> Arc<Self> {
        let (events, _) = broadcast::channel(1024);
        Arc::new(Guardian {
            config: RwLock::new(config),
            policy: RwLock::new(policy),
            data_dir: data_dir.into(),
            state: Mutex::new(RunState { status: Some(ServerStatus::Offline), ..RunState::default() }),
            events,
            console: Mutex::new(VecDeque::new()),
            environment: RwLock::new(None),
            seq: AtomicU64::new(0),
            crashes: AtomicU32::new(0),
        })
    }

    /// Subscribe to the live event stream. Late subscribers miss earlier events;
    /// pair this with [`Guardian::console`] to backfill.
    pub fn subscribe(&self) -> broadcast::Receiver<ServerEvent> {
        self.events.subscribe()
    }

    /// The retained console lines, oldest first.
    pub async fn console(&self) -> Vec<ConsoleLine> {
        self.console.lock().await.iter().cloned().collect()
    }

    /// The current configuration.
    pub async fn config(&self) -> ServerConfig {
        self.config.read().await.clone()
    }

    /// Replace the configuration. Takes effect on the next start.
    pub async fn set_config(&self, config: ServerConfig) {
        *self.config.write().await = config;
        // The environment was derived from the old config, so it is now stale.
        *self.environment.write().await = None;
    }

    /// The current supervision policy.
    pub async fn policy(&self) -> GuardianConfig {
        self.policy.read().await.clone()
    }

    /// Replace the supervision policy. Applies to the next crash.
    pub async fn set_policy(&self, policy: GuardianConfig) {
        *self.policy.write().await = policy;
    }

    /// Current status.
    pub async fn status(&self) -> ServerStatus {
        self.state.lock().await.status.unwrap_or(ServerStatus::Offline)
    }

    /// A consistent view of status, pid and uptime.
    pub async fn snapshot(&self) -> Snapshot {
        let state = self.state.lock().await;
        Snapshot {
            status: state.status.unwrap_or(ServerStatus::Offline),
            pid: state.pid,
            uptime_secs: state.started_at.map(|t| t.elapsed().as_secs()),
            crashes: self.crashes.load(Ordering::Relaxed),
        }
    }

    // -- event plumbing ----------------------------------------------------

    fn emit(&self, event: ServerEvent) {
        // A send error only means nobody is listening, which is normal.
        let _ = self.events.send(event);
    }

    async fn set_status(&self, status: ServerStatus) {
        self.state.lock().await.status = Some(status);
        self.emit(ServerEvent::Status { status });
    }

    async fn push_line(&self, stream: Stream, line: String) {
        let seq = self.seq.fetch_add(1, Ordering::Relaxed);
        let entry = ConsoleLine { seq, stream, line };

        let cap = self.policy.read().await.console_buffer;
        {
            let mut buf = self.console.lock().await;
            buf.push_back(entry.clone());
            while buf.len() > cap {
                buf.pop_front();
            }
        }
        self.emit(ServerEvent::Console(entry));
    }

    /// Write a line to the console buffer as if the guardian had said it.
    pub async fn say(&self, message: impl Into<String>) {
        self.push_line(Stream::System, message.into()).await;
    }

    // -- lifecycle ---------------------------------------------------------

    /// The environment from the last successful [`Guardian::prepare`], if any.
    pub async fn environment(&self) -> Option<ServerEnvironment> {
        self.environment.read().await.clone()
    }

    /// Supply a pre-provisioned environment, skipping discovery and download.
    ///
    /// [`Guardian::set_config`] clears it again, because an environment derived
    /// from one config says nothing about another.
    pub async fn set_environment(&self, environment: ServerEnvironment) {
        // Absolutized here rather than trusted, because `start` spawns with the
        // working directory set to the server folder: a relative path would be
        // re-resolved against it and the launch would fail.
        *self.environment.write().await = Some(environment.absolutize());
    }

    /// Provision the environment (Java, jar, directory) without starting anything.
    ///
    /// This always re-resolves. [`Guardian::start`] reuses a cached environment
    /// instead, so a crash-restart loop does not re-run discovery and re-verify
    /// the jar over the network on every attempt.
    pub async fn prepare(self: &Arc<Self>) -> Result<ServerEnvironment> {
        let config = self.config().await;
        let this = Arc::downgrade(self);

        let progress = move |stage: String, fraction: Option<f32>| {
            if let Some(g) = this.upgrade() {
                g.emit(ServerEvent::Progress { stage, fraction });
            }
        };

        let env = prepare(&config, &self.data_dir, progress).await?;
        *self.environment.write().await = Some(env.clone());
        Ok(env)
    }

    /// Provision if needed, then spawn the JVM.
    ///
    /// Returns once the process exists; it will still be `Starting` at that
    /// point. Wait for [`ServerStatus::Online`] on the event stream to know the
    /// world has loaded.
    pub async fn start(self: &Arc<Self>) -> Result<()> {
        {
            let state = self.state.lock().await;
            let current = state.status.unwrap_or(ServerStatus::Offline);
            if current.is_running() || current == ServerStatus::Preparing {
                return Err(Error::InvalidTransition { current: current.as_str(), action: "start" });
            }
        }

        let env = match self.environment().await {
            Some(env) => env,
            None => {
                self.set_status(ServerStatus::Preparing).await;
                match self.prepare().await {
                    Ok(env) => env,
                    Err(e) => {
                        self.say(format!("preparation failed: {e}")).await;
                        self.set_status(ServerStatus::Offline).await;
                        return Err(e);
                    }
                }
            }
        };

        let config = self.config().await;
        let mut command = Command::new(&env.java);
        command
            .current_dir(&env.directory)
            .args(config.memory.jvm_flags())
            .args(&config.jvm_args)
            .arg("-jar")
            .arg(&env.jar)
            .args(&config.server_args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            // The JVM outlives a panel restart on purpose: an operator updating
            // the panel should not disconnect everyone playing.
            .kill_on_drop(false);

        let mut child = command.spawn().map_err(|e| Error::io(&env.java, e))?;

        let pid = child.id().unwrap_or(0);
        let stdin = child.stdin.take();
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();

        let generation = {
            let mut state = self.state.lock().await;
            state.generation += 1;
            state.pid = Some(pid);
            state.stdin = stdin;
            state.started_at = Some(Instant::now());
            state.intentional = false;
            state.child = Some(child);
            state.generation
        };

        self.set_status(ServerStatus::Starting).await;
        self.emit(ServerEvent::Started { pid });
        self.say(format!("started {} {} (pid {pid})", config.core, config.version)).await;

        if let Some(out) = stdout {
            self.spawn_reader(out, Stream::Stdout);
        }
        if let Some(err) = stderr {
            self.spawn_reader(err, Stream::Stderr);
        }

        self.spawn_supervisor(generation);
        Ok(())
    }

    fn spawn_reader<R>(self: &Arc<Self>, reader: R, stream: Stream)
    where
        R: tokio::io::AsyncRead + Unpin + Send + 'static,
    {
        let this = Arc::downgrade(self);
        tokio::spawn(async move {
            let mut lines = BufReader::new(reader).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let Some(guardian) = this.upgrade() else { return };

                if stream == Stream::Stdout
                    && line_means_online(&line)
                    && guardian.status().await == ServerStatus::Starting
                {
                    guardian.set_status(ServerStatus::Online).await;
                    // A clean start invalidates the crash streak.
                    guardian.crashes.store(0, Ordering::Relaxed);
                }

                guardian.push_line(stream, line).await;
            }
        });
    }

    /// Poll for the child's exit and decide what it meant.
    fn spawn_supervisor(self: &Arc<Self>, generation: u64) {
        let this = Arc::downgrade(self);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(REAP_INTERVAL).await;

                let Some(guardian) = this.upgrade() else { return };

                let outcome = {
                    let mut state = guardian.state.lock().await;
                    if state.generation != generation {
                        return; // superseded by a newer process
                    }
                    match state.child.as_mut() {
                        None => return,
                        Some(child) => match child.try_wait() {
                            Ok(None) => None,
                            Ok(Some(status)) => Some((status.code(), state.intentional)),
                            // The child is unreachable; treat it as gone rather than spinning.
                            Err(_) => Some((None, state.intentional)),
                        },
                    }
                };

                let Some((code, intentional)) = outcome else { continue };

                {
                    let mut state = guardian.state.lock().await;
                    state.child = None;
                    state.stdin = None;
                    state.pid = None;
                    state.started_at = None;
                }

                if intentional {
                    guardian.say("server stopped").await;
                    guardian.emit(ServerEvent::Stopped { code });
                    guardian.set_status(ServerStatus::Offline).await;
                    return;
                }

                let attempt = guardian.crashes.fetch_add(1, Ordering::Relaxed) + 1;
                guardian.say(format!("server exited unexpectedly with code {code:?}")).await;
                guardian.emit(ServerEvent::Crashed { code, attempt });
                guardian.set_status(ServerStatus::Crashed).await;

                let policy = guardian.policy().await;
                if !policy.auto_restart {
                    return;
                }
                if attempt > policy.max_retries {
                    guardian
                        .say(format!("giving up after {} failed restarts", policy.max_retries))
                        .await;
                    return;
                }

                guardian
                    .say(format!(
                        "restarting in {}s ({attempt}/{})",
                        policy.retry_delay_secs, policy.max_retries
                    ))
                    .await;
                tokio::time::sleep(Duration::from_secs(policy.retry_delay_secs)).await;

                if let Err(e) = guardian.start().await {
                    guardian.say(format!("restart failed: {e}")).await;
                }
                return;
            }
        });
    }

    /// Send a raw console command to the server's stdin.
    pub async fn command(&self, command: &str) -> Result<()> {
        let mut state = self.state.lock().await;
        let stdin = state.stdin.as_mut().ok_or(Error::ConsoleUnavailable)?;
        stdin.write_all(command.as_bytes()).await?;
        stdin.write_all(b"\n").await?;
        stdin.flush().await?;
        drop(state);
        self.push_line(Stream::System, format!("> {command}")).await;
        Ok(())
    }

    /// Ask the server to shut down, killing it if it does not comply in time.
    pub async fn stop(&self) -> Result<()> {
        {
            let mut state = self.state.lock().await;
            let current = state.status.unwrap_or(ServerStatus::Offline);
            if !current.is_running() {
                return Err(Error::InvalidTransition { current: current.as_str(), action: "stop" });
            }
            state.intentional = true;
        }

        self.set_status(ServerStatus::Stopping).await;

        // Best effort: if stdin is already gone the process is on its way out
        // anyway, and the timeout below still covers us.
        let _ = self.command("stop").await;

        let deadline = Duration::from_secs(self.policy().await.stop_timeout_secs);
        let waited = Instant::now();
        while waited.elapsed() < deadline {
            if self.state.lock().await.child.is_none() {
                return Ok(());
            }
            tokio::time::sleep(REAP_INTERVAL).await;
        }

        self.say("graceful stop timed out, killing process").await;
        self.kill().await
    }

    /// Terminate the process immediately, without asking.
    pub async fn kill(&self) -> Result<()> {
        let mut state = self.state.lock().await;
        state.intentional = true;
        if let Some(child) = state.child.as_mut() {
            let _ = child.start_kill();
        }
        Ok(())
    }

    /// Stop and start again, tolerating an already-stopped server.
    pub async fn restart(self: &Arc<Self>) -> Result<()> {
        if self.status().await.is_running() {
            self.stop().await?;
        }
        // The supervisor clears the child asynchronously; wait for it so the
        // start below is not rejected as an invalid transition.
        for _ in 0..50 {
            if !self.status().await.is_running() {
                break;
            }
            tokio::time::sleep(REAP_INTERVAL).await;
        }
        self.start().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_done_line_marks_the_server_as_online() {
        assert!(line_means_online(
            r#"[11:05:24 INFO]: Done (12.612s)! For help, type "help""#
        ));
    }

    #[test]
    fn similar_looking_lines_do_not_mark_the_server_online() {
        // A plugin printing its own "Done" must not flip the status early.
        assert!(!line_means_online("[INFO]: Done (0.1s)! Loading plugins"));
        assert!(!line_means_online("[INFO]: Preparing spawn area: 84%"));
        assert!(!line_means_online(""));
    }
}
