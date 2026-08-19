import { fireEvent, render, screen } from "@testing-library/preact";
import { describe, expect, it, vi } from "vitest";
import { MenuProvider, useMenu } from "./Menu";
import { I18nProvider } from "../i18n";

/** A row that opens a menu the way a file row does. */
function Row({ onDelete }: { onDelete: () => void }) {
  const menu = useMenu();
  const items = [
    { label: "Rename", onSelect: () => {} },
    { label: "Delete", danger: true, onSelect: onDelete },
    { label: "Restore", onSelect: () => {}, disabled: true },
  ];

  return (
    <tr
      data-testid="row"
      onContextMenu={(event) => menu.open(event as unknown as MouseEvent, items, "world")}
    >
      <td>
        <button onClick={(event) => menu.open(event as unknown as MouseEvent, items, "world")}>
          open
        </button>
      </td>
    </tr>
  );
}

function setup(onDelete = vi.fn()) {
  render(
    <I18nProvider>
      <MenuProvider>
        <table>
          <tbody>
            <Row onDelete={onDelete} />
          </tbody>
        </table>
      </MenuProvider>
    </I18nProvider>,
  );
  return onDelete;
}

describe("contextual menu", () => {
  it("opens on a right-click, which is also what a long-press fires", () => {
    setup();
    expect(screen.queryByRole("menu")).toBeNull();

    fireEvent.contextMenu(screen.getByTestId("row"));

    expect(screen.getByRole("menu")).toBeInTheDocument();
    expect(screen.getByText("Rename")).toBeInTheDocument();
  });

  it("suppresses the browser's own menu", () => {
    setup();

    const event = new MouseEvent("contextmenu", { bubbles: true, cancelable: true });
    fireEvent(screen.getByTestId("row"), event);

    // Without this the native menu appears instead, which is what happened on
    // a phone before the menu existed.
    expect(event.defaultPrevented).toBe(true);
  });

  it("opens from the explicit trigger, because a long-press discovers nothing", () => {
    setup();
    fireEvent.click(screen.getByText("open"));

    expect(screen.getByRole("menu")).toBeInTheDocument();
  });

  it("runs the chosen action and closes", () => {
    const onDelete = setup();
    fireEvent.contextMenu(screen.getByTestId("row"));

    fireEvent.click(screen.getByText("Delete"));

    expect(onDelete).toHaveBeenCalledOnce();
    expect(screen.queryByRole("menu")).toBeNull();
  });

  it("ignores a disabled item", () => {
    setup();
    fireEvent.contextMenu(screen.getByTestId("row"));

    const restore = screen.getByText("Restore") as HTMLButtonElement;
    expect(restore.disabled).toBe(true);

    fireEvent.click(restore);
    // Still open: a disabled item must not act, and must not dismiss either.
    expect(screen.getByRole("menu")).toBeInTheDocument();
  });

  it("closes on Escape", () => {
    setup();
    fireEvent.contextMenu(screen.getByTestId("row"));

    fireEvent.keyDown(document, { key: "Escape" });

    expect(screen.queryByRole("menu")).toBeNull();
  });

  it("names the row it belongs to", () => {
    setup();
    fireEvent.contextMenu(screen.getByTestId("row"));

    // On a bottom sheet the menu covers the row, so it has to say what it acts on.
    expect(screen.getByRole("menu")).toHaveAttribute("aria-label", "world");
  });
});
