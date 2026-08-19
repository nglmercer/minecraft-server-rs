import { fireEvent, render, screen, waitFor } from "@testing-library/preact";
import { describe, expect, it, vi } from "vitest";
import { DialogProvider, useDialogs } from "./Modal";
import { I18nProvider } from "../i18n";

function Harness({ onAnswer }: { onAnswer: (value: unknown) => void }) {
  const dialogs = useDialogs();

  return (
    <>
      <button
        onClick={() =>
          dialogs
            .confirm({ title: "Delete world?", body: "Cannot be undone", danger: true })
            .then(onAnswer)
        }
      >
        ask
      </button>
      <button
        onClick={() =>
          dialogs.prompt({ title: "Rename", label: "New name", initial: "old" }).then(onAnswer)
        }
      >
        rename
      </button>
    </>
  );
}

function setup() {
  const onAnswer = vi.fn();
  render(
    <I18nProvider>
      <DialogProvider>
        <Harness onAnswer={onAnswer} />
      </DialogProvider>
    </I18nProvider>,
  );
  return onAnswer;
}

describe("confirm", () => {
  it("resolves true when accepted", async () => {
    const onAnswer = setup();
    fireEvent.click(screen.getByText("ask"));

    expect(screen.getByRole("dialog")).toBeInTheDocument();
    fireEvent.click(screen.getByText("Confirm"));

    await waitFor(() => expect(onAnswer).toHaveBeenCalledWith(true));
  });

  it("resolves false when dismissed, rather than hanging", async () => {
    const onAnswer = setup();
    fireEvent.click(screen.getByText("ask"));
    fireEvent.click(screen.getByText("Cancel"));

    // A dialog that never settles leaves the caller's `await` stuck forever.
    await waitFor(() => expect(onAnswer).toHaveBeenCalledWith(false));
  });

  it("dismisses on Escape", async () => {
    const onAnswer = setup();
    fireEvent.click(screen.getByText("ask"));
    fireEvent.keyDown(document, { key: "Escape" });

    await waitFor(() => expect(onAnswer).toHaveBeenCalledWith(false));
  });
});

describe("prompt", () => {
  it("resolves the entered text", async () => {
    const onAnswer = setup();
    fireEvent.click(screen.getByText("rename"));

    const input = screen.getByDisplayValue("old");
    fireEvent.input(input, { target: { value: "new-name" } });
    fireEvent.click(screen.getByText("Confirm"));

    await waitFor(() => expect(onAnswer).toHaveBeenCalledWith("new-name"));
  });

  it("treats an emptied field as a dismissal", async () => {
    const onAnswer = setup();
    fireEvent.click(screen.getByText("rename"));

    fireEvent.input(screen.getByDisplayValue("old"), { target: { value: "   " } });
    fireEvent.click(screen.getByText("Confirm"));

    // Otherwise a stray Enter renames a file to the empty string.
    await waitFor(() => expect(onAnswer).toHaveBeenCalledWith(null));
  });

  it("resolves null when cancelled", async () => {
    const onAnswer = setup();
    fireEvent.click(screen.getByText("rename"));
    fireEvent.click(screen.getByText("Cancel"));

    await waitFor(() => expect(onAnswer).toHaveBeenCalledWith(null));
  });
});
