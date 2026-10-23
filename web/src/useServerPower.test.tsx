import { fireEvent, render, screen, waitFor } from "@testing-library/preact";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { I18nProvider } from "./i18n";
import { DialogProvider } from "./components/Modal";
import { useServerPower } from "./useServerPower";
import type { Server } from "./types";
const api = vi.hoisted(() => ({
  server: vi.fn(),
  updateServer: vi.fn(),
  power: vi.fn(),
}));
vi.mock("./api", () => ({ api }));
const server = { id: "one", eula_accepted: false } as Server;
function Harness() {
  const power = useServerPower();
  return (
    <button
      onClick={() => {
        void power(server, "start").catch(() => {});
      }}
    >
      Start test
    </button>
  );
}
function mount() {
  render(
    <I18nProvider>
      <DialogProvider>
        <Harness />
      </DialogProvider>
    </I18nProvider>,
  );
}
beforeEach(() => {
  api.server.mockResolvedValue(server);
  api.updateServer.mockResolvedValue({ ...server, eula_accepted: true });
  api.power.mockResolvedValue({ ...server, status: "starting" });
  api.power.mockClear();
  api.updateServer.mockClear();
});
describe("EULA launch recovery", () => {
  it("keeps the server stopped when dismissed", async () => {
    mount();
    fireEvent.click(screen.getByText("Start test"));
    await screen.findByRole("dialog");
    expect(screen.getByRole("link")).toHaveAttribute(
      "href",
      "https://www.minecraft.net/eula",
    );
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    await waitFor(() =>
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument(),
    );
    expect(api.updateServer).not.toHaveBeenCalled();
    expect(api.power).not.toHaveBeenCalled();
  });
  it("persists acceptance before starting and suppresses double clicks", async () => {
    mount();
    const start = screen.getByText("Start test");
    fireEvent.click(start);
    fireEvent.click(start);
    fireEvent.click(
      await screen.findByRole("button", { name: "Accept EULA and start" }),
    );
    await waitFor(() => expect(api.power).toHaveBeenCalledTimes(1));
    expect(api.updateServer).toHaveBeenCalledExactlyOnceWith("one", {
      eula_accepted: true,
    });
    expect(api.updateServer.mock.invocationCallOrder[0]).toBeLessThan(
      api.power.mock.invocationCallOrder[0],
    );
  });
  it("does not launch when saving acceptance fails", async () => {
    api.updateServer.mockRejectedValueOnce(new Error("write failed"));
    mount();
    fireEvent.click(screen.getByText("Start test"));
    fireEvent.click(
      await screen.findByRole("button", { name: "Accept EULA and start" }),
    );
    await waitFor(() => expect(api.updateServer).toHaveBeenCalled());
    expect(api.power).not.toHaveBeenCalled();
  });
  it("uses fresh backend acceptance instead of a stale card", async () => {
    api.server.mockResolvedValue({ ...server, eula_accepted: true });
    mount();
    fireEvent.click(screen.getByText("Start test"));
    await waitFor(() => expect(api.power).toHaveBeenCalledWith("one", "start"));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(api.updateServer).not.toHaveBeenCalled();
  });
});
