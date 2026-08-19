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
}

export interface ProcessMetrics {
  cpu_percent: number;
  memory_mb: number;
}

export interface Backup {
  id: string;
  created_at: string;
  size: number;
  note: string;
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
  path: string;
  vendor: string | null;
  jdk: boolean;
}

export interface User {
  username: string;
  admin: boolean;
}
