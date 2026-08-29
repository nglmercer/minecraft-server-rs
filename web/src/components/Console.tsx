import { useEffect, useMemo, useRef, useState } from "preact/hooks";
import { openConsole } from "../api";
import { useT } from "../i18n";
import * as Icon from "./icons";
import { IconButton } from "./ui";
import type { ConsoleLine, ServerEvent, Status } from "../types";

/** Keep the DOM bounded; the backend keeps the authoritative buffer. */
const MAX_LINES = 2000;

/** Give up after this many failed reconnects rather than retrying forever. */
const MAX_RECONNECTS = 8;

/** Colour a line by what it obviously is, without parsing log formats strictly. */
function lineClass(line: ConsoleLine): string {
  if (line.stream === "system") return "text-sky-400";
  if (line.stream === "stderr") return "text-red-400";
  if (/\bERROR\b|\bSEVERE\b|Exception|\bFATAL\b/.test(line.line)) return "text-red-400";
  if (/\bWARN\b/.test(line.line)) return "text-amber-300";
  if (/\bDone \(/.test(line.line)) return "text-accent";
  return "text-fg/85";
}

export function Console({
  serverId,
  onStatus,
}: {
  serverId: string;
  onStatus: (status: Status) => void;
}) {
  const t = useT();
  const [lines, setLines] = useState<ConsoleLine[]>([]);
  const [connected, setConnected] = useState(false);
  const [draft, setDraft] = useState("");
  const [history, setHistory] = useState<string[]>([]);
  const [historyAt, setHistoryAt] = useState(-1);

  const [gaveUp, setGaveUp] = useState(false);
  const [expanded, setExpanded] = useState(false);
  const [atBottom, setAtBottom] = useState(true);

  const socket = useRef<WebSocket | null>(null);
  const scroller = useRef<HTMLDivElement | null>(null);
  const pinned = useRef(true);

  useEffect(() => {
    setGaveUp(false);
    setLines([]);
    let closed = false;
    let retry: number | undefined;
    let attempt = 0;

    const connect = async () => {
      let ws: WebSocket;
      try {
        ws = await openConsole(serverId);
      } catch {
        if (closed) return;
        attempt += 1;
        if (attempt > MAX_RECONNECTS) {
          setGaveUp(true);
          return;
        }
        retry = window.setTimeout(connect, Math.min(1000 * 2 ** (attempt - 1), 30000));
        return;
      }
      if (closed) {
        ws.close();
        return;
      }
      socket.current = ws;

      ws.onopen = () => {
        setConnected(true);
        attempt = 0;
      };
      ws.onclose = () => {
        setConnected(false);
        if (closed) return;

        // Backed off and bounded. A socket rejected because the session expired
        // closes immediately every time, and retrying twice a second forever
        // would hammer the panel from a tab nobody is even looking at.
        attempt += 1;
        if (attempt > MAX_RECONNECTS) {
          setGaveUp(true);
          return;
        }
        retry = window.setTimeout(connect, Math.min(1000 * 2 ** (attempt - 1), 30000));
      };
      ws.onmessage = (event) => {
        const message: ServerEvent = JSON.parse(event.data);
        switch (message.type) {
          case "backfill":
            setLines(message.lines.slice(-MAX_LINES));
            onStatus(message.status.status);
            break;
          case "console":
            setLines((prev) => {
              const next = [...prev, message as unknown as ConsoleLine];
              return next.length > MAX_LINES ? next.slice(-MAX_LINES) : next;
            });
            break;
          case "status":
            onStatus(message.status);
            break;
          case "lagged":
            setLines((prev) => [
              ...prev,
              {
                seq: -1,
                stream: "system",
                line: t("console.skipped", { count: message.skipped }),
              },
            ]);
            break;
        }
      };
    };

    void connect();
    return () => {
      closed = true;
      if (retry) clearTimeout(retry);
      socket.current?.close();
    };
  }, [serverId]);

  // Follow the tail, but stop fighting the operator once they scroll up.
  useEffect(() => {
    if (pinned.current && scroller.current) {
      scroller.current.scrollTop = scroller.current.scrollHeight;
    }
  }, [lines]);

  function onScroll() {
    const el = scroller.current;
    if (!el) return;
    pinned.current = el.scrollHeight - el.scrollTop - el.clientHeight < 40;
    setAtBottom(pinned.current);
  }

  function scrollToBottom() {
    const el = scroller.current;
    if (!el) return;
    el.scrollTop = el.scrollHeight;
    pinned.current = true;
    setAtBottom(true);
  }

  function send(event: Event) {
    event.preventDefault();
    const command = draft.trim();
    if (!command || !socket.current || socket.current.readyState !== WebSocket.OPEN) return;
    socket.current.send(JSON.stringify({ type: "command", command }));
    setHistory((prev) => [command, ...prev.filter((c) => c !== command)].slice(0, 50));
    setHistoryAt(-1);
    setDraft("");
    pinned.current = true;
  }

  function onKeyDown(event: KeyboardEvent) {
    if (event.key !== "ArrowUp" && event.key !== "ArrowDown") return;
    event.preventDefault();
    const next = event.key === "ArrowUp"
      ? Math.min(historyAt + 1, history.length - 1)
      : Math.max(historyAt - 1, -1);
    setHistoryAt(next);
    setDraft(next === -1 ? "" : history[next]);
  }

  const rendered = useMemo(
    () =>
      lines.map((line, index) => (
        <div key={`${line.seq}-${index}`} class={`whitespace-pre-wrap break-words ${lineClass(line)}`}>
          {line.line}
        </div>
      )),
    [lines],
  );

  return (
    <section
      class={
        expanded
          ? "fixed inset-0 z-50 flex flex-col bg-ink-900 p-4"
          : "flex min-h-0 flex-1 flex-col gap-3 rounded-2xl border border-ink-700 bg-ink-850 p-4 sm:p-5"
      }
    >
      <div class="flex items-center justify-between">
        <h2 class="flex items-center gap-2.5 text-lg font-semibold">
          {t("console.title")}
          <span
            class={`size-2.5 rounded-full ${
              connected ? "bg-accent" : gaveUp ? "bg-red-500" : "animate-pulse bg-amber-400"
            }`}
            role="status"
            aria-label={connected ? t("console.live") : t("console.reconnecting")}
          />
        </h2>

        <IconButton
          label={expanded ? t("console.collapse") : t("console.expand")}
          icon={expanded ? <Icon.Collapse size={17} /> : <Icon.Expand size={17} />}
          onClick={() => setExpanded((v) => !v)}
        />
      </div>

      <div class="relative min-h-0 flex-1 overflow-hidden rounded-xl bg-ink-950">
        <div
          ref={scroller}
          onScroll={onScroll}
          class="h-full overflow-y-auto px-4 py-3 font-mono text-[13px] leading-relaxed"
        >
          {rendered.length === 0 ? <p class="text-fg-muted">{t("console.empty")}</p> : rendered}
        </div>

        {/* Only offered when it would do something, so it does not sit there
            inviting a click that changes nothing. */}
        {!atBottom && (
          <div class="absolute bottom-3 right-3">
            <IconButton
              label={t("console.toBottom")}
              side="top"
              icon={<Icon.ArrowDown size={17} />}
              onClick={scrollToBottom}
              class="!bg-ink-800 !text-fg shadow-lg hover:!bg-ink-700"
            />
          </div>
        )}
      </div>

      <form
        onSubmit={send}
        class="flex items-center gap-2.5 rounded-xl border border-ink-700 bg-ink-950 px-3.5 py-2.5 focus-within:border-accent/60"
      >
        <span class="shrink-0 text-fg-muted">
          <Icon.Terminal size={16} />
        </span>
        <input
          value={draft}
          onInput={(e) => setDraft((e.target as HTMLInputElement).value)}
          onKeyDown={onKeyDown}
          placeholder={connected ? t("console.placeholder") : t("console.disconnected")}
          disabled={!connected}
          spellcheck={false}
          autocomplete="off"
          class="flex-1 bg-transparent font-mono text-sm text-fg placeholder:text-fg-muted/60 focus:outline-none disabled:opacity-50"
        />
      </form>
    </section>
  );
}
