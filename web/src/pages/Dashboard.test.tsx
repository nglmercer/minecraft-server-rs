import { fireEvent, render, screen, waitFor } from "@testing-library/preact";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { Dashboard } from "./Dashboard";
import { I18nProvider } from "../i18n";
import { ToastProvider } from "../components/Toast";

const apiMock = vi.hoisted(() => ({
  servers: vi.fn().mockResolvedValue([]),
  system: vi.fn().mockResolvedValue({
    cpu_percent: 1,
    memory_used_mb: 1,
    memory_total_mb: 2,
    servers_online: 0,
  }),
  power: vi.fn(),
  providers: vi.fn().mockResolvedValue([{ id: "paper", server: true }]),
  versions: vi.fn().mockResolvedValue(["1.21.8"]),
  createServer: vi.fn().mockResolvedValue({}),
}));

vi.mock("../api", () => ({ api: apiMock }));

describe("Dashboard create server form", () => {
  beforeEach(() => {
    apiMock.servers.mockResolvedValue([]);
    apiMock.system.mockResolvedValue({
      cpu_percent: 1,
      memory_used_mb: 1,
      memory_total_mb: 2,
      servers_online: 0,
    });
    apiMock.providers.mockResolvedValue([{ id: "paper", server: true }]);
    apiMock.versions.mockResolvedValue(["1.21.8"]);
    apiMock.createServer.mockResolvedValue({});
  });

  it("shows unknown host CPU when Android denies CPU statistics", async () => {
    apiMock.system.mockResolvedValue({
      cpu_percent: null,
      memory_used_mb: 1,
      memory_total_mb: 2,
      servers_online: 0,
    });
    render(
      <I18nProvider>
        <ToastProvider>
          <Dashboard user={{ username: "admin", admin: true }} onOpen={vi.fn()} />
        </ToastProvider>
      </I18nProvider>,
    );
    expect(await screen.findByText("Unknown")).toBeInTheDocument();
    expect(screen.queryByText("0%")).not.toBeInTheDocument();
  });

  it("sends exactly one create request for one submit button click", async () => {
    render(
      <I18nProvider>
        <ToastProvider>
          <Dashboard user={{ username: "admin", admin: true }} onOpen={vi.fn()} />
        </ToastProvider>
      </I18nProvider>,
    );

    fireEvent.click(screen.getByRole("button", { name: "New server" }));
    await waitFor(() => expect(apiMock.versions).toHaveBeenCalled());
    const submit = await screen.findByRole("button", { name: "Create server" });
    await waitFor(() => expect(submit).not.toBeDisabled());

    fireEvent.click(submit);

    await waitFor(() => expect(apiMock.createServer).toHaveBeenCalledTimes(1));
  });

  it("keeps focus on the control selected after the modal rerenders", async () => {
    render(
      <I18nProvider>
        <ToastProvider>
          <Dashboard user={{ username: "admin", admin: true }} onOpen={vi.fn()} />
        </ToastProvider>
      </I18nProvider>,
    );

    fireEvent.click(screen.getByRole("button", { name: "New server" }));
    const port = await screen.findByDisplayValue("25565");

    port.focus();
    expect(document.activeElement).toBe(port);

    fireEvent.input(port, { target: { value: "25566" } });
    await waitFor(() => expect(document.activeElement).toBe(port));
  });
});
