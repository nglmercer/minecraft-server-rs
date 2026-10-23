export interface CompatibilityView {
  availability: "available" | "unavailable" | "unknown";
  support: "supported" | "deprecated" | "unsupported" | "unknown";
  java_minimum: number | null;
}

/** Mirrors `guardian::ServerStatus`. */
export type Status =
  | "offline"
  | "preparing"
  | "starting"
  | "online"
  | "stopping"
  | "crashed";

export interface Memory {
  min_mb: number;
  max_mb: number;
}

export interface Policy {
  auto_restart: boolean;
  max_retries: number;
  retry_delay_secs: number;
  stop_timeout_secs: number;
  console_buffer: number;
}

export type PlayitConnectionState =
  | "connected"
  | "needs_claim"
  | "starting"
  | "reconnecting"
  | "stopping"
  | "unavailable"
  | "unsupported"
  | "error";

export type PlayitAccountStatus =
  | "unknown"
  | "guest"
  | "email_not_verified"
  | "verified";

export type PlayitProtocol = "tcp" | "udp" | "both";

export interface PlayitStatus {
  status: PlayitConnectionState;
  version: string | null;
  message: string | null;
}

export interface PlayitAccount {
  status: PlayitAccountStatus;
  agent_id: string | null;
  login_link: string | null;
  claim_url: string | null;
}

export interface PlayitAuthSession {
  authenticated: boolean;
  requires_totp: boolean;
  account_id: number | null;
  account_status: string | null;
  read_only: boolean;
}

export interface PlayitAgent {
  id: string;
  name: string;
}

export interface PlayitClaimDetails {
  agent_type: string;
  name: string;
  remote_ip: string;
  version: string;
}

export interface PlayitDirectSetup {
  agent_id: string | null;
  already_configured: boolean;
  connected: boolean;
  message: string | null;
}

export interface PlayitDomain {
  id: string;
  name: string;
}

export interface PlayitTunnel {
  id: string;
  name: string | null;
  display_address: string;
  destination: string;
  protocol: PlayitProtocol;
  tunnel_type: string | null;
  agent_id: string | null;
  local_address: string | null;
  local_port: number | null;
  disabled: boolean;
  disabled_reason: string | null;
}

/** Which authority a tunnel listing was read from. */
export type TunnelSource = "agent" | "account" | "none";

/** A tunnel listing together with its source authority. */
export interface TunnelCatalog {
  available: boolean;
  source: TunnelSource;
  tunnels: PlayitTunnel[];
}

/** Whether the runtime agent is owned by the logged-in account. */
export type AgentOwnership = "matched" | "different_account" | "no_agent" | "unknown";

/** The verified relationship between the runtime agent and the account. */
export interface AgentOwnershipInfo {
  ownership: AgentOwnership;
  agent_id: string | null;
}

/** The outcome of switching to a different Playit account. */
export interface ChangeAccountResult {
  session: PlayitAuthSession;
  /** Null while a TOTP code is still pending for the new account. */
  setup: PlayitDirectSetup | null;
  ownership_before: AgentOwnership;
  servers_recovered: number;
  servers_total: number;
}

/** The TOTP result: the verified session plus any automatic account-change continuation. */
export interface TotpResponse {
  session: PlayitAuthSession;
  /** The automatic setup after a TOTP-pending account change; null for plain logins. */
  setup: PlayitDirectSetup | null;
  servers_recovered: number;
  servers_total: number;
  /** Set when verification succeeded but the automatic setup failed (session stays live). */
  setup_error: string | null;
}

export interface PlayitBinding {
  tunnel_id: string;
  protocol: PlayitProtocol;
  local_address: string;
  local_port: number;
  agent_id: string | null;
  created_at: number | null;
}

export type ServerPlayitState =
  | "disabled"
  | "provisioning"
  | "connected"
  | "reconnecting"
  | "disabled_by_playit"
  | "drifted"
  | "missing"
  | "account_mismatch"
  | "agent_mismatch"
  | "ambiguous"
  | "unavailable";

/** How a server tunnel attach obtained its tunnel. Mirrors the backend. */
export type PlayitAttachDisposition = "created" | "reused" | "updated";

export interface ServerPlayitView {
  state: ServerPlayitState;
  binding: PlayitBinding | null;
  tunnel: PlayitTunnel | null;
  message: string | null;
  cleanup_pending: boolean;
  /** Present only on responses that directly follow an attach. */
  disposition?: PlayitAttachDisposition | null;
}

/** Mirrors the flattened `ServerView` returned by the API. */
export interface Server {
  id: string;
  name: string;
  core: string;
  version: string;
  port: number;
  java_major: number;
  memory: Memory;
  eula_accepted: boolean;
  jvm_args: string[];
  server_args: string[];
  policy: Policy;
  created_at: string;
  status: Status;
  pid: number | null;
  uptime_secs: number | null;
  crashes: number;
  /** Current provisioning activity, present while the server is preparing. */
  progress: ProgressState | null;
  metrics: ProcessMetrics | null;
  installed: Installation | null;
  /** True when starting would download a new artifact first. */
  needs_install: boolean;
  /** Bytes on disk in this server's directory. */
  disk_bytes: number;
  /** Panel-owned Playit association, when configured. */
  playit: PlayitBinding | null;
  /** True when the running process was launched with a different configuration. */
  pending_restart: boolean;
}

export interface ProgressState {
  stage: string;
  fraction: number | null;
}

/** The Guardian snapshot sent with the console WebSocket backfill. */
export interface ServerSnapshot {
  status: Status;
  pid: number | null;
  uptime_secs: number | null;
  crashes: number;
  progress: ProgressState | null;
}

export interface ProcessMetrics {
  cpu_percent: number | null;
  /** Approximate number of logical CPUs consumed by the process. */
  cpu_cores: number | null;
  /** Process CPU normalized against the panel host's logical CPU capacity. */
  cpu_host_percent: number | null;
  logical_cpu_count: number;
  /** Resident memory (RSS), in MiB. */
  memory_mb: number;
}

export interface Backup {
  id: string;
  created_at: string;
  size: number;
  size_bytes: number;
  note: string;
  provider?: "local" | "google_drive";
  remote_id?: string;
  checksum_sha256?: string | null;
  server_id?: string;
}

export interface BackupRetention {
  max_backups: number;
  max_age_days?: number | null;
}

export interface BackupStorageSettings {
  provider: "local" | "google_drive";
  retention: BackupRetention;
  google_drive?: {
    folder_id: string;
    credentials_present: boolean;
    configured: boolean;
    oauth_connected?: boolean;
    oauth_configured?: boolean;
  } | null;
}

export interface ServerBackupSettings {
  inherit_global: boolean;
  provider?: "local" | "google_drive" | null;
  retention?: BackupRetention | null;
  google_drive?: {
    folder_id: string;
    credentials_present: boolean;
    configured: boolean;
    oauth_connected?: boolean;
    oauth_configured?: boolean;
  } | null;
}

export interface Project {
  project_id: string;
  slug: string;
  title: string;
  description: string;
  downloads: number;
  icon_url: string | null;
  categories: string[];
}

export interface Installed {
  filename: string;
  path: string;
  size: number;
}

export interface PanelUser {
  username: string;
  admin: boolean;
  servers: string[];
}

export interface Installation {
  core: string;
  version: string;
  build: string;
  java_major: number;
  java: string;
  jar: string;
  installed_at: string;
}

export interface ConsoleLine {
  seq: number;
  stream: "stdout" | "stderr" | "system";
  line: string;
}

export type ServerEvent =
  | { type: "status"; status: Status }
  | { type: "console"; seq: number; stream: ConsoleLine["stream"]; line: string }
  | { type: "started"; pid: number }
  | { type: "stopped"; code: number | null }
  | { type: "crashed"; code: number | null; attempt: number }
  | { type: "progress"; stage: string; fraction: number | null }
  | {
      type: "backfill";
      status: ServerSnapshot;
      lines: ConsoleLine[];
      /** Highest console sequence represented by this backfill. */
      through_seq: number | null;
    }
  | { type: "lagged"; skipped: number };

export interface FileEntry {
  name: string;
  path: string;
  directory: boolean;
  size: number;
  modified: number | null;
}

export interface SystemStats {
  cpu_percent: number | null;
  /** Android host load may be estimated from kernel idle counters. */
  cpu_estimated?: boolean;
  memory_used_mb: number;
  memory_total_mb: number;
  servers_online: number;
}

export interface JavaInstall {
  major: number;
  version: string;
  /** Present for administrators; omitted for regular server operators. */
  path?: string;
  vendor: string | null;
  jdk: boolean;
}

export interface User {
  username: string;
  admin: boolean;
}
