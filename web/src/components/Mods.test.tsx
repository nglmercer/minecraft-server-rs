import { fireEvent, render, screen, waitFor } from "@testing-library/preact";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { I18nProvider } from "../i18n";
import { MenuProvider } from "./Menu";
import { DialogProvider } from "./Modal";
import { ToastProvider } from "./Toast";
import { Mods } from "./Mods";
import type { Server } from "../types";
const api = vi.hoisted(() => ({
  installedMods: vi.fn(),
  searchMods: vi.fn(),
  installMod: vi.fn(),
  deleteFile: vi.fn(),
}));
vi.mock("../api", () => ({ api }));
beforeEach(() => {
  api.installedMods.mockResolvedValue([
    { filename: "Example.jar", path: "plugins/Example.jar", size: 100 },
  ]);
  api.searchMods.mockResolvedValue([]);
  api.searchMods.mockClear();
});
function mount() {
  render(
    <I18nProvider>
      <ToastProvider>
        <MenuProvider>
          <DialogProvider>
            <Mods
              server={
                {
                  id: "one",
                  core: "paper",
                  version: "1.21.11",
                  status: "offline",
                } as Server
              }
            />
          </DialogProvider>
        </MenuProvider>
      </ToastProvider>
    </I18nProvider>,
  );
}
describe("add-on sections", () => {
  it("opens the installed inventory without fetching the marketplace", async () => {
    mount();
    expect(await screen.findByText("Example.jar")).toBeInTheDocument();
    expect(screen.queryByPlaceholderText(/Search/)).not.toBeInTheDocument();
    expect(api.searchMods).not.toHaveBeenCalled();
    fireEvent.click(
      screen.getByRole("button", { name: "Browse plugins / mods" }),
    );
    await waitFor(() => expect(api.searchMods).toHaveBeenCalledWith("one", ""));
    expect(screen.queryByText("Example.jar")).not.toBeInTheDocument();
  });
  it("distinguishes a failed inventory load from no installed plugins", async () => {
    api.installedMods.mockRejectedValueOnce(new Error("offline"));
    mount();
    expect(
      await screen.findByText(/Could not load installed add-ons/),
    ).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Check again" }));
    expect(await screen.findByText("Example.jar")).toBeInTheDocument();
  });
});
