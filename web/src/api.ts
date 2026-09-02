import type {
  Backup,
  BackupStorageSettings,
  ConsoleLine,
  FileEntry,
  Installed,
  JavaInstall,
  PanelUser,
  PlayitAccount,
  PlayitStatus,
  PlayitTunnel,
  Project,
  Server,
  ServerPlayitView,
  SystemStats,
  User,
} from "./types";

/** Session credentials are held in an HttpOnly cookie, never in JavaScript storage. */
export function token(): string | null {
  return null;
}

function csrfToken(): string | null {
  const item = document.cookie
    .split(";")
    .map((part) => part.trim())
    .find((part) => part.startsWith("mcpanel_csrf="));
  return item?.slice("mcpanel_csrf=".length) ?? null;
}

/** An API call that came back with a non-2xx status. */
export class ApiError extends Error {
  constructor(
    message: string,
    readonly status: number,
  ) {
    super(message);
  }
}

async function request<T>(
  path: string,
  init: RequestInit = {},
): Promise<T> {
  const headers = new Headers(init.headers);
  if (init.body) headers.set("Content-Type", "application/json");
  if (!["GET", "HEAD", "OPTIONS"].includes((init.method ?? "GET").toUpperCase())) {
    const csrf = csrfToken();
    if (csrf) headers.set("X-CSRF-Token", csrf);
  }

  const response = await fetch(`/api${path}`, {
    ...init,
    headers,
    credentials: "same-origin",
  });

  if (response.status === 401) {
    // The session is gone; drop it so the shell renders the login screen.
    window.dispatchEvent(new CustomEvent("mcpanel:logout"));
    throw new ApiError("session expired", 401);
  }

  if (!response.ok) {
    const body = await response.json().catch(() => ({}));
    throw new ApiError(body.error ?? response.statusText, response.status);
  }

  if (response.status === 204) return undefined as T;
  return response.json() as Promise<T>;
}

const json = (body: unknown) => JSON.stringify(body);

/** Navigate to `url` in a way the browser treats as a download. */
function startDownload(url: string) {
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.rel = "noreferrer";
  document.body.append(anchor);
  anchor.click();
  anchor.remove();
}

export const api = {
 async login(username: string, password: string): Promise<User> {
    const result = await request<{ user: User }>("/auth/login", {
     method: "POST",
     body: json({ username, password }),
   });
   return result.user;
 },

 async logout(): Promise<void> {
   await request("/auth/logout", { method: "POST" }).catch(() => {});
 },

  me: () => request<User>("/auth/me"),

  changePassword: (current: string, next: string) =>
    request<{ ok: boolean }>("/auth/password", {
      method: "POST",
      body: json({ current, new: next }),
    }),

  playitStatus: () => request<PlayitStatus>("/playit/status"),

  playitAccount: () => request<PlayitAccount>("/playit/account"),

  playitClaim: () => request<{ claim_url: string }>("/playit/claim", { method: "POST" }),

  playitTunnels: () => request<PlayitTunnel[]>("/playit/tunnels"),

  createPlayitTunnel: (body: {
    local_port: number;
    protocol: "tcp" | "udp" | "both";
    local_address?: string;
    name?: string;
  }) => request<{ tunnel_id: string; message: string | null }>("/playit/tunnels", {
    method: "POST",
    body: json(body),
  }),

  deletePlayitTunnel: (tunnelId: string) =>
    request<{ ok: boolean }>(`/playit/tunnels/${encodeURIComponent(tunnelId)}`, {
      method: "DELETE",
    }),

  servers: () => request<Server[]>("/servers"),

  server: (id: string) => request<Server>(`/servers/${id}`),

  serverPlayit: (id: string) => request<ServerPlayitView>(`/servers/${id}/playit`),

  attachPlayit: (id: string, name?: string) =>
    request<ServerPlayitView>(`/servers/${id}/playit`, {
      method: "POST",
      body: json(name ? { name } : {}),
    }),

  detachPlayit: (id: string) =>
    request<ServerPlayitView>(`/servers/${id}/playit`, { method: "DELETE" }),

  createServer: (body: Record<string, unknown>) =>
    request<Server>("/servers", { method: "POST", body: json(body) }),

  updateServer: (id: string, body: Record<string, unknown>) =>
    request<Server>(`/servers/${id}`, { method: "PATCH", body: json(body) }),

  deleteServer: (id: string) =>
    request<{ ok: boolean }>(`/servers/${id}`, { method: "DELETE" }),

  power: (id: string, action: "start" | "stop" | "restart" | "kill") =>
    request<Server>(`/servers/${id}/power`, {
      method: "POST",
      body: json({ action }),
    }),

  command: (id: string, command: string) =>
    request<{ ok: boolean }>(`/servers/${id}/command`, {
      method: "POST",
      body: json({ command }),
    }),

 logs: (id: string) => request<ConsoleLine[]>(`/servers/${id}/logs`),

  consoleTicket: (id: string) =>
    request<{ ticket: string }>(`/servers/${id}/ws/ticket`, { method: "POST" }),

  reinstall: (id: string) =>
    request<{ ok: boolean }>(`/servers/${id}/reinstall`, { method: "POST" }),

  prepare: (id: string) =>
    request<{ ok: boolean }>(`/servers/${id}/prepare`, { method: "POST" }),

  files: (id: string, path = "") =>
    request<FileEntry[]>(`/servers/${id}/files?path=${encodeURIComponent(path)}`),

  directorySizes: (id: string, path = "") =>
    request<{ path: string; bytes: number }[]>(
      `/servers/${id}/files/sizes?path=${encodeURIComponent(path)}`,
    ),

  readFile: (id: string, path: string) =>
    request<{ path: string; content: string }>(
      `/servers/${id}/files/read?path=${encodeURIComponent(path)}`,
    ),

  writeFile: (id: string, path: string, content: string) =>
    request<{ ok: boolean }>(`/servers/${id}/files`, {
      method: "PUT",
      body: json({ path, content }),
    }),

  deleteFile: (id: string, path: string) =>
    request<{ ok: boolean }>(
      `/servers/${id}/files?path=${encodeURIComponent(path)}`,
      { method: "DELETE" },
    ),

  mkdir: (id: string, path: string) =>
    request<{ ok: boolean }>(`/servers/${id}/files/mkdir`, {
      method: "POST",
      body: json({ path }),
    }),

  /**
   * Start a file download.
   *
   * A ticket is fetched first and the browser navigates to that. The session
   * token never enters a URL, so it stays out of history and access logs.
   */
  async download(id: string, path: string): Promise<void> {
    const { ticket } = await request<{ ticket: string }>(
      `/servers/${id}/files/ticket?path=${encodeURIComponent(path)}`,
      { method: "POST" },
    );
    startDownload(
      `/api/servers/${id}/files/download?ticket=${encodeURIComponent(ticket)}`,
    );
  },

  async upload(id: string, path: string, files: File[]): Promise<void> {
    const body = new FormData();
    for (const file of files) body.append("file", file, file.name);

   const headers = new Headers();
    const csrf = csrfToken();
    if (csrf) headers.set("X-CSRF-Token", csrf);

    const response = await fetch(
     `/api/servers/${id}/files/upload?path=${encodeURIComponent(path)}`,
      { method: "POST", body, headers, credentials: "same-origin" },
    );

    // Upload cannot go through `request` because the body is FormData, so the
    // expired-session handling has to be repeated rather than inherited.
    if (response.status === 401) {
     window.dispatchEvent(new CustomEvent("mcpanel:logout"));
      throw new ApiError("session expired", 401);
    }
    if (!response.ok) {
      const detail = await response.json().catch(() => ({}));
      throw new ApiError(detail.error ?? response.statusText, response.status);
    }
  },

  extract: (id: string, path: string, into?: string) =>
    request<{ ok: boolean; entries: number }>(`/servers/${id}/files/extract`, {
      method: "POST",
      body: json({ path, into }),
    }),

  rename: (id: string, from: string, to: string) =>
    request<{ ok: boolean }>(`/servers/${id}/files/rename`, {
      method: "POST",
      body: json({ from, to }),
    }),

  backups: (id: string) => request<Backup[]>(`/servers/${id}/backups`),

  createBackup: (id: string, note: string) =>
    request<Backup>(`/servers/${id}/backups`, { method: "POST", body: json({ note }) }),

  backupSettings: () => request<BackupStorageSettings>("/settings/backups"),

  updateBackupSettings: (body: Record<string, unknown>) =>
    request<BackupStorageSettings>("/settings/backups", { method: "PATCH", body: json(body) }),

  testBackupSettings: () =>
    request<{ ok: boolean; message: string }>("/settings/backups/test", { method: "POST" }),

  uploadBackupSecret: (content: string, credential_ref?: string) =>
    request<{ ok: boolean }>("/settings/backups/secret", { method: "POST", body: json({ content, credential_ref }) }),

  serverBackupSettings: (id: string) => request<import("./types").ServerBackupSettings>(`/servers/${id}/backup-settings`),

  updateServerBackupSettings: (id: string, body: Record<string, unknown>) =>
    request<import("./types").ServerBackupSettings>(`/servers/${id}/backup-settings`, { method: "PATCH", body: json(body) }),

  restoreBackup: (id: string, backup: string) =>
    request<{ ok: boolean }>(`/servers/${id}/backups/${backup}/restore`, { method: "POST" }),

  deleteBackup: (id: string, backup: string) =>
    request<{ ok: boolean }>(`/servers/${id}/backups/${backup}`, { method: "DELETE" }),

  async downloadBackup(id: string, backup: string): Promise<void> {
    const { ticket } = await request<{ ticket: string }>(
      `/servers/${id}/backups/${backup}/ticket`,
      { method: "POST" },
    );
    startDownload(
      `/api/servers/${id}/backups/${backup}/download?ticket=${encodeURIComponent(ticket)}`,
    );
  },

  searchMods: (id: string, q: string) =>
    request<Project[]>(`/servers/${id}/mods/search?q=${encodeURIComponent(q)}`),

  installedMods: (id: string) => request<Installed[]>(`/servers/${id}/mods`),

  installMod: (id: string, project: string) =>
    request<{ name: string; version: string; filename: string; path: string; size: number }>(
      `/servers/${id}/mods/install`,
      { method: "POST", body: json({ project }) },
    ),

  users: () => request<PanelUser[]>("/users"),

  createUser: (body: {
    username: string;
    password: string;
    admin: boolean;
    servers: string[];
  }) => request<PanelUser>("/users", { method: "POST", body: json(body) }),

  updateUser: (
    username: string,
    body: { password?: string; admin?: boolean; servers?: string[] },
  ) => request<PanelUser>(`/users/${encodeURIComponent(username)}`, {
    method: "PATCH",
    body: json(body),
  }),

  deleteUser: (username: string) =>
    request<{ ok: boolean }>(`/users/${encodeURIComponent(username)}`, { method: "DELETE" }),

  providers: () => request<{ id: string; server: boolean }[]>("/catalog/providers"),

  versions: (provider: string) =>
    request<string[]>(`/catalog/${provider}/versions`),

  javas: () => request<JavaInstall[]>("/catalog/javas"),

  system: () => request<SystemStats>("/system"),

  setupStatus: () => request<{ needs_setup: boolean }>("/setup/status"),

  setup: (body: { username: string; password: string; confirm: string }) =>
    request<{ ok: boolean }>("/setup", { method: "POST", body: json(body) }),

  recoveryReset: (body: { token: string; password: string; confirm: string }) =>
    request<{ ok: boolean }>("/recovery/reset", { method: "POST", body: json(body) }),
};

/** Open the console socket with a short-lived one-use ticket. */
export async function openConsole(id: string): Promise<WebSocket> {
  const { ticket } = await api.consoleTicket(id);
 const scheme = location.protocol === "https:" ? "wss" : "ws";
  return new WebSocket(
    `${scheme}://${location.host}/api/servers/${id}/ws?ticket=${encodeURIComponent(ticket)}`,
  );
}
