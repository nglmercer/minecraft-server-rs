import type { ComponentChildren } from "preact";
import { createContext } from "preact";
import { createPortal } from "preact/compat";
import { useCallback, useContext, useEffect, useMemo, useRef, useState } from "preact/hooks";
import { useT } from "../i18n";
import { Button, Input } from "./ui";
import * as Icon from "./icons";

/**
 * A modal dialog rendered at document body via a portal.
 *
 * Native `confirm`/`prompt` block the event loop, cannot be styled, cannot be
 * translated, and in the browser-automation guidance are outright hazardous —
 * so the panel does not use them anywhere.
 */
export function Modal({
  title,
  onClose,
  children,
  footer,
  width = "md",
}: {
  title: string;
  onClose: () => void;
  children: ComponentChildren;
  footer?: ComponentChildren;
  width?: "sm" | "md" | "lg" | "xl";
}) {
  const t = useT();
  const panel = useRef<HTMLDivElement | null>(null);
  const onCloseRef = useRef(onClose);
  const openerRef = useRef<HTMLElement | null>(null);

  // Keep Escape wired to the current callback without making the open/close
  // lifecycle effect rerun when a parent creates a new function identity.
  onCloseRef.current = onClose;

  useEffect(() => {
    const focusInside = (preferred: "first" | "last" = "first") => {
      const currentPanel = panel.current;
      if (!currentPanel) return;
      const focusable = getFocusableElements(currentPanel);
      const target = preferred === "last"
        ? focusable[focusable.length - 1]
        : focusable[0];
      (target ?? currentPanel).focus();
    };

    const onKeyDown = (event: KeyboardEvent) => {
      const currentPanel = panel.current;
      if (!currentPanel) return;

      if (event.key === "Escape") {
        event.preventDefault();
        onCloseRef.current();
        return;
      }

      if (event.key !== "Tab") return;

      const focusable = getFocusableElements(currentPanel);
      if (focusable.length === 0) {
        event.preventDefault();
        currentPanel.focus();
        return;
      }

      const active = document.activeElement;
      const first = focusable[0];
      const last = focusable[focusable.length - 1];
      const activeIndex = active instanceof HTMLElement ? focusable.indexOf(active) : -1;

      // A script, browser extension, or assistive technology can move focus
      // outside the dialog without producing a Tab event inside it. Pull it
      // back into the trap before allowing the traversal to continue.
      if (!currentPanel.contains(active)) {
        event.preventDefault();
        (event.shiftKey ? last : first).focus();
      } else if (event.shiftKey && (activeIndex <= 0 || activeIndex === -1)) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && (activeIndex === focusable.length - 1 || activeIndex === -1)) {
        event.preventDefault();
        first.focus();
      }
    };
    const onFocusIn = (event: FocusEvent) => {
      const currentPanel = panel.current;
      if (!currentPanel || !(event.target instanceof Node) || currentPanel.contains(event.target)) {
        return;
      }
      // Keep programmatic focus changes and browser/assistive-technology focus
      // moves inside the active dialog as well as keyboard traversal.
      focusInside();
    };
    document.addEventListener("keydown", onKeyDown);
    document.addEventListener("focusin", onFocusIn);

    openerRef.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;

    // Moving focus in makes the dialog reachable by keyboard immediately, prioritizing
    // editable inputs first, then actions.
    const focusable = panel.current && getFocusableElements(panel.current);
    const formControl =
      panel.current &&
      Array.from(
        panel.current.querySelectorAll<HTMLElement>(
          "input:not([type='hidden']), textarea, select",
        ),
      ).find(canFocus);
    (formControl ?? focusable?.[0] ?? panel.current)?.focus();

    const previousOverflow = document.body.style.overflow;
    document.body.style.overflow = "hidden";

    return () => {
      document.removeEventListener("keydown", onKeyDown);
      document.removeEventListener("focusin", onFocusIn);
      document.body.style.overflow = previousOverflow;

      const opener = openerRef.current;
      if (opener && document.contains(opener) && canReceiveFocus(opener)) {
        try {
          opener.focus();
        } catch {
          // A detached or browser-managed element can reject focus even after
          // the connectivity checks above. Closing the dialog must still win.
        }
      }
    };
  }, []);

  if (typeof document === "undefined") return null;

  const widths = {
    sm: "max-w-sm",
    md: "max-w-md",
    lg: "max-w-2xl",
    xl: "max-w-3xl",
  };

  const modal = (
    <div
      class="fixed inset-0 z-50 grid place-items-center overflow-y-auto bg-black/60 p-4 backdrop-blur-sm sm:p-6"
      onClick={onClose}
      role="presentation"
    >
      <div
        ref={panel}
        role="dialog"
        aria-modal="true"
        aria-label={title}
        tabIndex={-1}
        class={`my-auto flex w-full max-h-[calc(100vh-2rem)] sm:max-h-[calc(100vh-3.5rem)] flex-col ${widths[width]} overflow-hidden rounded-2xl border border-ink-700 bg-ink-850 shadow-2xl animate-[fade-in_150ms_ease-out]`}
        // Without this a click inside the panel bubbles to the backdrop and closes it.
        onClick={(event) => event.stopPropagation()}
      >
        <header class="flex items-center justify-between gap-4 border-b border-ink-700 px-5 py-4">
          <h2 class="text-base font-semibold text-fg">{title}</h2>
          <button
            type="button"
            onClick={onClose}
            aria-label={t("common.close")}
            class="grid size-8 shrink-0 place-items-center rounded-lg text-fg-muted transition-colors hover:bg-ink-700 hover:text-fg focus:outline-none focus-visible:ring-2 focus-visible:ring-accent/60"
          >
            <Icon.X size={16} />
          </button>
        </header>

        <div class="min-h-0 flex-1 overflow-y-auto px-5 py-4 text-sm text-fg-muted">{children}</div>

        {footer && (
          <footer class="flex justify-end gap-2 border-t border-ink-700 px-5 py-3.5">
            {footer}
          </footer>
        )}
      </div>
    </div>
  );

  return createPortal(modal, document.body);
}

const FOCUSABLE_SELECTOR = [
  "button",
  "input",
  "select",
  "textarea",
  "a[href]",
  "[tabindex]",
].join(",");

function getFocusableElements(container: HTMLElement): HTMLElement[] {
  return Array.from(container.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR)).filter(canFocus);
}

function canFocus(element: HTMLElement): boolean {
  if (element.matches("[disabled], [hidden], [aria-hidden='true']")) return false;
  if (element.closest("[aria-hidden='true']")) return false;
  if (element instanceof HTMLInputElement && element.type === "hidden") return false;
  if (element.tabIndex < 0) return false;
  return true;
}

function canReceiveFocus(element: HTMLElement): boolean {
  if (element.matches("[disabled], [hidden], [aria-hidden='true']")) return false;
  if (element.closest("[aria-hidden='true']")) return false;
  if (element instanceof HTMLInputElement && element.type === "hidden") return false;
  return true;
}

/** What a caller asks the dialog host to show. */
type ConfirmRequest = {
  kind: "confirm";
  title: string;
  body?: ComponentChildren;
  confirmLabel?: string;
  danger?: boolean;
};

type PromptRequest = {
  kind: "prompt";
  title: string;
  label: string;
  initial?: string;
  placeholder?: string;
  hint?: string;
  password?: boolean;
  confirmLabel?: string;
};

type Request = (ConfirmRequest | PromptRequest) & {
  resolve: (value: string | boolean | null) => void;
};

type QueuedRequest = Request & { id: number };

interface Dialogs {
  /** Resolves true when accepted, false when dismissed. */
  confirm: (request: Omit<ConfirmRequest, "kind">) => Promise<boolean>;
  /** Resolves the entered text, or null when dismissed. */
  prompt: (request: Omit<PromptRequest, "kind">) => Promise<string | null>;
}

const DialogContext = createContext<Dialogs | null>(null);

/** Hosts one dialog at a time and hands out promise-based openers. */
export function DialogProvider({ children }: { children: ComponentChildren }) {
  const [request, setRequest] = useState<QueuedRequest | null>(null);
  const activeRequest = useRef<QueuedRequest | null>(null);
  const pendingRequests = useRef<QueuedRequest[]>([]);
  const nextRequestId = useRef(0);

  const enqueue = useCallback((next: Request) => {
    const queued = { ...next, id: ++nextRequestId.current };
    if (activeRequest.current) {
      pendingRequests.current.push(queued);
      return;
    }
    activeRequest.current = queued;
    setRequest(queued);
  }, []);

  const value = useMemo<Dialogs>(
    () => ({
      confirm: (options) =>
        new Promise<boolean>((resolve) =>
          enqueue({
            ...options,
            kind: "confirm",
            resolve: (value) => resolve(value === true),
          }),
        ),
      prompt: (options) =>
        new Promise<string | null>((resolve) =>
          enqueue({
            ...options,
            kind: "prompt",
            resolve: (value) => resolve(typeof value === "string" ? value : null),
          }),
        ),
    }),
    [enqueue],
  );

  const settle = useCallback(
    (id: number, value: string | boolean | null) => {
      const current = activeRequest.current;
      // A stale callback from a dialog that has already been replaced must
      // never settle the next request in the queue. This also makes an
      // accidental double click on a dialog action harmless.
      if (!current || current.id !== id) return;

      current.resolve(value);
      const next = pendingRequests.current.shift() ?? null;
      activeRequest.current = next;
      setRequest(next);
    },
    [],
  );

  useEffect(
    () => () => {
      activeRequest.current?.resolve(null);
      for (const pending of pendingRequests.current) pending.resolve(null);
      activeRequest.current = null;
      pendingRequests.current = [];
    },
    [],
  );

  return (
    <DialogContext.Provider value={value}>
      {children}
      {request?.kind === "confirm" && (
        <ConfirmDialog key={request.id} request={request} settle={(value) => settle(request.id, value)} />
      )}
      {request?.kind === "prompt" && (
        <PromptDialog key={request.id} request={request} settle={(value) => settle(request.id, value)} />
      )}
    </DialogContext.Provider>
  );
}

function ConfirmDialog({
  request,
  settle,
}: {
  request: ConfirmRequest & { resolve: unknown };
  settle: (value: boolean) => void;
}) {
  const t = useT();

  return (
    <Modal
      title={request.title}
      onClose={() => settle(false)}
      width="sm"
      footer={
        <>
          <Button variant="ghost" onClick={() => settle(false)}>
            {t("common.cancel")}
          </Button>
          <Button
            variant={request.danger ? "danger" : "primary"}
            onClick={() => settle(true)}
          >
            {request.confirmLabel ?? t("common.confirm")}
          </Button>
        </>
      }
    >
      {request.body}
    </Modal>
  );
}

function PromptDialog({
  request,
  settle,
}: {
  request: PromptRequest & { resolve: unknown };
  settle: (value: string | null) => void;
}) {
  const t = useT();
  const [value, setValue] = useState(request.initial ?? "");

  function submit(event: Event) {
    event.preventDefault();
    const trimmed = value.trim();
    // An empty answer is a dismissal, not an instruction to name something "".
    settle(trimmed === "" ? null : trimmed);
  }

  return (
    <Modal
      title={request.title}
      onClose={() => settle(null)}
      width="sm"
      footer={
        <>
          <Button variant="ghost" onClick={() => settle(null)}>
            {t("common.cancel")}
          </Button>
          <Button variant="primary" onClick={submit}>
            {request.confirmLabel ?? t("common.confirm")}
          </Button>
        </>
      }
    >
      <form onSubmit={submit} class="space-y-2">
        <label class="block space-y-1.5">
          <span class="block text-xs font-medium uppercase tracking-wider text-fg-muted">
            {request.label}
          </span>
          <Input
            type={request.password ? "password" : "text"}
            value={value}
            placeholder={request.placeholder}
            autocomplete={request.password ? "new-password" : "off"}
            onInput={(e) => setValue((e.target as HTMLInputElement).value)}
          />
        </label>
        {request.hint && <p class="text-xs text-fg-muted">{request.hint}</p>}
      </form>
    </Modal>
  );
}

/** Promise-based `confirm` and `prompt` replacements. */
export function useDialogs(): Dialogs {
  const context = useContext(DialogContext);
  if (!context) throw new Error("useDialogs must be used inside a DialogProvider");
  return context;
}
