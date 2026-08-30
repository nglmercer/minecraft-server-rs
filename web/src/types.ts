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

export interface PlayitBinding {
  tunnel_id: string;
  protocol: PlayitProtocol;
  local_address: string;
  local_port: number;
}

export type ServerPlayitState =
  | "disabled"
  | "provisioning"
  | "connected"
  | "disabled_by_playit"
  | "drifted"
  | "unavailable";

export interface ServerPlayitView {
  state: ServerPlayitState;
  binding: PlayitBinding | null;
  tunnel: PlayitTunnel | null;
  message: string | null;
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

export interface ProcessMetrics {
  cpu_percent: number;
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
  | { type: "backfill"; status: Server; lines: ConsoleLine[] }
  | { type: "lagged"; skipped: number };

export interface FileEntry {
  name: string;
  path: string;
  directory: boolean;
  size: number;
  modified: number | null;
}

export interface SystemStats {
  cpu_percent: number;
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
