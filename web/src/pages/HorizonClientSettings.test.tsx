import { fireEvent, render, screen, waitFor } from "@testing-library/preact";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ToastProvider } from "../components/Toast";
import { I18nProvider } from "../i18n";
import { HorizonClientSettings } from "./HorizonClientSettings";

const apiMock = vi.hoisted(() => ({
  horizonSettings: vi.fn(),
  updateHorizonSettings: vi.fn(),
}));

vi.mock("../api", () => ({ api: apiMock }));

describe("HorizonClientSettings", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    apiMock.horizonSettings.mockResolvedValue({
      online_mode: true,
      accept_older_clients: false,
      enforce_secure_profile: true,
    });
    apiMock.updateHorizonSettings.mockImplementation(async (_id, body) => body);
  });

  it("loads the server's client settings and patches the changed keys", async () => {
    render(
      <I18nProvider>
        <ToastProvider>
          <HorizonClientSettings serverId="one" />
        </ToastProvider>
      </I18nProvider>,
    );

    const older = await screen.findByLabelText(/Accept 26.2 clients/i);
    await waitFor(() => expect((older as HTMLInputElement).disabled).toBe(false));
    fireEvent.click(older);
    fireEvent.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() =>
      expect(apiMock.updateHorizonSettings).toHaveBeenCalledWith("one", {
        online_mode: true,
        accept_older_clients: true,
        enforce_secure_profile: true,
      }),
    );
  });
});
