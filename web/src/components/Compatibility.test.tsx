import { fireEvent, render, screen, waitFor } from "@testing-library/preact";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { I18nProvider } from "../i18n";
import { Compatibility } from "./Compatibility";
import type { CompatibilityView } from "../types";
const api = vi.hoisted(() => ({ compatibility: vi.fn() }));
vi.mock("../api", () => ({ api }));
const archived: CompatibilityView = {
  availability: "available",
  support: "unsupported",
  java_minimum: 21,
};
beforeEach(() => {
  api.compatibility.mockResolvedValue(archived);
});
describe("version checks", () => {
  it("distinguishes downloadable archived Paper and requires acknowledgement", async () => {
    const allowed = vi.fn();
    render(
      <I18nProvider>
        <Compatibility
          core="paper"
          version="1.21.11"
          javaMajor={21}
          onAllowedChange={allowed}
        />
      </I18nProvider>,
    );
    expect(
      await screen.findByText("Upstream support: ended."),
    ).toBeInTheDocument();
    expect(
      screen.getByText(/A downloadable build is available/),
    ).toBeInTheDocument();
    expect(allowed).toHaveBeenLastCalledWith(false);
    fireEvent.click(screen.getByRole("checkbox"));
    await waitFor(() => expect(allowed).toHaveBeenLastCalledWith(true));
  });
  it("blocks a Java mismatch even after acknowledgement", async () => {
    const allowed = vi.fn();
    render(
      <I18nProvider>
        <Compatibility
          core="paper"
          version="1.21.11"
          javaMajor={17}
          onAllowedChange={allowed}
        />
      </I18nProvider>,
    );
    expect(await screen.findByText(/Selected Java 17/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("checkbox"));
    expect(allowed).toHaveBeenLastCalledWith(false);
  });
  it("reports network errors as unknown and permits retry", async () => {
    api.compatibility.mockRejectedValueOnce(new Error("offline"));
    render(
      <I18nProvider>
        <Compatibility core="paper" version="1.21.11" javaMajor={21} />
      </I18nProvider>,
    );
    expect(
      await screen.findByText(/Build availability could not be verified/),
    ).toBeInTheDocument();
    expect(
      screen.queryByText("Upstream support: ended."),
    ).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Check again" }));
    expect(
      await screen.findByText("Upstream support: ended."),
    ).toBeInTheDocument();
  });
  it("ignores stale responses after changing versions", async () => {
    let resolve!: (value: CompatibilityView) => void;
    api.compatibility.mockImplementationOnce(
      () =>
        new Promise<CompatibilityView>((r) => {
          resolve = r;
        }),
    );
    api.compatibility.mockResolvedValueOnce({
      ...archived,
      availability: "unavailable",
    });
    const allowed = vi.fn();
    const view = (version: string) => (
      <I18nProvider>
        <Compatibility
          core="paper"
          version={version}
          javaMajor={21}
          onAllowedChange={allowed}
        />
      </I18nProvider>
    );
    const { rerender } = render(view("old"));
    await waitFor(() =>
      expect(api.compatibility).toHaveBeenCalledWith("paper", "old"),
    );
    rerender(view("new"));
    expect(
      await screen.findByText(/No downloadable build was found/),
    ).toBeInTheDocument();
    resolve(archived);
    await waitFor(() => expect(allowed).toHaveBeenLastCalledWith(false));
    expect(
      screen.queryByText(/A downloadable build is available/),
    ).not.toBeInTheDocument();
  });
});
