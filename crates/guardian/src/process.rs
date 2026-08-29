//! The process supervisor.
//!
//! A [`Guardian`] owns one Java process: it provisions the environment, spawns
//! the JVM, pumps its stdio into a broadcast channel, drives a status machine,
//! stops it gracefully, and restarts it when it dies unexpectedly.
//!
//! Every method takes `&self`, so a `Arc<Guardian>` can be shared across HTTP
//! handlers, WebSocket sessions and the supervisor task without coordination.

use crate::config::{GuardianConfig, ServerConfig, MAX_ARGUMENT_BYTES};
use crate::environment::{prepare, Provision, ServerEnvironment};
use crate::error::{Error, Result};
use crate::events::{ConsoleLine, ServerEvent, ServerStatus, Stream};
use crate::fs::ScopedFs;
use crate::install::Installation;
use std::collections::VecDeque;
use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin};
use tokio::sync::{broadcast, watch, Mutex, RwLock};
use tokio::task::AbortHandle;

/// How often the supervisor checks whether the child has exited.
const REAP_INTERVAL: Duration = Duration::from_millis(200);
/// Maximum size of one line sent to the Minecraft console.
pub const MAX_COMMAND_BYTES: usize = 8 * 1024;

/// Vanilla and every fork log this once the world is loaded and the port is open.
fn line_means_online(line: &str) -> bool {
    line.contains("Done (") && line.contains("For help, type")
}

/// Whether `start` is permitted from `status`.
fn may_start(status: ServerStatus) -> bool {
    matches!(status, ServerStatus::Offline | ServerStatus::Crashed)
}

/// Whether `stop` and `kill` are permitted from `status`.
///
/// `Preparing` counts: a download the operator no longer wants must be
/// abandonable, or a slow provision leaves the server unusable until the panel
/// itself is restarted.
fn may_stop(status: ServerStatus) -> bool {
    status.is_running() || status == ServerStatus::Preparing
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
    /// Handle to the in-flight provisioning task, so it can be abandoned.
    preparing: Option<AbortHandle>,
    /// Cooperative cancellation for a start provision.  The task is not
    /// aborted after it might have spawned a child; `launch` must get a chance
    /// to inspect the state and kill a child that raced with cancellation.
    preparation_cancel: Option<watch::Sender<bool>>,
    /// Identifies the preparation task that owns `preparing` and
    /// `preparation_cancel`, so a cancelled task cannot clean up a later one.
    preparation_id: u64,
    /// Monotonic source for preparation ownership ids.
    next_preparation_id: u64,
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
    pub fn new(
        config: ServerConfig,
        policy: GuardianConfig,
        data_dir: impl Into<PathBuf>,
    ) -> Arc<Self> {
        let (events, _) = broadcast::channel(1024);
        Arc::new(Guardian {
            config: RwLock::new(config),
            policy: RwLock::new(policy),
            data_dir: data_dir.into(),
            state: Mutex::new(RunState {
                status: Some(ServerStatus::Offline),
                ..RunState::default()
            }),
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
    ///
    /// The resolved environment is only discarded when the change actually
    /// affects which artifact runs. Renaming a server, or giving it more RAM,
    /// must not cost a re-resolve.
    pub async fn set_config(&self, config: ServerConfig) {
        let artifact_changed = {
            let current = self.config.read().await;
            current.artifact_key() != config.artifact_key()
        };

        *self.config.write().await = config;

        if artifact_changed {
            *self.environment.write().await = None;
        }
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
        self.state
            .lock()
            .await
            .status
            .unwrap_or(ServerStatus::Offline)
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

    /// What is installed in this server's directory, if anything.
    pub async fn installation(&self) -> Option<Installation> {
        Installation::load(&self.config.read().await.directory).await
    }

    /// Re-resolve and download the server artifact, replacing what is installed.
    ///
    /// This is how an operator deliberately takes a newer build: an ordinary
    /// start never does it, so restarting cannot change what is running.
    pub async fn reinstall(self: &Arc<Self>) -> Result<ServerEnvironment> {
        let task = {
            let mut state = self.state.lock().await;
            let current = state.status.unwrap_or(ServerStatus::Offline);
            if current.is_running() || current == ServerStatus::Preparing {
                return Err(Error::InvalidTransition {
                    current: current.as_str(),
                    action: "reinstall",
                });
            }
            state.status = Some(ServerStatus::Preparing);
            state.intentional = false;
            state.next_preparation_id = state.next_preparation_id.wrapping_add(1);
            let preparation_id = state.next_preparation_id;
            state.preparation_id = preparation_id;
            *self.environment.write().await = None;
            let this = Arc::clone(self);
            let task = tokio::spawn(async move {
                let result = this.provision(Provision::Force).await;
                let changed = {
                    let mut state = this.state.lock().await;
                    if state.preparation_id == preparation_id {
                        state.preparing = None;
                        state.preparation_cancel = None;
                        if state.status == Some(ServerStatus::Preparing) {
                            state.status = Some(ServerStatus::Offline);
                            true
                        } else {
                            false
                        }
                    } else {
                        false
                    }
                };
                if changed {
                    this.emit(ServerEvent::Status {
                        status: ServerStatus::Offline,
                    });
                }
                result
            });
            state.preparing = Some(task.abort_handle());
            task
        };
        self.emit(ServerEvent::Status {
            status: ServerStatus::Preparing,
        });

        match task.await {
            Ok(result) => result,
            Err(error) => Err(Error::Task(error.to_string())),
        }
    }

    /// Provision the environment (Java, jar, directory) without starting anything.
    ///
    /// Reuses the recorded installation when it already satisfies the config,
    /// so this is a local, offline operation in the common case.
    pub async fn prepare(self: &Arc<Self>) -> Result<ServerEnvironment> {
        self.provision(Provision::IfNeeded).await
    }

    async fn provision(self: &Arc<Self>, mode: Provision) -> Result<ServerEnvironment> {
        let config = self.config().await;
        let this = Arc::downgrade(self);

        let progress = move |stage: String, fraction: Option<f32>| {
            let Some(guardian) = this.upgrade() else {
                return;
            };

            guardian.emit(ServerEvent::Progress {
                stage: stage.clone(),
                fraction,
            });

            // Also recorded to the console, so a client that connects part-way
            // through a long download still sees why the server is not up yet.
            let line = match fraction {
                Some(f) => format!("{stage} ({}%)", (f * 100.0).round() as u32),
                None => stage,
            };
            tokio::spawn(async move { guardian.say(line).await });
        };

        let env = prepare(&config, &self.data_dir, mode, progress).await?;
        *self.environment.write().await = Some(env.clone());
        Ok(env)
    }

    /// Begin starting the server, returning as soon as the work is under way.
    ///
    /// Provisioning can take minutes — downloading a JDK and a server jar — so
    /// it runs in a detached task rather than inside the caller's future. An
    /// HTTP handler that is dropped when the browser navigates away must not be
    /// able to strand the state machine in [`ServerStatus::Preparing`].
    ///
    /// Watch the event stream for [`ServerStatus::Starting`] and then
    /// [`ServerStatus::Online`]; a failure reports [`ServerStatus::Offline`]
    /// with the reason on the console.
    pub async fn start(self: &Arc<Self>) -> Result<()> {
        // The transition, the intent reset, and the task reservation all happen
        // while one lock is held.  In particular, there is no awaitable gap
        // between observing Offline and publishing Preparing, so two callers
        // cannot both reserve a launch.
        let task = {
            let mut state = self.state.lock().await;
            let current = state.status.unwrap_or(ServerStatus::Offline);
            if !may_start(current) {
                return Err(Error::InvalidTransition {
                    current: current.as_str(),
                    action: "start",
                });
            }
            state.intentional = false;
            state.status = Some(ServerStatus::Preparing);
            state.next_preparation_id = state.next_preparation_id.wrapping_add(1);
            let preparation_id = state.next_preparation_id;
            state.preparation_id = preparation_id;

            let (cancel, cancel_rx) = watch::channel(false);
            let this = Arc::clone(self);
            let task =
                tokio::spawn(
                    async move { this.provision_and_launch(cancel_rx, preparation_id).await },
                );
            state.preparing = Some(task.abort_handle());
            state.preparation_cancel = Some(cancel);
            task
        };

        self.emit(ServerEvent::Status {
            status: ServerStatus::Preparing,
        });
        // Dropping the join handle is intentional: the lifecycle task owns the
        // state transition and must outlive the HTTP request that started it.
        drop(task);
        Ok(())
    }

    /// Resolve the environment and spawn the JVM. Always leaves a terminal status.
    async fn provision_and_launch(
        self: Arc<Self>,
        mut cancel: watch::Receiver<bool>,
        preparation_id: u64,
    ) {
        let environment = match self.environment().await {
            Some(environment) => Ok(environment),
            None => {
                let limit = Duration::from_secs(self.policy().await.prepare_timeout_secs);
                let preparation = tokio::time::timeout(limit, self.prepare());
                tokio::pin!(preparation);
                tokio::select! {
                    result = &mut preparation => match result {
                        Ok(result) => result,
                        Err(_) => Err(Error::PrepareTimedOut(limit.as_secs())),
                    },
                    changed = cancel.changed() => {
                        if changed.is_ok() && *cancel.borrow() {
                            Err(Error::StartCancelled)
                        } else {
                            match preparation.await {
                                Ok(result) => result,
                                Err(_) => Err(Error::PrepareTimedOut(limit.as_secs())),
                            }
                        }
                    },
                }
            }
        };

        let outcome = match environment {
            Ok(environment) => self.launch(environment, preparation_id).await,
            Err(e) => Err(e),
        };

        if let Err(e) = outcome {
            let (owned, changed) = {
                let mut state = self.state.lock().await;
                if state.preparation_id != preparation_id {
                    (false, false)
                } else {
                    state.preparing = None;
                    state.preparation_cancel = None;
                    let changed = if state.status == Some(ServerStatus::Preparing) {
                        state.status = Some(ServerStatus::Offline);
                        true
                    } else {
                        false
                    };
                    (true, changed)
                }
            };
            if !owned {
                return;
            }
            tracing::error!(error = ?e, "server start failed");
            self.say(format!("could not start: {}", e.client_message()))
                .await;
            if changed {
                self.emit(ServerEvent::Status {
                    status: ServerStatus::Offline,
                });
            }
            return;
        }

        let mut state = self.state.lock().await;
        if state.preparation_id == preparation_id {
            state.preparing = None;
            state.preparation_cancel = None;
        }
    }

    /// Spawn the JVM for an already-resolved environment.
    async fn launch(self: &Arc<Self>, env: ServerEnvironment, preparation_id: u64) -> Result<()> {
        let config = self.config().await;
        if crate::environment::absolute(config.directory.clone())
            != crate::environment::absolute(env.directory.clone())
        {
            return Err(Error::InvalidConfiguration(
                "the launch directory does not match the configured server directory".into(),
            ));
        }
        let server_fs =
            ScopedFs::open(&env.directory).map_err(|error| Error::io(&env.directory, error))?;
        let jar_relative = env.jar.strip_prefix(&env.directory).map_err(|_| {
            Error::InvalidConfiguration("the server jar must be inside its server directory".into())
        })?;
        let jar_metadata = server_fs
            .metadata(jar_relative)
            .map_err(|error| Error::io(&env.jar, error))?;
        if !jar_metadata.is_file {
            return Err(Error::InvalidConfiguration(
                "the server jar must be a regular file".into(),
            ));
        }
        let mut launch_args: Vec<OsString> = config
            .memory
            .jvm_flags()
            .into_iter()
            .map(OsString::from)
            .collect();
        launch_args.extend(config.jvm_args.iter().cloned().map(OsString::from));
        launch_args.push(OsString::from("-jar"));
        launch_args.push(env.jar.clone().into_os_string());
        launch_args.extend(config.server_args.iter().cloned().map(OsString::from));

        let mut command =
            crate::sandbox::command(&env.java, &env.directory, &env.jar, &launch_args);
        command
            .env_clear()
            .current_dir(&env.directory)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            // The JVM outlives a panel restart on purpose: an operator updating
            // the panel should not disconnect everyone playing.
            .kill_on_drop(false);

        for (key, value) in sanitized_environment(std::env::vars_os()) {
            let name = key.to_string_lossy();
            if !matches!(
                name.as_ref(),
                "HOME" | "USERPROFILE" | "TEMP" | "TMP" | "TMPDIR"
            ) {
                command.env(key, value);
            }
        }
        // These variables are useful to Java, but the panel user's home and
        // global temporary directory are not part of a server's capability.
        // Point them at the server root even on platforms without a kernel
        // sandbox; the Linux/macOS wrappers additionally map them inside the
        // sandbox namespace.
        command
            .env("HOME", &env.directory)
            .env("USERPROFILE", &env.directory)
            .env("TEMP", &env.directory)
            .env("TMP", &env.directory)
            .env("TMPDIR", &env.directory);

        let mut child = command.spawn().map_err(|e| Error::io(&env.java, e))?;

        let pid = child.id().unwrap_or(0);
        let stdin = child.stdin.take();
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();

        let generation = {
            let mut state = self.state.lock().await;
            if state.preparation_id != preparation_id
                || state.status != Some(ServerStatus::Preparing)
                || state.intentional
            {
                let _ = child.start_kill();
                return Err(Error::StartCancelled);
            }
            state.generation += 1;
            state.pid = Some(pid);
            state.stdin = stdin;
            state.started_at = Some(Instant::now());
            state.intentional = false;
            state.child = Some(child);
            state.status = Some(ServerStatus::Starting);
            state.generation
        };

        self.emit(ServerEvent::Status {
            status: ServerStatus::Starting,
        });
        self.emit(ServerEvent::Started { pid });
        self.say(format!(
            "started {} {} (pid {pid})",
            config.core, config.version
        ))
        .await;

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
                let Some(guardian) = this.upgrade() else {
                    return;
                };

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

                let Some(guardian) = this.upgrade() else {
                    return;
                };

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

                let Some((code, intentional)) = outcome else {
                    continue;
                };

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
                guardian
                    .say(format!("server exited unexpectedly with code {code:?}"))
                    .await;
                guardian.set_status(ServerStatus::Crashed).await;
                guardian.emit(ServerEvent::Crashed { code, attempt });

                let policy = guardian.policy().await;
                if !policy.auto_restart {
                    return;
                }
                if attempt > policy.max_retries {
                    guardian
                        .say(format!(
                            "giving up after {} failed restarts",
                            policy.max_retries
                        ))
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

                let still_crashed =
                    guardian.state.lock().await.status == Some(ServerStatus::Crashed);
                if !still_crashed {
                    return;
                }

                if let Err(e) = guardian.start().await {
                    tracing::error!(error = ?e, "automatic restart failed");
                    guardian
                        .say(format!("restart failed: {}", e.client_message()))
                        .await;
                }
                return;
            }
        });
    }

    /// Send a raw console command to the server's stdin.
    pub async fn command(&self, command: &str) -> Result<()> {
        validate_command(command)?;
        let mut state = self.state.lock().await;
        let stdin = state.stdin.as_mut().ok_or(Error::ConsoleUnavailable)?;
        stdin.write_all(command.as_bytes()).await?;
        stdin.write_all(b"\n").await?;
        stdin.flush().await?;
        drop(state);
        self.push_line(Stream::System, format!("> {command}")).await;
        Ok(())
    }

    /// Abandon an in-flight provision, leaving the server offline.
    ///
    /// Safe to lose the race with a launch that has just spawned: if a child
    /// exists by the time the lock is taken, this becomes an ordinary kill
    /// rather than a status change that contradicts a running process.
    pub async fn cancel_preparation(&self) -> Result<()> {
        let (launched, changed) = {
            let mut state = self.state.lock().await;

            if let Some(cancel) = state.preparation_cancel.take() {
                let _ = cancel.send(true);
            } else if let Some(task) = state.preparing.take() {
                // Reinstall only provisions and can never have a child.  It
                // therefore remains safe to abort that task outright.
                task.abort();
            }

            let launched = state.child.is_some();
            if let Some(child) = state.child.as_mut() {
                let _ = child.start_kill();
            }
            if launched {
                state.intentional = true;
            }
            let changed = if !launched && state.status == Some(ServerStatus::Preparing) {
                state.status = Some(ServerStatus::Offline);
                true
            } else {
                false
            };
            (launched, changed)
        };

        if !launched {
            self.say("preparation cancelled").await;
            if changed {
                self.emit(ServerEvent::Status {
                    status: ServerStatus::Offline,
                });
            }
        }

        Ok(())
    }

    /// Ask the server to shut down, killing it if it does not comply in time.
    ///
    /// While [`ServerStatus::Preparing`] this abandons the provision instead.
    pub async fn stop(&self) -> Result<()> {
        let preparing = {
            let mut state = self.state.lock().await;
            let current = state.status.unwrap_or(ServerStatus::Offline);
            if !may_stop(current) {
                return Err(Error::InvalidTransition {
                    current: current.as_str(),
                    action: "stop",
                });
            }
            if current == ServerStatus::Preparing {
                true
            } else {
                state.intentional = true;
                state.status = Some(ServerStatus::Stopping);
                false
            }
        };

        if preparing {
            return self.cancel_preparation().await;
        }

        self.emit(ServerEvent::Status {
            status: ServerStatus::Stopping,
        });

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
        self.kill().await?;
        if !self.wait_for_exit(Duration::from_secs(10)).await {
            return Err(Error::Task(
                "the server process did not exit after being killed".into(),
            ));
        }
        Ok(())
    }

    /// Wait until the child handle has been reaped by the supervisor.
    ///
    /// This is used by callers that are about to drop the guardian. Since
    /// child handles deliberately use kill_on_drop(false), dropping one before
    /// the supervisor observes exit could otherwise leave a live JVM orphaned.
    pub async fn wait_for_exit(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            if self.state.lock().await.child.is_none() {
                return true;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return false;
            }
            tokio::time::sleep(REAP_INTERVAL.min(remaining)).await;
        }
    }

    /// Terminate the process immediately, without asking.
    ///
    /// While [`ServerStatus::Preparing`] this abandons the provision instead.
    pub async fn kill(&self) -> Result<()> {
        let preparing = {
            let mut state = self.state.lock().await;
            let current = state.status.unwrap_or(ServerStatus::Offline);
            if current == ServerStatus::Preparing {
                true
            } else {
                if !current.is_running() {
                    return Err(Error::InvalidTransition {
                        current: current.as_str(),
                        action: "kill",
                    });
                }
                state.intentional = true;
                let should_emit = current != ServerStatus::Stopping;
                state.status = Some(ServerStatus::Stopping);
                if let Some(child) = state.child.as_mut() {
                    let _ = child.start_kill();
                }
                drop(state);
                if should_emit {
                    self.emit(ServerEvent::Status {
                        status: ServerStatus::Stopping,
                    });
                }
                false
            }
        };
        if preparing {
            return self.cancel_preparation().await;
        }
        Ok(())
    }

    /// Stop and start again, tolerating an already-stopped server.
    pub async fn restart(self: &Arc<Self>) -> Result<()> {
        if may_stop(self.status().await) {
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

/// Environment variables deliberately passed to a Minecraft child.
///
/// Java plugins are arbitrary code, so inheriting the panel's complete process
/// environment would hand them every deployment credential exposed to the
/// service.  The allow-list contains only values needed for a normal Java
/// runtime and locale/temp handling.
pub(crate) fn sanitized_environment(
    variables: impl IntoIterator<Item = (std::ffi::OsString, std::ffi::OsString)>,
) -> Vec<(std::ffi::OsString, std::ffi::OsString)> {
    const ALLOWED: &[&str] = &[
        "PATH",
        "HOME",
        "USER",
        "LOGNAME",
        "USERPROFILE",
        "TEMP",
        "TMP",
        "TMPDIR",
        "LANG",
        "LC_ALL",
        "LC_CTYPE",
        "TZ",
        "SYSTEMROOT",
        "WINDIR",
    ];

    variables
        .into_iter()
        .filter(|(key, _)| {
            let key = key.to_string_lossy();
            ALLOWED.iter().any(|allowed| {
                if cfg!(windows) {
                    key.eq_ignore_ascii_case(allowed)
                } else {
                    key == *allowed
                }
            })
        })
        .collect()
}

/// Validate one command before it is written to the server's stdin.
pub fn validate_command(command: &str) -> Result<()> {
    if command.is_empty() {
        return Err(Error::InvalidCommand("command must not be empty"));
    }
    if command.len() > MAX_COMMAND_BYTES || command.len() > MAX_ARGUMENT_BYTES {
        return Err(Error::InvalidCommand("command is too long"));
    }
    if command.contains(['\0', '\r', '\n']) {
        return Err(Error::InvalidCommand(
            "command may not contain NUL, carriage-return, or newline characters",
        ));
    }
    Ok(())
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
    fn a_preparing_server_can_be_stopped_but_not_started_again() {
        // Refusing to stop while preparing is what left a cancelled download
        // stuck: nothing short of restarting the panel could clear it.
        assert!(may_stop(ServerStatus::Preparing));
        assert!(!may_start(ServerStatus::Preparing));
    }

    #[test]
    fn only_a_settled_server_can_be_started() {
        assert!(may_start(ServerStatus::Offline));
        assert!(may_start(ServerStatus::Crashed));

        assert!(!may_start(ServerStatus::Starting));
        assert!(!may_start(ServerStatus::Online));
        assert!(!may_start(ServerStatus::Stopping));
    }

    #[test]
    fn a_settled_server_cannot_be_stopped() {
        assert!(!may_stop(ServerStatus::Offline));
        assert!(!may_stop(ServerStatus::Crashed));

        assert!(may_stop(ServerStatus::Starting));
        assert!(may_stop(ServerStatus::Online));
    }

    #[test]
    fn similar_looking_lines_do_not_mark_the_server_online() {
        // A plugin printing its own "Done" must not flip the status early.
        assert!(!line_means_online("[INFO]: Done (0.1s)! Loading plugins"));
        assert!(!line_means_online("[INFO]: Preparing spawn area: 84%"));
        assert!(!line_means_online(""));
    }

    #[test]
    fn child_environment_excludes_credentials_and_runtime_injection_variables() {
        let environment = sanitized_environment([
            ("PATH".into(), "/bin".into()),
            ("HOME".into(), "/home/panel".into()),
            ("GH_TOKEN".into(), "secret".into()),
            ("AWS_SECRET_ACCESS_KEY".into(), "secret".into()),
            ("JAVA_TOOL_OPTIONS".into(), "-javaagent:evil".into()),
            ("MCPANEL_PLAYIT_SECRET".into(), "secret".into()),
        ]);

        assert!(environment.iter().any(|(key, _)| key == "PATH"));
        assert!(environment.iter().any(|(key, _)| key == "HOME"));
        assert!(!environment.iter().any(|(key, _)| key == "GH_TOKEN"));
        assert!(!environment
            .iter()
            .any(|(key, _)| key == "AWS_SECRET_ACCESS_KEY"));
        assert!(!environment
            .iter()
            .any(|(key, _)| key == "JAVA_TOOL_OPTIONS"));
        assert!(!environment
            .iter()
            .any(|(key, _)| key == "MCPANEL_PLAYIT_SECRET"));
    }

    #[test]
    fn console_commands_have_a_single_bounded_line() {
        assert!(validate_command("say hello").is_ok());
        assert!(validate_command("").is_err());
        assert!(validate_command("say\nstop").is_err());
        assert!(validate_command(&"x".repeat(MAX_COMMAND_BYTES + 1)).is_err());
    }
}
