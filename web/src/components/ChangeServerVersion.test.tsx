import { fireEvent, render, screen, waitFor } from "@testing-library/preact";
import { describe, expect, it, vi } from "vitest";
import { I18nProvider } from "../i18n";
import type { Server } from "../types";
import { ChangeServerVersion } from "./ChangeServerVersion";

const api = vi.hoisted(() => ({
  providers: vi.fn().mockResolvedValue([{ id: "paper", server: true }, { id: "fabric", server: true }]),
  versions: vi.fn().mockResolvedValue(["1.21.8", "26.3"]),
  compatibility: vi.fn().mockResolvedValue({ availability: "available", support: "supported", java_minimum: 21 }),
  updateServer: vi.fn().mockResolvedValue({}),
}));
vi.mock("../api", () => ({ api }));

describe("ChangeServerVersion", () => {
  it("saves the selected core and version with recommended Java before closing", async () => {
    const onSaved = vi.fn();
    const onClose = vi.fn();
    render(<I18nProvider><ChangeServerVersion
      server={{ id: "one", core: "paper", version: "1.21.8", java_major: 21, status: "offline" } as Server}
      onSaved={onSaved} onClose={onClose}
    /></I18nProvider>);
    await screen.findByRole("option", { name: "fabric" });
    fireEvent.input(screen.getByLabelText("Server core"), { target: { value: "fabric" } });
    await waitFor(() => expect(api.versions).toHaveBeenCalledWith("fabric"));
    await screen.findByRole("option", { name: "26.3" });
    fireEvent.input(screen.getByLabelText("Minecraft version"), { target: { value: "26.3" } });
    expect(screen.getByLabelText("Java version")).toHaveValue("25");
    const save = screen.getByRole("button", { name: "Save changes" });
    await waitFor(() => expect(save).toBeEnabled());
    fireEvent.click(save);
    await waitFor(() => expect(onClose).toHaveBeenCalledOnce());
    expect(api.updateServer).toHaveBeenCalledExactlyOnceWith("one", { core: "fabric", version: "26.3", java_major: 25 });
    expect(onSaved).toHaveBeenCalledOnce();
  });
});
