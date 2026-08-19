import type {
  Backup,
  ConsoleLine,
  FileEntry,
  Installed,
  JavaInstall,
  PanelUser,
  Project,
  Server,
  SystemStats,
  User,
} from "./types";

const TOKEN_KEY = "mcpanel.token";

/** The bearer token, or null when logged out. */
export function token(): string | null {
  return localStorage.getItem(TOKEN_KEY);
}

function setToken(value: string | null) {
  if (value === null) localStorage.removeItem(TOKEN_KEY);
  else localStorage.setItem(TOKEN_KEY, value);
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
  const auth = token();
  if (auth) headers.set("Authorization", `Bearer ${auth}`);
  if (init.body) headers.set("Content-Type", "application/json");

  const response = await fetch(`/api${path}`, { ...init, headers });

  if (response.status === 401) {
    // The session is gone; drop it so the shell renders the login screen.
    setToken(null);
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

export const api = {
  async login(username: string, password: string): Promise<User> {
    const result = await request<{ token: string; user: User }>("/auth/login", {
      method: "POST",
      body: json({ username, password }),
    });
    setToken(result.token);
    return result.user;
  },

  async logout(): Promise<void> {
    await request("/auth/logout", { method: "POST" }).catch(() => {});
    setToken(null);
  },

  me: () => request<User>("/auth/me"),

  changePassword: (current: string, next: string) =>
    request<{ ok: boolean }>("/auth/password", {
      method: "POST",
      body: json({ current, new: next }),
    }),

  servers: () => request<Server[]>("/servers"),

  server: (id: string) => request<Server>(`/servers/${id}`),

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

  files: (id: string, path = "") =>
    request<FileEntry[]>(`/servers/${id}/files?path=${encodeURIComponent(path)}`),

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

  downloadUrl(id: string, path: string): string {
    // The browser follows this itself, so the token rides in the query string.
    const auth = encodeURIComponent(token() ?? "");
    return `/api/servers/${id}/files/download?path=${encodeURIComponent(path)}&token=${auth}`;
  },

  async upload(id: string, path: string, files: File[]): Promise<void> {
    const body = new FormData();
    for (const file of files) body.append("file", file, file.name);

    const headers = new Headers();
    const auth = token();
    if (auth) headers.set("Authorization", `Bearer ${auth}`);

    const response = await fetch(
      `/api/servers/${id}/files/upload?path=${encodeURIComponent(path)}`,
      { method: "POST", body, headers },
    );
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

  restoreBackup: (id: string, backup: string) =>
    request<{ ok: boolean }>(`/servers/${id}/backups/${backup}/restore`, { method: "POST" }),

  deleteBackup: (id: string, backup: string) =>
    request<{ ok: boolean }>(`/servers/${id}/backups/${backup}`, { method: "DELETE" }),

  backupUrl(id: string, backup: string): string {
    const auth = encodeURIComponent(token() ?? "");
    return `/api/servers/${id}/backups/${backup}/download?token=${auth}`;
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
};

/**
 * Open the console socket for `id`.
 *
 * The token goes in the query string because browsers cannot set headers on a
 * WebSocket handshake.
 */
export function openConsole(id: string): WebSocket {
  const scheme = location.protocol === "https:" ? "wss" : "ws";
  const auth = encodeURIComponent(token() ?? "");
  return new WebSocket(`${scheme}://${location.host}/api/servers/${id}/ws?token=${auth}`);
}
