//! Short-lived grants for browser-driven downloads.
//!
//! A download has to be a plain navigation — the browser saves the file, and it
//! cannot attach an `Authorization` header to that. Putting the session token in
//! the query string solved it and created a worse problem: the token then lands
//! in browser history, in any proxy's access log, and in the panel's own request
//! log, where a session credential has no business being.
//!
//! A ticket names one resource, expires in a minute, and grants nothing else.

use rand::RngCore;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How long a ticket is accepted after being issued.
const TTL: Duration = Duration::from_secs(60);
const CONSOLE_TTL: Duration = Duration::from_secs(45);
const MAX_TICKETS: usize = 4096;

/// Exactly what a ticket permits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resource {
    /// One file inside one server's directory.
    File { server: String, path: String },
    /// One backup archive of one server.
    Backup { server: String, backup: String },
}

struct ConsoleGrant {
    issued: Instant,
    server: String,
    session_token: String,
}

/// Issues and redeems download tickets.
#[derive(Default)]
pub struct Tickets {
    inner: Mutex<HashMap<String, (Instant, Resource)>>,
    console: Mutex<HashMap<String, ConsoleGrant>>,
}

impl Tickets {
    fn random() -> String {
        let mut bytes = [0u8; 24];
        rand::rng().fill_bytes(&mut bytes);
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// Grant access to `resource` for [`TTL`].
    pub fn issue(&self, resource: Resource) -> String {
        let ticket = Self::random();

        if let Ok(mut tickets) = self.inner.lock() {
            // Cleared here rather than on a timer: the table only grows when
            // someone is downloading, and every issue is a chance to sweep.
            tickets.retain(|_, (issued, _)| issued.elapsed() < TTL);
            while tickets.len() >= MAX_TICKETS {
                let Some(oldest) = tickets
                    .iter()
                    .min_by_key(|(_, (issued, _))| *issued)
                    .map(|(ticket, _)| ticket.clone())
                else {
                    break;
                };
                tickets.remove(&oldest);
            }
            tickets.insert(ticket.clone(), (Instant::now(), resource));
        }

        ticket
    }

    /// Issue a short-lived, server-scoped, single-use console grant.
    pub fn issue_console(&self, server: String, session_token: String) -> String {
        let ticket = Self::random();
        if let Ok(mut tickets) = self.console.lock() {
            tickets.retain(|_, grant| grant.issued.elapsed() < CONSOLE_TTL);
            while tickets.len() >= MAX_TICKETS {
                let Some(oldest) = tickets
                    .iter()
                    .min_by_key(|(_, grant)| grant.issued)
                    .map(|(ticket, _)| ticket.clone())
                else {
                    break;
                };
                tickets.remove(&oldest);
            }
            tickets.insert(
                ticket.clone(),
                ConsoleGrant {
                    issued: Instant::now(),
                    server,
                    session_token,
                },
            );
        }
        ticket
    }

    /// Consume a console grant if it is fresh and return its session binding.
    pub fn redeem_console(&self, ticket: &str) -> Option<(String, String)> {
        let mut tickets = self.console.lock().ok()?;
        let grant = tickets.remove(ticket)?;
        (grant.issued.elapsed() < CONSOLE_TTL).then_some((grant.server, grant.session_token))
    }

    /// The resource a ticket grants, if it is still valid.
    ///
    /// Tickets are not consumed on use: a browser that retries a download, or
    /// issues a range request, would otherwise fail halfway through a file.
    pub fn redeem(&self, ticket: &str) -> Option<Resource> {
        let tickets = self.inner.lock().ok()?;
        let (issued, resource) = tickets.get(ticket)?;

        (issued.elapsed() < TTL).then(|| resource.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file() -> Resource {
        Resource::File {
            server: "abc".into(),
            path: "server.properties".into(),
        }
    }

    #[test]
    fn a_ticket_grants_exactly_the_resource_it_was_issued_for() {
        let tickets = Tickets::default();
        let ticket = tickets.issue(file());

        assert_eq!(tickets.redeem(&ticket), Some(file()));
    }

    #[test]
    fn an_unknown_ticket_grants_nothing() {
        let tickets = Tickets::default();
        assert!(tickets.redeem("deadbeef").is_none());
        assert!(tickets.redeem("").is_none());
    }

    #[test]
    fn tickets_are_long_and_unique() {
        let tickets = Tickets::default();
        let a = tickets.issue(file());
        let b = tickets.issue(file());

        assert_eq!(a.len(), 48, "192 bits, hex encoded");
        assert_ne!(a, b);
    }

    #[test]
    fn a_ticket_for_one_file_does_not_open_another() {
        let tickets = Tickets::default();
        let ticket = tickets.issue(file());

        let granted = tickets.redeem(&ticket).unwrap();
        assert_ne!(
            granted,
            Resource::File {
                server: "abc".into(),
                path: "ops.json".into()
            }
        );
    }

    #[test]
    fn console_ticket_is_scoped_and_single_use() {
        let tickets = Tickets::default();
        let ticket = tickets.issue_console("server-a".into(), "session-a".into());

        assert_eq!(
            tickets.redeem_console(&ticket),
            Some(("server-a".into(), "session-a".into()))
        );
        assert!(
            tickets.redeem_console(&ticket).is_none(),
            "console grants are consumed at the handshake"
        );
    }

    #[test]
    fn download_and_console_ticket_namespaces_do_not_interoperate() {
        let tickets = Tickets::default();
        let download = tickets.issue(file());
        let console = tickets.issue_console("server-a".into(), "session-a".into());

        assert!(tickets.redeem_console(&download).is_none());
        assert!(tickets.redeem(&console).is_none());
    }
}
