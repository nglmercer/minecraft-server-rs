import { fireEvent, render, screen, waitFor } from "@testing-library/preact";
import { describe, expect, it, vi } from "vitest";
import { I18nProvider } from "../i18n";
import { Guide } from "./Guide";
describe("guided help", () => {
  it("offers first-run help, remembers dismissal and can be replayed", () => {
    render(
      <I18nProvider>
        <Guide
          storageKey="test-user"
          steps={[{ title: "First", body: "Details" }]}
        />
      </I18nProvider>,
    );
    expect(screen.getByRole("dialog", { name: "First" })).toBeInTheDocument();
    fireEvent.keyDown(screen.getByRole("dialog"), { key: "Escape" });
    expect(localStorage.getItem("mcpanel:guide:v1:test-user")).toBe("seen");
    fireEvent.click(screen.getByRole("button", { name: "Help · guided tour" }));
    expect(screen.getByRole("dialog")).toBeInTheDocument();
  });
  it("switches the actual server section when advancing", async () => {
    const enter = vi.fn();
    render(
      <I18nProvider>
        <Guide
          steps={[
            { title: "First", body: "One" },
            { title: "Second", body: "Two", enter },
          ]}
        />
      </I18nProvider>,
    );
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Help · guided tour" }));
    fireEvent.click(screen.getByRole("button", { name: "Next" }));
    expect(screen.getByRole("dialog", { name: "Second" })).toBeInTheDocument();
    await waitFor(() => expect(enter).toHaveBeenCalledTimes(1));
  });
});
