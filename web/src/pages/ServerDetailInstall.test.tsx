import { fireEvent, render, screen, waitFor } from "@testing-library/preact";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { I18nProvider } from "../i18n";
import { MenuProvider } from "../components/Menu";
import { DialogProvider } from "../components/Modal";
import { ToastProvider } from "../components/Toast";
import { ServerDetail } from "./ServerDetail";
import type { Server } from "../types";

const apiMock = vi.hoisted(() => ({
  server: vi.fn(),
  compatibility: vi.fn().mockResolvedValue({ availability: "available", support: "supported", java_minimum: 21 }),
  serverPlayit: vi.fn(),
  openConsole: vi.fn(),
  reinstall: vi.fn(),
  prepare: vi.fn(),
}));

vi.mock("../api", () => ({
  api: apiMock,
  openConsole: apiMock.openConsole,
}));

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

async function openSettings(server: Server) {
  apiMock.server.mockResolvedValue(server);
  apiMock.serverPlayit.mockResolvedValue(null);
  apiMock.openConsole.mockRejectedValue(new Error("not connected in this test"));

  render(
    <I18nProvider>
      <ToastProvider>
        <MenuProvider>
          <DialogProvider>
            <ServerDetail id="server-1" user={{ username: "admin", admin: true }} onBack={vi.fn()} />
          </DialogProvider>
        </MenuProvider>
      </ToastProvider>
    </I18nProvider>,
  );

  await waitFor(() => expect(screen.getByText("Survival")).toBeTruthy());
  fireEvent.click(screen.getByRole("button", { name: "Settings" }));
  await waitFor(() => expect(screen.getByText("Installed build")).toBeTruthy());
}

describe("ServerDetail installed build", () => {
  beforeEach(() => { apiMock.compatibility.mockResolvedValue({ availability: "available", support: "supported", java_minimum: 21 }); });
  it("renders exactly one Install button for a fresh server", async () => {
    await openSettings(makeServer({ installed: null, needs_install: true }));
    expect(screen.getAllByRole("button", { name: "Install" })).toHaveLength(1);
  });

  it("renders no Install button once the install matches the config", async () => {
    await openSettings(
      makeServer({
        installed: {
          core: "paper",
          version: "26.2",
          build: "600",
          java_major: 21,
          java: "/jdks/21/bin/java",
          jar: "/servers/one/server.jar",
          installed_at: "2026-01-01T00:00:00Z",
        },
        needs_install: false,
      }),
    );
    expect(screen.queryByRole("button", { name: "Install" })).toBeNull();
  });

  it("hides the header uptime chip while the server is offline", async () => {
    await openSettings(makeServer({ status: "offline", uptime_secs: null }));
    expect(screen.queryByRole("tooltip", { name: "Uptime" })).toBeNull();
  });

  it("shows the header uptime chip while the server is online", async () => {
    await openSettings(makeServer({ status: "online", uptime_secs: 90 }));
    expect(screen.getByRole("tooltip", { name: "Uptime" })).toBeTruthy();
    expect(screen.getByText("1m 30s")).toBeTruthy();
  });
});
