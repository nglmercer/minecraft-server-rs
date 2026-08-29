//! End-to-end supervisor behaviour, driven with a fake JVM.
//!
//! These tests spawn a real child process and exercise the real status machine,
//! stdio pumps and restart policy — only the Java/jar provisioning is stubbed,
//! because downloading a JDK is not something a unit test should do.
//!
//! The stand-in launcher is `tests/support/fake_java.rs`, a real binary, so
//! these run on Windows as well as Unix. Its behaviour is driven through
//! `server_args`, which is the same path a real server's arguments take.

use guardian::events::Stream;
use guardian::{
    Guardian, GuardianConfig, ServerConfig, ServerEnvironment, ServerEvent, ServerStatus,
};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::sync::broadcast::Receiver;

/// The stand-in launcher, built by cargo alongside these tests.
fn fake_java() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_guardian-fake-java"))
}

/// A guardian wired to the fake launcher, told to behave as `instructions` say.
fn guardian_with(
    dir: &Path,
    policy: GuardianConfig,
    instructions: &[&str],
) -> std::sync::Arc<Guardian> {
    let mut config = ServerConfig::paper(dir, "1.21.8");
    config.eula_accepted = true;
    // Passed after `-jar`, exactly where a real server's arguments go.
    config.server_args = instructions.iter().map(|s| s.to_string()).collect();

    // The fake launcher insists on a real jar, just as the real one does.
    std::fs::write(dir.join("server.jar"), b"not really a jar").unwrap();

    let guardian = Guardian::new(config, policy, dir);
    let environment = ServerEnvironment {
        java: fake_java(),
        java_major: 21,
        jar: dir.join("server.jar"),
        directory: dir.to_path_buf(),
    };
    futures_block_on(guardian.set_environment(environment));
    guardian
}

/// Tiny helper so the setup above can stay synchronous.
fn futures_block_on<F: std::future::Future>(future: F) -> F::Output {
    tokio::task::block_in_place(|| tokio::runtime::Handle::current().block_on(future))
}

/// Wait for a status, failing the test rather than hanging forever.
async fn await_status(events: &mut Receiver<ServerEvent>, want: ServerStatus) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        let remaining = deadline - tokio::time::Instant::now();
        let event = tokio::time::timeout(remaining, events.recv())
            .await
            .unwrap_or_else(|_| panic!("timed out waiting for {want:?}"))
            .expect("event stream closed");

        if let ServerEvent::Status { status } = event {
            if status == want {
                return;
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_server_that_logs_done_becomes_online_and_stops_gracefully() {
    let tmp = tempfile::tempdir().unwrap();
    let guardian = guardian_with(tmp.path(), GuardianConfig::default(), &["done", "serve"]);
    let mut events = guardian.subscribe();

    guardian.start().await.unwrap();
    await_status(&mut events, ServerStatus::Online).await;

    let snapshot = guardian.snapshot().await;
    assert!(snapshot.pid.is_some(), "an online server must report a pid");
    assert_eq!(snapshot.crashes, 0);

    guardian.stop().await.unwrap();
    assert_eq!(guardian.status().await, ServerStatus::Offline);
    assert!(guardian.snapshot().await.pid.is_none());

    // The graceful path must not be mistaken for a crash.
    let console = guardian.console().await;
    assert!(
        console.iter().any(|l| l.line.contains("server stopped")),
        "expected a clean stop notice, got: {console:?}"
    );
    assert!(!console.iter().any(|l| l.line.contains("unexpectedly")));
}

#[tokio::test(flavor = "multi_thread")]
async fn console_commands_reach_the_process_stdin() {
    let tmp = tempfile::tempdir().unwrap();
    let guardian = guardian_with(tmp.path(), GuardianConfig::default(), &["done", "serve"]);
    let mut events = guardian.subscribe();

    guardian.start().await.unwrap();
    await_status(&mut events, ServerStatus::Online).await;

    guardian.command("say hello").await.unwrap();

    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    let mut saw_echo = false;
    while tokio::time::Instant::now() < deadline && !saw_echo {
        saw_echo = guardian
            .console()
            .await
            .iter()
            .any(|l| l.stream == Stream::Stdout && l.line == "echoed: say hello");
        if !saw_echo {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
    assert!(saw_echo, "the command never reached the process");

    // The command is also echoed locally so the operator sees what they typed.
    assert!(guardian
        .console()
        .await
        .iter()
        .any(|l| l.stream == Stream::System && l.line == "> say hello"));

    guardian.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unexpected_exit_crashes_and_restarts_up_to_the_retry_limit() {
    let tmp = tempfile::tempdir().unwrap();
    let policy = GuardianConfig {
        auto_restart: true,
        max_retries: 2,
        retry_delay_secs: 0,
        ..GuardianConfig::default()
    };

    let guardian = guardian_with(tmp.path(), policy, &["err:boom", "exit:1"]);
    let mut events = guardian.subscribe();

    guardian.start().await.unwrap();

    // Three exits in total: the original plus two restarts.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    let mut attempts = Vec::new();
    while attempts.len() < 3 && tokio::time::Instant::now() < deadline {
        let remaining = deadline - tokio::time::Instant::now();
        let Ok(Ok(event)) = tokio::time::timeout(remaining, events.recv()).await else {
            break;
        };
        if let ServerEvent::Crashed { attempt, code } = event {
            assert_eq!(code, Some(1));
            attempts.push(attempt);
        }
    }

    assert_eq!(
        attempts,
        vec![1, 2, 3],
        "expected the original plus two restarts"
    );
    assert_eq!(guardian.status().await, ServerStatus::Crashed);

    // Having exhausted its retries, it must stay down rather than loop forever.
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(guardian.status().await, ServerStatus::Crashed);
    assert!(guardian
        .console()
        .await
        .iter()
        .any(|l| l.line.contains("giving up")));
}

#[tokio::test(flavor = "multi_thread")]
async fn auto_restart_can_be_switched_off() {
    let tmp = tempfile::tempdir().unwrap();
    let policy = GuardianConfig {
        auto_restart: false,
        ..GuardianConfig::default()
    };
    let guardian = guardian_with(tmp.path(), policy, &["exit:3"]);
    let mut events = guardian.subscribe();

    guardian.start().await.unwrap();
    await_status(&mut events, ServerStatus::Crashed).await;

    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(
        guardian.snapshot().await.crashes,
        1,
        "it must not have retried"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn starting_twice_is_rejected_rather_than_spawning_two_jvms() {
    let tmp = tempfile::tempdir().unwrap();
    let guardian = guardian_with(tmp.path(), GuardianConfig::default(), &["done", "serve"]);
    let mut events = guardian.subscribe();

    guardian.start().await.unwrap();
    await_status(&mut events, ServerStatus::Online).await;

    let pid = guardian.snapshot().await.pid;
    let error = guardian.start().await.unwrap_err();
    assert!(matches!(error, guardian::Error::InvalidTransition { .. }));
    assert_eq!(
        guardian.snapshot().await.pid,
        pid,
        "the original process must survive"
    );

    guardian.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn genuinely_concurrent_starts_reserve_exactly_one_process() {
    let tmp = tempfile::tempdir().unwrap();
    let guardian = guardian_with(tmp.path(), GuardianConfig::default(), &["done", "serve"]);
    let mut events = guardian.subscribe();
    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(16));

    let callers = (0..16)
        .map(|_| {
            let guardian = guardian.clone();
            let barrier = barrier.clone();
            tokio::spawn(async move {
                barrier.wait().await;
                guardian.start().await
            })
        })
        .collect::<Vec<_>>();

    let mut successes = 0;
    let mut conflicts = 0;
    for caller in callers {
        match caller.await.unwrap() {
            Ok(()) => successes += 1,
            Err(guardian::Error::InvalidTransition { .. }) => conflicts += 1,
            Err(error) => panic!("unexpected concurrent start error: {error}"),
        }
    }

    assert_eq!(successes, 1);
    assert_eq!(conflicts, 15);
    await_status(&mut events, ServerStatus::Online).await;
    let snapshot = guardian.snapshot().await;
    assert!(snapshot.pid.is_some());

    guardian.stop().await.unwrap();
    assert_eq!(guardian.status().await, ServerStatus::Offline);
    assert!(guardian.snapshot().await.pid.is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn stopping_an_offline_server_is_rejected() {
    let tmp = tempfile::tempdir().unwrap();
    let guardian = guardian_with(tmp.path(), GuardianConfig::default(), &["exit:0"]);

    let error = guardian.stop().await.unwrap_err();
    assert!(matches!(error, guardian::Error::InvalidTransition { .. }));
}

#[tokio::test(flavor = "multi_thread")]
async fn killing_an_offline_server_is_rejected_without_poisoning_state() {
    let tmp = tempfile::tempdir().unwrap();
    let guardian = guardian_with(tmp.path(), GuardianConfig::default(), &["exit:0"]);

    let error = guardian.kill().await.unwrap_err();
    assert!(matches!(error, guardian::Error::InvalidTransition { .. }));
    assert_eq!(guardian.status().await, ServerStatus::Offline);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_server_that_ignores_stop_is_killed_after_the_timeout() {
    let tmp = tempfile::tempdir().unwrap();
    // This one never exits on "stop", the way a deadlocked server would not.
    let policy = GuardianConfig {
        stop_timeout_secs: 1,
        ..GuardianConfig::default()
    };
    let guardian = guardian_with(tmp.path(), policy, &["done", "hang"]);
    let mut events = guardian.subscribe();

    guardian.start().await.unwrap();
    await_status(&mut events, ServerStatus::Online).await;

    guardian.stop().await.unwrap();

    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    while guardian.snapshot().await.pid.is_some() && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    assert!(
        guardian.snapshot().await.pid.is_none(),
        "the process was never killed"
    );
    assert!(guardian
        .console()
        .await
        .iter()
        .any(|l| l.line.contains("killing process")));
}

/// The panel's default `--data-dir` is `./data`, which made every launch fail:
/// the JVM is spawned with its working directory set to the server folder, so
/// `./data/servers/<id>/server.jar` was looked for *inside* that folder and
/// reported as "Unable to access jarfile".
///
/// Reproducing it needs the server directory to sit below the process's working
/// directory, exactly as `./data/servers/<id>` sits below the panel's. A temp
/// directory elsewhere on the filesystem would be reached through `../..`,
/// which happens to resolve the same way from either base and hides the bug.
#[tokio::test(flavor = "multi_thread")]
async fn a_relative_jar_path_still_launches() {
    let cwd = std::env::current_dir().unwrap();
    let tmp = tempfile::tempdir_in(&cwd).unwrap();

    let relative_dir = tmp.path().strip_prefix(&cwd).unwrap().to_path_buf();
    assert!(relative_dir.is_relative() && !relative_dir.starts_with(".."));
    std::fs::write(tmp.path().join("server.jar"), b"not really a jar").unwrap();
    let relative_jar = relative_dir.join("server.jar");

    let mut config = ServerConfig::paper(tmp.path(), "1.21.8");
    config.eula_accepted = true;
    config.server_args = vec!["done".into(), "serve".into()];

    let guardian = Guardian::new(config, GuardianConfig::default(), tmp.path());
    guardian
        .set_environment(ServerEnvironment {
            java: fake_java(),
            java_major: 21,
            jar: relative_jar,
            directory: tmp.path().to_path_buf(),
        })
        .await;

    let mut events = guardian.subscribe();
    guardian.start().await.unwrap();

    // Without absolutizing, this times out in Crashed instead of reaching Online.
    await_status(&mut events, ServerStatus::Online).await;

    guardian.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn the_console_buffer_is_bounded() {
    let tmp = tempfile::tempdir().unwrap();
    let policy = GuardianConfig {
        console_buffer: 50,
        ..GuardianConfig::default()
    };
    let guardian = guardian_with(tmp.path(), policy, &["spam:200", "done", "serve"]);
    let mut events = guardian.subscribe();

    guardian.start().await.unwrap();
    await_status(&mut events, ServerStatus::Online).await;

    let console = guardian.console().await;
    assert!(console.len() <= 50, "buffer grew to {}", console.len());
    // Sequence numbers keep rising even though old lines were dropped.
    assert!(console.last().unwrap().seq >= 200);

    guardian.stop().await.unwrap();
}

/// A launch that cannot even spawn must settle, not sit in `Preparing`.
#[tokio::test(flavor = "multi_thread")]
async fn a_launch_that_cannot_spawn_returns_to_offline() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("server.jar"), b"not really a jar").unwrap();

    let mut config = ServerConfig::paper(tmp.path(), "1.21.8");
    config.eula_accepted = true;

    let guardian = Guardian::new(config, GuardianConfig::default(), tmp.path());
    guardian
        .set_environment(ServerEnvironment {
            java: PathBuf::from("/nonexistent/bin/java"),
            java_major: 21,
            jar: tmp.path().join("server.jar"),
            directory: tmp.path().to_path_buf(),
        })
        .await;

    let mut events = guardian.subscribe();

    // `start` reports success because the work was accepted, not completed.
    guardian.start().await.unwrap();
    await_status(&mut events, ServerStatus::Offline).await;

    assert!(guardian
        .console()
        .await
        .iter()
        .any(|l| l.line.contains("could not start")));

    // And the failure must leave the server startable again.
    assert!(guardian.start().await.is_ok());
}

/// Provisioning that never finishes must not pin the server in `Preparing`.
#[tokio::test(flavor = "multi_thread")]
async fn provisioning_that_overruns_its_limit_gives_up() {
    let tmp = tempfile::tempdir().unwrap();

    let mut config = ServerConfig::paper(tmp.path(), "1.21.8");
    config.eula_accepted = true;

    // No cached environment, so this really enters provisioning — and a zero
    // budget means it cannot finish, whatever the network is doing.
    let policy = GuardianConfig {
        prepare_timeout_secs: 0,
        ..GuardianConfig::default()
    };
    let guardian = Guardian::new(config, policy, tmp.path());
    let mut events = guardian.subscribe();

    guardian.start().await.unwrap();
    await_status(&mut events, ServerStatus::Offline).await;

    assert!(guardian.snapshot().await.pid.is_none());
    assert!(
        guardian.start().await.is_ok(),
        "the server must be startable again"
    );
}

/// The exact dead end from the bug report: a provision the operator abandons.
#[tokio::test(flavor = "multi_thread")]
async fn an_abandoned_provision_can_be_stopped_and_started_again() {
    let tmp = tempfile::tempdir().unwrap();

    let mut config = ServerConfig::paper(tmp.path(), "1.21.8");
    config.eula_accepted = true;
    // A Java nobody has, so provisioning has real work to do and cannot race
    // to completion before the stop below.
    config.java_major = 999;

    let guardian = Guardian::new(config, GuardianConfig::default(), tmp.path());

    guardian.start().await.unwrap();
    assert_eq!(guardian.status().await, ServerStatus::Preparing);

    // Previously this was rejected, and nothing short of restarting the panel
    // could clear the status. An error here is tolerated only for the case where
    // provisioning has already failed on its own — a slow network makes that a
    // race, and the property under test is the state it ends in either way.
    let _ = guardian.stop().await;

    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    while guardian.status().await == ServerStatus::Preparing
        && tokio::time::Instant::now() < deadline
    {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    assert_eq!(guardian.status().await, ServerStatus::Offline);
    assert!(guardian.snapshot().await.pid.is_none());
    assert!(
        guardian.start().await.is_ok(),
        "the server must be startable again"
    );
}

/// `kill` is the other escape hatch, and has to work the same way.
#[tokio::test(flavor = "multi_thread")]
async fn killing_during_provisioning_also_recovers() {
    let tmp = tempfile::tempdir().unwrap();

    let mut config = ServerConfig::paper(tmp.path(), "1.21.8");
    config.eula_accepted = true;
    config.java_major = 999;

    let guardian = Guardian::new(config, GuardianConfig::default(), tmp.path());

    guardian.start().await.unwrap();
    assert_eq!(guardian.status().await, ServerStatus::Preparing);

    let _ = guardian.kill().await;

    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    while guardian.status().await == ServerStatus::Preparing
        && tokio::time::Instant::now() < deadline
    {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    assert_eq!(guardian.status().await, ServerStatus::Offline);
}
