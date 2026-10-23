import { render, screen, waitFor } from "@testing-library/preact";
import { describe, expect, it, vi } from "vitest";
import { I18nProvider } from "../i18n";
import { MenuProvider } from "../components/Menu";
import { ToastProvider } from "../components/Toast";
import { ServerDetail } from "./ServerDetail";
import type { Server } from "../types";

const apiMock = vi.hoisted(() => ({
  server: vi.fn(),
  serverPlayit: vi.fn(),
  openConsole: vi.fn(),
}));

vi.mock("../api", () => ({
  api: apiMock,
  openConsole: apiMock.openConsole,
}));

function makeServer(metrics: Server["metrics"]): Server {
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
    metrics,
    installed: null,
    needs_install: false,
    disk_bytes: 233 * 1024 * 1024,
    playit: null,
    pending_restart: false,
  };
}

function renderDetail(server: Server) {
  apiMock.server.mockResolvedValue(server);
  apiMock.serverPlayit.mockResolvedValue(null);
  apiMock.openConsole.mockRejectedValue(new Error("not connected in this test"));

  render(
    <I18nProvider>
      <ToastProvider>
        <MenuProvider>
          <ServerDetail id="server-1" user={{ username: "admin", admin: true }} onBack={vi.fn()} />
        </MenuProvider>
      </ToastProvider>
    </I18nProvider>,
  );
}

describe("server process metric semantics", () => {
  it("renders unknown process metrics instead of fake zeroes", async () => {
    renderDetail(makeServer(null));

    const cpuLabel = await screen.findByText("CPU usage");
    const memoryLabel = screen.getByText("Process memory (RSS)");
    const cpuCard = cpuLabel.closest("section");
    const memoryCard = memoryLabel.closest("section");

    expect(cpuCard).toHaveTextContent("—");
    expect(cpuCard).toHaveTextContent("Not running");
    expect(cpuCard).not.toHaveTextContent("0.00%");
    expect(memoryCard).toHaveTextContent("—");
    expect(memoryCard).toHaveTextContent("Not running");
    expect(memoryCard).not.toHaveTextContent("0 MiB");
  });

  it("keeps RSS visible when Android restricts CPU counters", async () => {
    renderDetail(makeServer({
      cpu_percent: null,
      cpu_cores: null,
      cpu_host_percent: null,
      logical_cpu_count: 8,
      memory_mb: 512,
    }));
    const cpuLabel = await screen.findByText("CPU usage");
    expect(cpuLabel.closest("section")).toHaveTextContent("—");
    expect(cpuLabel.closest("section")).not.toHaveTextContent("0.00%");
    expect(screen.getByText("Process memory (RSS)").closest("section")).toHaveTextContent("512 MiB");
  });

  it("normalizes CPU to host capacity and keeps RSS separate from heap max", async () => {
    renderDetail(
      makeServer({
        cpu_percent: 278.98,
        cpu_cores: 2.7898,
        cpu_host_percent: 34.8725,
        logical_cpu_count: 8,
        memory_mb: 5000,
      }),
    );

    await waitFor(() => expect(screen.getByText("CPU usage")).toBeInTheDocument());
    const cpuCard = screen.getByText("CPU usage").closest("section");
    const memoryCard = screen.getByText("Process memory (RSS)").closest("section");

    expect(cpuCard).toHaveTextContent(/34\.87%\/ 100%/);
    expect(cpuCard).toHaveTextContent("2.79 logical cores");
    expect(cpuCard).not.toHaveTextContent("278.98% / 100%");
    expect(memoryCard).toHaveTextContent("5000 MiB");
    expect(memoryCard).toHaveTextContent("Heap max: 4096 MiB");
    expect(memoryCard).not.toHaveTextContent("/ 4096");
  });
});
