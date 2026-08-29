# Frontend

The panel frontend lives in `web` (Vite + Preact + TypeScript + Tailwind v4) and is embedded in the binary via `rust-embed`. The whole UI compiles to ~40 KB JavaScript and ~20 KB CSS; see [Architecture — Why this stack](architecture.md#why-this-stack).

## Icons

Icons are inline SVG rather than an icon font or a sprite sheet: the frontend is embedded in the binary and served under a strict CSP, so anything fetched from elsewhere would not load. They inherit `currentColor`, so a control's icon and its text always match.

## Accessibility

Icon-only controls carry a tooltip *and* an `aria-label`. The tooltip is a convenience for pointer users; the label is what makes the control usable on a touchscreen, where hover does not exist, and in a screen reader.

## Mobile

Row actions live in a contextual menu rather than on hover, because a hover target does not exist on a touchscreen. It opens three ways: the always-visible `⋯` button, a right-click, or a long-press — the last two arrive as the same `contextmenu` event, which is suppressed so the browser's own menu does not appear instead.

Below 640px the menu becomes a bottom sheet with larger hit targets, tables drop their less important columns rather than scrolling sideways, and toolbars wrap.

## Development notes

```sh
cd web
npm install
npm run dev     # http://localhost:5173, proxies /api to :8080
npm test
npm run build   # also type-checks (tsc)
```

See [Development](development.md) and [Testing](testing.md).
