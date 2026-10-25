import { render, screen } from "@testing-library/preact";
import { describe, expect, it, vi } from "vitest";
import { I18nProvider } from "../i18n";
import { DialogProvider } from "../components/Modal";
import { ToastProvider } from "../components/Toast";
import { PlayitSettings } from "./ServerSettings";
import type { Server, ServerPlayitView, User } from "../types";

const apiMock = vi.hoisted(() => ({
  attachPlayit: vi.fn(),
  detachPlayit: vi.fn(),
  reconcilePlayit: vi.fn(),
  forgetPlayit: vi.fn(),
}));

vi.mock("../api", () => ({ api: apiMock }));

const server = {
  id: "server-1",
  name: "Survival",
} as Server;

const admin = { username: "admin", admin: true, servers: [] } as User;

describe("server Playit recovery controls", () => {
  it("offers reconciliation and forgetting for an account mismatch", () => {
    const playit = {
      state: "account_mismatch",
      binding: {
        tunnel_id: "tunnel-1",
        protocol: "tcp",
        local_address: "127.0.0.1",
        local_port: 25565,
        agent_id: "old-agent",
        created_at: 1,
      },
      tunnel: null,
      message: "The stored Playit association belongs to another agent",
      cleanup_pending: false,
    } as ServerPlayitView;

    render(
      <I18nProvider>
        <ToastProvider>
          <DialogProvider>
            <PlayitSettings server={server} playit={playit} user={admin} onChanged={vi.fn()} />
          </DialogProvider>
        </ToastProvider>
      </I18nProvider>,
    );

    expect(screen.getByRole("button", { name: "Reconcile" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Forget association" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Disconnect server" })).toBeInTheDocument();
  });
});
