import type { ComponentChildren, JSX } from "preact";
import type { Status } from "../types";

/** Colour and label for each lifecycle state. */
const STATUS_STYLE: Record<Status, { dot: string; text: string; label: string }> = {
  offline: { dot: "bg-slate-500", text: "text-slate-400", label: "Offline" },
  preparing: { dot: "bg-sky-400 animate-pulse", text: "text-sky-300", label: "Preparing" },
  starting: { dot: "bg-amber-400 animate-pulse", text: "text-amber-300", label: "Starting" },
  online: { dot: "bg-accent", text: "text-accent", label: "Online" },
  stopping: { dot: "bg-amber-400 animate-pulse", text: "text-amber-300", label: "Stopping" },
  crashed: { dot: "bg-red-500", text: "text-red-400", label: "Crashed" },
};

export function StatusPill({ status }: { status: Status }) {
  const style = STATUS_STYLE[status] ?? STATUS_STYLE.offline;
  return (
    <span class="inline-flex items-center gap-2 text-sm font-medium">
      <span class={`size-2 rounded-full ${style.dot}`} />
      <span class={style.text}>{style.label}</span>
    </span>
  );
}

type ButtonProps = JSX.IntrinsicElements["button"] & {
  variant?: "primary" | "ghost" | "danger" | "subtle";
};

export function Button({ variant = "subtle", class: extra, ...props }: ButtonProps) {
  const base =
    "inline-flex items-center justify-center gap-2 rounded-lg px-3.5 py-2 text-sm font-medium " +
    "transition-colors disabled:opacity-40 disabled:cursor-not-allowed focus:outline-none " +
    "focus-visible:ring-2 focus-visible:ring-accent/60";
  const variants = {
    primary: "bg-accent text-ink-950 hover:bg-accent/90",
    danger: "bg-red-600/90 text-white hover:bg-red-600",
    ghost: "text-fg-muted hover:text-fg hover:bg-ink-700",
    subtle: "bg-ink-700 text-fg hover:bg-ink-600",
  };
  return <button {...props} class={`${base} ${variants[variant]} ${extra ?? ""}`} />;
}

export function Card({
  title,
  actions,
  children,
  class: extra,
}: {
  title?: ComponentChildren;
  actions?: ComponentChildren;
  children: ComponentChildren;
  class?: string;
}) {
  return (
    <section class={`rounded-xl border border-ink-700 bg-ink-850 ${extra ?? ""}`}>
      {(title || actions) && (
        <header class="flex items-center justify-between gap-4 border-b border-ink-700 px-5 py-3.5">
          <h2 class="text-sm font-semibold tracking-wide text-fg">{title}</h2>
          {actions}
        </header>
      )}
      <div class="p-5">{children}</div>
    </section>
  );
}

export function Field({
  label,
  hint,
  children,
}: {
  label: string;
  hint?: string;
  children: ComponentChildren;
}) {
  return (
    <label class="block space-y-1.5">
      <span class="block text-xs font-medium uppercase tracking-wider text-fg-muted">
        {label}
      </span>
      {children}
      {hint && <span class="block text-xs text-fg-muted">{hint}</span>}
    </label>
  );
}

const CONTROL =
  "w-full rounded-lg border border-ink-600 bg-ink-900 px-3 py-2 text-sm text-fg " +
  "placeholder:text-fg-muted/60 focus:border-accent focus:outline-none focus:ring-1 focus:ring-accent";

export function Input(props: JSX.IntrinsicElements["input"]) {
  const { class: extra, ...rest } = props;
  return <input {...rest} class={`${CONTROL} ${extra ?? ""}`} />;
}

export function Select(props: JSX.IntrinsicElements["select"]) {
  const { class: extra, ...rest } = props;
  return <select {...rest} class={`${CONTROL} ${extra ?? ""}`} />;
}

export function Banner({ kind, children }: { kind: "error" | "info"; children: ComponentChildren }) {
  const styles =
    kind === "error"
      ? "border-red-500/40 bg-red-500/10 text-red-200"
      : "border-sky-500/40 bg-sky-500/10 text-sky-100";
  return <div class={`rounded-lg border px-4 py-3 text-sm ${styles}`}>{children}</div>;
}

/** `1h 04m` style duration, for uptimes. */
export function formatUptime(seconds: number | null): string {
  if (seconds === null) return "—";
  const h = Math.floor(seconds / 3600);
  const m = Math.floor((seconds % 3600) / 60);
  const s = seconds % 60;
  if (h > 0) return `${h}h ${String(m).padStart(2, "0")}m`;
  if (m > 0) return `${m}m ${String(s).padStart(2, "0")}s`;
  return `${s}s`;
}

/** Human-readable byte counts for the file manager. */
export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KiB", "MiB", "GiB", "TiB"];
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value.toFixed(value < 10 ? 1 : 0)} ${units[unit]}`;
}
