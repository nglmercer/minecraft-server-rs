import { useEffect, useMemo, useRef, useState } from "preact/hooks";
import { openConsole } from "../api";
import type { ConsoleLine, ServerEvent, Status } from "../types";

/** Keep the DOM bounded; the backend keeps the authoritative buffer. */
const MAX_LINES = 2000;

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
  const [lines, setLines] = useState<ConsoleLine[]>([]);
  const [connected, setConnected] = useState(false);
  const [draft, setDraft] = useState("");
  const [history, setHistory] = useState<string[]>([]);
  const [historyAt, setHistoryAt] = useState(-1);

  const socket = useRef<WebSocket | null>(null);
  const scroller = useRef<HTMLDivElement | null>(null);
  const pinned = useRef(true);

  useEffect(() => {
    setLines([]);
    let closed = false;
    let retry: number | undefined;

    const connect = () => {
      const ws = openConsole(serverId);
      socket.current = ws;

      ws.onopen = () => setConnected(true);
      ws.onclose = () => {
        setConnected(false);
        // Reconnect unless the component is going away; the panel restarting
        // should not require the operator to reload the page.
        if (!closed) retry = window.setTimeout(connect, 2000);
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
                line: `— ${message.skipped} lines skipped (client fell behind) —`,
              },
            ]);
            break;
        }
      };
    };

    connect();
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
    <div class="flex min-h-0 flex-1 flex-col overflow-hidden rounded-xl border border-ink-700 bg-ink-950">
      <div class="flex items-center justify-between border-b border-ink-700 px-4 py-2 text-xs text-fg-muted">
        <span class="font-medium">Console</span>
        <span class="inline-flex items-center gap-1.5">
          <span class={`size-1.5 rounded-full ${connected ? "bg-accent" : "bg-amber-400 animate-pulse"}`} />
          {connected ? "live" : "reconnecting…"}
        </span>
      </div>

      <div
        ref={scroller}
        onScroll={onScroll}
        class="min-h-0 flex-1 overflow-y-auto px-4 py-3 font-mono text-[13px] leading-relaxed"
      >
        {rendered.length === 0 ? (
          <p class="text-fg-muted">No output yet. Start the server to see its log here.</p>
        ) : (
          rendered
        )}
      </div>

      <form onSubmit={send} class="flex items-center gap-2 border-t border-ink-700 px-4 py-2.5">
        <span class="font-mono text-sm text-accent">&gt;</span>
        <input
          value={draft}
          onInput={(e) => setDraft((e.target as HTMLInputElement).value)}
          onKeyDown={onKeyDown}
          placeholder={connected ? "Type a command and press Enter" : "Not connected"}
          disabled={!connected}
          spellcheck={false}
          autocomplete="off"
          class="flex-1 bg-transparent font-mono text-sm text-fg placeholder:text-fg-muted/60 focus:outline-none disabled:opacity-50"
        />
      </form>
    </div>
  );
}
