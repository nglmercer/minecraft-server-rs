import { DialogProvider } from "../components/Modal";
import { render, screen, waitFor } from "@testing-library/preact";
import { describe, expect, it, vi } from "vitest";
import { Dashboard } from "./Dashboard";
import { I18nProvider } from "../i18n";
import { ToastProvider } from "../components/Toast";
import type { Server } from "../types";

const apiMock = vi.hoisted(() => ({
  servers: vi.fn().mockResolvedValue([]),
  system: vi.fn().mockResolvedValue({
    cpu_percent: 1,
    memory_used_mb: 1,
    memory_total_mb: 2,
    servers_online: 0,
  }),
  power: vi.fn(),
  compatibility: vi.fn().mockResolvedValue({ availability: "available", support: "supported", java_minimum: 21 }),
  providers: vi.fn().mockResolvedValue([{ id: "paper", server: true }]),
  versions: vi.fn().mockResolvedValue(["1.21.8"]),
  createServer: vi.fn().mockResolvedValue({}),
}));

vi.mock("../api", () => ({ api: apiMock }));

function makeServer(overrides: Partial<Server>): Server {
  return {
    id: "server-1",
    name: "Survival",
    core: "paper",
    version: "26.2",
    port: 25565,
    java_major: 21,
    memory: { min_mb: 1024, max_mb: 4096 },
    eula_accepted: true,
    jvm_args: [],
    server_args: [],
    policy: {
      auto_restart: true,
      max_retries: 3,
      retry_delay_secs: 5,
      stop_timeout_secs: 60,
      console_buffer: 500,
    },
    created_at: "2026-01-01T00:00:00Z",
    status: "offline",
    pid: null,
    uptime_secs: null,
    crashes: 0,
    progress: null,
    metrics: null,
    installed: null,
    needs_install: true,
    disk_bytes: 0,
    playit: null,
    pending_restart: false,
    ...overrides,
  };
}

function renderDashboard(servers: Server[]) {
  apiMock.servers.mockResolvedValue(servers);
  localStorage.setItem("mcpanel:guide:v1:dashboard:admin", "seen");
    render(
    <I18nProvider>
      <ToastProvider>
        <DialogProvider>
          <Dashboard user={{ username: "admin", admin: true }} onOpen={vi.fn()} />
        </DialogProvider>
      </ToastProvider>
    </I18nProvider>,
  );
}

describe("Dashboard server uptime", () => {
  it("hides the uptime line while the server is offline", async () => {
    renderDashboard([makeServer({ status: "offline", uptime_secs: null })]);
    await waitFor(() => expect(screen.getByText("Survival")).toBeTruthy());
    expect(screen.queryByText(/^up /)).toBeNull();
  });

  it("shows the uptime line while the server is online", async () => {
    renderDashboard([makeServer({ status: "online", uptime_secs: 3661 })]);
    await waitFor(() => expect(screen.getByText("Survival")).toBeTruthy());
    expect(screen.getByText("up 1h 01m")).toBeTruthy();
  });
});
