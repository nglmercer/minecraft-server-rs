import type { ComponentChildren, JSX } from "preact";
import { useT } from "../i18n";
import { Tooltip } from "./Tooltip";
import type { Status } from "../types";

/** Colour for each lifecycle state. The label comes from the dictionary. */
const STATUS_STYLE: Record<Status, { dot: string; text: string }> = {
  offline: { dot: "bg-slate-500", text: "text-slate-400" },
  preparing: { dot: "bg-sky-400 animate-pulse", text: "text-sky-300" },
  starting: { dot: "bg-amber-400 animate-pulse", text: "text-amber-300" },
  online: { dot: "bg-accent", text: "text-accent" },
  stopping: { dot: "bg-amber-400 animate-pulse", text: "text-amber-300" },
  crashed: { dot: "bg-red-500", text: "text-red-400" },
};

export function StatusPill({ status }: { status: Status }) {
  const t = useT();
  const style = STATUS_STYLE[status] ?? STATUS_STYLE.offline;

  return (
    <span class="inline-flex items-center gap-2 text-sm font-medium">
      <span class={`size-2 rounded-full ${style.dot}`} />
      <span class={style.text}>{t(`status.${status}` as "status.offline")}</span>
    </span>
  );
}

/** A right-aligned row of actions, used in toolbars and card headers. */
export function Actions({ children }: { children: ComponentChildren }) {
  return <div class="flex flex-wrap items-center justify-end gap-2">{children}</div>;
}

/** Placeholder for a list or table with nothing in it. */
export function Empty({ children }: { children: ComponentChildren }) {
  return <p class="px-4 py-8 text-center text-sm text-fg-muted">{children}</p>;
}

type ButtonProps = JSX.IntrinsicElements["button"] & {
  variant?: "primary" | "ghost" | "danger" | "subtle";
  /** Rendered before the label. */
  icon?: ComponentChildren;
};

export function Button({
  variant = "subtle",
  icon,
  class: extra,
  children,
  ...props
}: ButtonProps) {
  const base =
    "inline-flex items-center justify-center gap-2 rounded-full px-4 py-2 text-sm font-medium " +
    "transition-colors disabled:opacity-40 disabled:cursor-not-allowed focus:outline-none " +
    "focus-visible:ring-2 focus-visible:ring-accent/60";
  const variants = {
    primary: "bg-accent text-ink-950 hover:bg-accent/90",
    danger: "bg-red-500/15 text-red-300 hover:bg-red-500/25",
    ghost: "text-fg-muted hover:text-fg hover:bg-ink-700",
    subtle: "bg-ink-700 text-fg hover:bg-ink-600",
  };
  return (
    <button {...props} class={`${base} ${variants[variant]} ${extra ?? ""}`}>
      {icon}
      {children}
    </button>
  );
}

/**
 * A square, icon-only control.
 *
 * The label is required and becomes both the tooltip and the accessible name,
 * so the control is never unlabelled on a touchscreen where hover does not exist.
 */
export function IconButton({
  label,
  icon,
  side = "bottom",
  class: extra,
  ...props
}: JSX.IntrinsicElements["button"] & {
  label: string;
  icon: ComponentChildren;
  side?: "top" | "bottom";
}) {
  return (
    <Tooltip label={label} side={side}>
      <button
        {...props}
        aria-label={label}
        class={`grid size-9 place-items-center rounded-lg text-fg-muted transition-colors hover:bg-ink-700 hover:text-fg disabled:cursor-not-allowed disabled:opacity-40 focus:outline-none focus-visible:ring-2 focus-visible:ring-accent/60 ${extra ?? ""}`}
      >
        {icon}
      </button>
    </Tooltip>
  );
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

/**
 * A headline metric with an optional capacity bar.
 *
 * `max` is genuinely optional: disk usage has no configured ceiling here, and
 * inventing one to make the card look symmetrical would be a lie.
 */
export function StatCard({
  value,
  max,
  label,
  icon,
  fraction,
  tone = "accent",
}: {
  value: string;
  max?: string;
  label: string;
  icon: ComponentChildren;
  /** 0..1. Omit to render the card without a bar. */
  fraction?: number;
  tone?: "accent" | "warn";
}) {
  const clamped = fraction === undefined ? undefined : Math.max(0, Math.min(1, fraction));
  const bar = tone === "warn" ? "bg-amber-400" : "bg-accent";

  return (
    <section class="relative overflow-hidden rounded-2xl border border-ink-700 bg-ink-850 px-5 py-4">
      <div class="flex items-start justify-between gap-3">
        <p class="flex items-baseline gap-1.5">
          <span class="text-2xl font-semibold tabular-nums tracking-tight sm:text-3xl">
            {value}
          </span>
          {max && <span class="text-sm text-fg-muted">/ {max}</span>}
        </p>
        <span class="shrink-0 text-fg-muted">{icon}</span>
      </div>

      <p class="mt-1 text-sm text-fg-muted">{label}</p>

      {clamped !== undefined && (
        <>
          {/* A soft wash above the bar, so a full card reads as full at a glance. */}
          <div
            class={`pointer-events-none absolute inset-x-0 bottom-0 bg-gradient-to-t ${
              tone === "warn" ? "from-amber-400/20" : "from-accent/20"
            } to-transparent transition-[height] duration-500`}
            style={{ height: `${clamped * 100}%` }}
          />
          <div class="absolute inset-x-0 bottom-0 h-1 bg-ink-700">
            <div
              class={`h-full ${bar} transition-[width] duration-500`}
              style={{ width: `${clamped * 100}%` }}
            />
          </div>
        </>
      )}
    </section>
  );
}

/** `1h 04m` style duration, for uptimes. */
export function formatUptime(seconds: number | null): string {
  if (seconds === null || seconds < 0) return "—";
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
