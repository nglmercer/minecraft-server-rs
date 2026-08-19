//! End-to-end supervisor behaviour, driven with a fake JVM.
//!
//! These tests spawn a real child process and exercise the real status machine,
//! stdio pumps and restart policy — only the Java/jar provisioning is stubbed,
//! because downloading a JDK is not something a unit test should do.

#![cfg(unix)]

use guardian::events::Stream;
use guardian::{Guardian, GuardianConfig, ServerConfig, ServerEnvironment, ServerEvent, ServerStatus};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::sync::broadcast::Receiver;

/// Write an executable stand-in for `java` that behaves like a server.
fn fake_java(dir: &Path, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let path = dir.join("fake-java");
    // The launcher arguments (-Xms, -jar, nogui) are deliberately ignored: the
    // point is to exercise the supervisor, not to parse a JVM command line.
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

fn guardian_for(dir: &Path, java: PathBuf, policy: GuardianConfig) -> std::sync::Arc<Guardian> {
    let mut config = ServerConfig::paper(dir, "1.21.8");
    config.eula_accepted = true;

    let guardian = Guardian::new(config, policy, dir);
    let environment = ServerEnvironment {
        java,
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
    let java = fake_java(
        tmp.path(),
        r#"
echo 'Starting minecraft server version 1.21.8'
echo '[11:05:24 INFO]: Done (1.234s)! For help, type "help"'
while read -r line; do
  if [ "$line" = "stop" ]; then echo 'Stopping server'; exit 0; fi
  echo "handled: $line"
done
"#,
    );

    let guardian = guardian_for(tmp.path(), java, GuardianConfig::default());
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
    let java = fake_java(
        tmp.path(),
        r#"
echo '[11:05:24 INFO]: Done (0.5s)! For help, type "help"'
while read -r line; do
  if [ "$line" = "stop" ]; then exit 0; fi
  echo "echoed: $line"
done
"#,
    );

    let guardian = guardian_for(tmp.path(), java, GuardianConfig::default());
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
    let java = fake_java(tmp.path(), "echo 'boom' >&2\nexit 1");

    let policy = GuardianConfig {
        auto_restart: true,
        max_retries: 2,
        retry_delay_secs: 0,
        ..GuardianConfig::default()
    };

    let guardian = guardian_for(tmp.path(), java, policy);
    let mut events = guardian.subscribe();

    guardian.start().await.unwrap();

    // Three exits in total: the original plus two restarts.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    let mut attempts = Vec::new();
    while attempts.len() < 3 && tokio::time::Instant::now() < deadline {
        let remaining = deadline - tokio::time::Instant::now();
        let Ok(Ok(event)) = tokio::time::timeout(remaining, events.recv()).await else { break };
        if let ServerEvent::Crashed { attempt, code } = event {
            assert_eq!(code, Some(1));
            attempts.push(attempt);
        }
    }

    assert_eq!(attempts, vec![1, 2, 3], "expected the original plus two restarts");
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
    let java = fake_java(tmp.path(), "exit 3");

    let policy = GuardianConfig { auto_restart: false, ..GuardianConfig::default() };
    let guardian = guardian_for(tmp.path(), java, policy);
    let mut events = guardian.subscribe();

    guardian.start().await.unwrap();
    await_status(&mut events, ServerStatus::Crashed).await;

    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(guardian.snapshot().await.crashes, 1, "it must not have retried");
}

#[tokio::test(flavor = "multi_thread")]
async fn starting_twice_is_rejected_rather_than_spawning_two_jvms() {
    let tmp = tempfile::tempdir().unwrap();
    let java = fake_java(
        tmp.path(),
        r#"
echo '[11:05:24 INFO]: Done (0.5s)! For help, type "help"'
while read -r line; do
  if [ "$line" = "stop" ]; then exit 0; fi
done
"#,
    );

    let guardian = guardian_for(tmp.path(), java, GuardianConfig::default());
    let mut events = guardian.subscribe();

    guardian.start().await.unwrap();
    await_status(&mut events, ServerStatus::Online).await;

    let pid = guardian.snapshot().await.pid;
    let error = guardian.start().await.unwrap_err();
    assert!(matches!(error, guardian::Error::InvalidTransition { .. }));
    assert_eq!(guardian.snapshot().await.pid, pid, "the original process must survive");

    guardian.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn stopping_an_offline_server_is_rejected() {
    let tmp = tempfile::tempdir().unwrap();
    let java = fake_java(tmp.path(), "exit 0");
    let guardian = guardian_for(tmp.path(), java, GuardianConfig::default());

    let error = guardian.stop().await.unwrap_err();
    assert!(matches!(error, guardian::Error::InvalidTransition { .. }));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_server_that_ignores_stop_is_killed_after_the_timeout() {
    let tmp = tempfile::tempdir().unwrap();
    // This one never exits on "stop", the way a deadlocked server would not.
    let java = fake_java(
        tmp.path(),
        r#"
echo '[11:05:24 INFO]: Done (0.5s)! For help, type "help"'
while true; do sleep 1; done
"#,
    );

    let policy = GuardianConfig { stop_timeout_secs: 1, ..GuardianConfig::default() };
    let guardian = guardian_for(tmp.path(), java, policy);
    let mut events = guardian.subscribe();

    guardian.start().await.unwrap();
    await_status(&mut events, ServerStatus::Online).await;

    guardian.stop().await.unwrap();

    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    while guardian.snapshot().await.pid.is_some() && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    assert!(guardian.snapshot().await.pid.is_none(), "the process was never killed");
    assert!(guardian
        .console()
        .await
        .iter()
        .any(|l| l.line.contains("killing process")));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_console_buffer_is_bounded() {
    let tmp = tempfile::tempdir().unwrap();
    let java = fake_java(
        tmp.path(),
        r#"
i=0
while [ $i -lt 200 ]; do echo "line $i"; i=$((i+1)); done
echo '[11:05:24 INFO]: Done (0.5s)! For help, type "help"'
while read -r line; do
  if [ "$line" = "stop" ]; then exit 0; fi
done
"#,
    );

    let policy = GuardianConfig { console_buffer: 50, ..GuardianConfig::default() };
    let guardian = guardian_for(tmp.path(), java, policy);
    let mut events = guardian.subscribe();

    guardian.start().await.unwrap();
    await_status(&mut events, ServerStatus::Online).await;

    let console = guardian.console().await;
    assert!(console.len() <= 50, "buffer grew to {}", console.len());
    // Sequence numbers keep rising even though old lines were dropped.
    assert!(console.last().unwrap().seq >= 200);

    guardian.stop().await.unwrap();
}
