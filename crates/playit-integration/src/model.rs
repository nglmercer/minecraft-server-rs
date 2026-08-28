//! Panel-facing Playit models.

use serde::{Deserialize, Serialize};

/// The availability of the local Playit service.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlayitConnectionState {
    /// The Playit service is running with a configured account.
    Connected,
    /// The Playit service is running but needs account setup through the claim flow.
    NeedsClaim,
    /// The Playit service is starting.
    Starting,
    /// The Playit control connection is recovering while the service remains alive.
    Reconnecting,
    /// The Playit service is stopping.
    Stopping,
    /// No usable Playit service is available.
    Unavailable,
    /// An external Playit daemon speaks an incompatible IPC protocol.
    Unsupported,
    /// The Playit service reported an operational error.
    Error,
}

/// A safe summary of Playit's local service state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayitStatus {
    /// The state a panel UI should display.
    pub status: PlayitConnectionState,
    /// The Playit service version, when it could be read.
    pub version: Option<String>,
    /// A human-readable diagnostic, when one is available.
    pub message: Option<String>,
}

/// The account state reported by Playit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlayitAccountStatus {
    /// The account state is not known yet.
    Unknown,
    /// A guest or not-yet-claimed account.
    Guest,
    /// The account has an email verification pending.
    EmailNotVerified,
    /// The account is ready for normal use.
    Verified,
}

/// Account information that is safe to send to the panel UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayitAccount {
    /// The account state, without exposing the account secret.
    pub status: PlayitAccountStatus,
    /// The Playit agent's public identifier.
    pub agent_id: Option<String>,
    /// A login link supplied by Playit, if any.
    pub login_link: Option<String>,
    /// A claim link supplied by Playit, if any.
    pub claim_url: Option<String>,
}

/// The URL the operator should open to claim/configure Playit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaimInfo {
    /// The Playit claim URL.
    pub claim_url: String,
}

/// Supported transport protocols for a tunnel.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlayitProtocol {
    /// TCP, which is the protocol used by Java Minecraft.
    #[default]
    Tcp,
    /// UDP, useful for future Bedrock support.
    Udp,
    /// Both TCP and UDP.
    Both,
}

/// A tunnel known to the Playit service.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayitTunnel {
    /// Stable Playit tunnel identifier.
    pub id: String,
    /// Optional operator-facing name.
    pub name: Option<String>,
    /// Public address players can use.
    pub display_address: String,
    /// Destination represented by the Playit service.
    pub destination: String,
    /// Transport protocol.
    pub protocol: PlayitProtocol,
    /// Playit's semantic tunnel type, when the account API supplies one.
    pub tunnel_type: Option<String>,
    /// Agent currently assigned to the tunnel, when known.
    pub agent_id: Option<String>,
    /// Local bind address, when supplied by the Playit service.
    pub local_address: Option<String>,
    /// Local destination port, when supplied by the Playit service.
    pub local_port: Option<u16>,
    /// Whether Playit has disabled this tunnel.
    pub disabled: bool,
    /// Why the tunnel is disabled, when supplied.
    pub disabled_reason: Option<String>,
}

/// The immediate result of creating a tunnel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TunnelCreateInfo {
    /// Stable Playit tunnel identifier.
    pub tunnel_id: String,
    /// Optional Playit service message.
    pub message: Option<String>,
}
