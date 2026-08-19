import type { ComponentChildren } from "preact";
import { createContext } from "preact";
import { useCallback, useContext, useEffect, useMemo, useRef, useState } from "preact/hooks";

/** Below this width the menu becomes a bottom sheet, which is far easier to hit. */
const SHEET_BREAKPOINT = 640;

/** Estimated menu box, used to keep a popup inside the viewport. */
const MENU_WIDTH = 200;
const ITEM_HEIGHT = 40;

/** One entry in a contextual menu. */
export interface MenuItem {
  label: string;
  /** Run when chosen. Omit when `href` is set. */
  onSelect?: () => void;
  /** Rendered as a link instead of a button — needed for browser-driven downloads. */
  href?: string;
  /** Styled as destructive. */
  danger?: boolean;
  /** Greyed out and unselectable. */
  disabled?: boolean;
}

interface OpenMenu {
  items: MenuItem[];
  x: number;
  y: number;
  /** Heading shown on the bottom sheet, where context is otherwise lost. */
  title?: string;
}

interface Menus {
  /**
   * Open a menu at the event's position.
   *
   * Call this from `onContextMenu` — which fires for a desktop right-click and
   * for a touch long-press — and from an explicit trigger button, because a
   * long-press is undiscoverable on its own.
   */
  open: (event: MouseEvent, items: MenuItem[], title?: string) => void;
  close: () => void;
}

const MenuContext = createContext<Menus | null>(null);

export function MenuProvider({ children }: { children: ComponentChildren }) {
  const [menu, setMenu] = useState<OpenMenu | null>(null);
  const panel = useRef<HTMLDivElement | null>(null);

  const close = useCallback(() => setMenu(null), []);

  const value = useMemo<Menus>(
    () => ({
      open: (event, items, title) => {
        // Suppress the browser's own menu; on touch this is the long-press one.
        event.preventDefault();
        event.stopPropagation();
        setMenu({ items, x: event.clientX, y: event.clientY, title });
      },
      close,
    }),
    [close],
  );

  useEffect(() => {
    if (!menu) return;

    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") close();
    };
    // Scrolling would leave a positioned popup pointing at the wrong row.
    const onScroll = () => close();

    document.addEventListener("keydown", onKeyDown);
    window.addEventListener("scroll", onScroll, true);
    window.addEventListener("resize", onScroll);

    return () => {
      document.removeEventListener("keydown", onKeyDown);
      window.removeEventListener("scroll", onScroll, true);
      window.removeEventListener("resize", onScroll);
    };
  }, [menu, close]);

  const sheet = typeof window !== "undefined" && window.innerWidth < SHEET_BREAKPOINT;

  function choose(item: MenuItem) {
    if (item.disabled) return;
    close();
    item.onSelect?.();
  }

  const itemClass = (item: MenuItem) =>
    [
      "flex w-full items-center gap-3 px-4 text-left transition-colors",
      sheet ? "py-3.5 text-base" : "py-2 text-sm",
      item.disabled
        ? "cursor-not-allowed text-fg-muted/50"
        : item.danger
          ? "text-red-400 hover:bg-red-500/10"
          : "text-fg hover:bg-ink-700",
    ].join(" ");

  return (
    <MenuContext.Provider value={value}>
      {children}

      {menu && (
        <div
          class={`fixed inset-0 z-[55] ${sheet ? "flex items-end bg-black/50" : ""}`}
          onClick={close}
          onContextMenu={(event) => {
            // A second long-press should dismiss, not stack another menu.
            event.preventDefault();
            close();
          }}
          role="presentation"
        >
          <div
            ref={panel}
            role="menu"
            aria-label={menu.title}
            onClick={(event) => event.stopPropagation()}
            class={
              sheet
                ? "w-full animate-[fade-in_150ms_ease-out] rounded-t-2xl border-t border-ink-700 bg-ink-850 pb-[env(safe-area-inset-bottom)] shadow-2xl"
                : "absolute w-[200px] overflow-hidden rounded-xl border border-ink-700 bg-ink-850 py-1 shadow-2xl"
            }
            style={
              sheet
                ? undefined
                : {
                    // Flipped rather than clipped when the pointer is near an edge.
                    left: Math.min(menu.x, window.innerWidth - MENU_WIDTH - 8),
                    top: Math.min(
                      menu.y,
                      window.innerHeight - menu.items.length * ITEM_HEIGHT - 16,
                    ),
                  }
            }
          >
            {menu.title && (
              <p
                class={`truncate border-b border-ink-700 px-4 font-mono text-xs text-fg-muted ${
                  sheet ? "py-3" : "py-2"
                }`}
              >
                {menu.title}
              </p>
            )}

            {menu.items.map((item, index) =>
              item.href ? (
                <a
                  key={`${item.label}-${index}`}
                  href={item.href}
                  role="menuitem"
                  onClick={close}
                  class={itemClass(item)}
                >
                  {item.label}
                </a>
              ) : (
                <button
                  key={`${item.label}-${index}`}
                  role="menuitem"
                  disabled={item.disabled}
                  onClick={() => choose(item)}
                  class={itemClass(item)}
                >
                  {item.label}
                </button>
              ),
            )}
          </div>
        </div>
      )}
    </MenuContext.Provider>
  );
}

/** Open a contextual menu from a right-click, a long-press, or a trigger button. */
export function useMenu(): Menus {
  const context = useContext(MenuContext);
  if (!context) throw new Error("useMenu must be used inside a MenuProvider");
  return context;
}

/** The always-visible "⋯" affordance, since a long-press discovers nothing. */
export function MenuButton({
  onOpen,
  label,
}: {
  onOpen: (event: MouseEvent) => void;
  label: string;
}) {
  return (
    <button
      aria-label={label}
      onClick={(event) => onOpen(event as unknown as MouseEvent)}
      // Comfortably tappable: below ~44px touch targets start getting missed.
      class="grid size-9 shrink-0 place-items-center rounded-lg text-fg-muted transition-colors hover:bg-ink-700 hover:text-fg"
    >
      <svg viewBox="0 0 20 20" class="size-5" fill="currentColor" aria-hidden="true">
        <circle cx="10" cy="4" r="1.6" />
        <circle cx="10" cy="10" r="1.6" />
        <circle cx="10" cy="16" r="1.6" />
      </svg>
    </button>
  );
}
