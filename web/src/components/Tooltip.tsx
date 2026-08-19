import type { ComponentChildren } from "preact";

/**
 * A hover and focus tooltip.
 *
 * CSS-driven rather than JS-positioned: there is no measurement, nothing to
 * recalculate on scroll, and it appears on keyboard focus as well as hover —
 * which a mouse-only implementation would miss. Touch devices have no hover, so
 * a tooltip must never be the only place a label exists; every icon-only
 * control below also carries an `aria-label`.
 */
export function Tooltip({
  label,
  side = "bottom",
  children,
}: {
  label: string;
  side?: "top" | "bottom";
  children: ComponentChildren;
}) {
  const position =
    side === "top"
      ? "bottom-full mb-2 origin-bottom"
      : "top-full mt-2 origin-top";

  return (
    <span class="group/tooltip relative inline-flex">
      {children}
      <span
        role="tooltip"
        class={`pointer-events-none absolute left-1/2 z-40 -translate-x-1/2 scale-95 whitespace-nowrap rounded-lg border border-ink-700 bg-ink-950 px-2.5 py-1.5 text-xs text-fg opacity-0 shadow-lg transition-[opacity,transform] duration-100 group-hover/tooltip:scale-100 group-hover/tooltip:opacity-100 group-focus-within/tooltip:scale-100 group-focus-within/tooltip:opacity-100 ${position}`}
      >
        {label}
      </span>
    </span>
  );
}
