//! Server status and the event stream emitted by a [`Guardian`](crate::Guardian).

use serde::{Deserialize, Serialize};

/// Lifecycle state of a single server.
///
/// The panel renders this directly, so the serialised form is part of the API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ServerStatus {
    /// No process, and none wanted.
    Offline,
    /// Downloading Java and/or the server jar.
    Preparing,
    /// The JVM is up but the server has not logged "Done".
    Starting,
    /// The server has finished loading and accepts players.
    Online,
    /// A graceful shutdown is in progress.
    Stopping,
    /// The process exited without being asked to.
    Crashed,
}

impl ServerStatus {
    /// A stable lowercase name, used in errors and logs.
    pub fn as_str(self) -> &'static str {
        match self {
            ServerStatus::Offline => "offline",
            ServerStatus::Preparing => "preparing",
            ServerStatus::Starting => "starting",
            ServerStatus::Online => "online",
            ServerStatus::Stopping => "stopping",
            ServerStatus::Crashed => "crashed",
        }
    }

    /// Whether a process is expected to exist in this state.
    pub fn is_running(self) -> bool {
        matches!(
            self,
            ServerStatus::Starting | ServerStatus::Online | ServerStatus::Stopping
        )
    }
}

/// Which stream a console line came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Stream {
    /// The process's stdout.
    Stdout,
    /// The process's stderr.
    Stderr,
    /// Emitted by the guardian itself, not by the JVM.
    System,
}

/// One retained console line.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsoleLine {
    /// Monotonic sequence number, so clients can resume without duplicates.
    pub seq: u64,
    /// Where the line came from.
    pub stream: Stream,
    /// The line itself, without its trailing newline.
    pub line: String,
}

/// Everything a guardian reports about its server.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerEvent {
    /// The status changed. Always sent, even for redundant-looking transitions.
    Status {
        /// The new status.
        status: ServerStatus,
    },
    /// A line was written to the console.
    Console(ConsoleLine),
    /// The JVM was spawned.
    Started {
        /// Operating-system process id.
        pid: u32,
    },
    /// The process exited as requested.
    Stopped {
        /// Exit code, when the platform reported one.
        code: Option<i32>,
    },
    /// The process exited without being asked to.
    Crashed {
        /// Exit code, when the platform reported one.
        code: Option<i32>,
        /// Consecutive crash count, including this one.
        attempt: u32,
    },
    /// Provisioning progress, for the setup UI.
    Progress {
        /// What is happening, e.g. `"downloading paper 1.21.8"`.
        stage: String,
        /// Completion in `0.0..=1.0`, when it can be determined.
        fraction: Option<f32>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_states_with_a_process_count_as_running() {
        assert!(ServerStatus::Starting.is_running());
        assert!(ServerStatus::Online.is_running());
        assert!(ServerStatus::Stopping.is_running());

        assert!(!ServerStatus::Offline.is_running());
        assert!(!ServerStatus::Crashed.is_running());
        // Preparing is downloading, not running: nothing has been spawned yet.
        assert!(!ServerStatus::Preparing.is_running());
    }

    #[test]
    fn status_serialises_as_the_lowercase_name_the_ui_expects() {
        let json = serde_json::to_string(&ServerStatus::Online).unwrap();
        assert_eq!(json, "\"online\"");
        assert_eq!(ServerStatus::Online.as_str(), "online");
    }

    #[test]
    fn events_are_tagged_so_the_client_can_switch_on_type() {
        let event = ServerEvent::Crashed {
            code: Some(1),
            attempt: 2,
        };
        let json = serde_json::to_value(&event).unwrap();

        assert_eq!(json["type"], "crashed");
        assert_eq!(json["code"], 1);
        assert_eq!(json["attempt"], 2);
    }

    #[test]
    fn console_events_carry_the_line_at_the_top_level() {
        let event = ServerEvent::Console(ConsoleLine {
            seq: 7,
            stream: Stream::Stdout,
            line: "Done (1.0s)!".into(),
        });
        let json = serde_json::to_value(&event).unwrap();

        assert_eq!(json["type"], "console");
        assert_eq!(json["seq"], 7);
        assert_eq!(json["stream"], "stdout");
    }
}
