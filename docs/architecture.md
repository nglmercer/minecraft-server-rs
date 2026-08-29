# Architecture

```
┌─────────────────┐     ┌────────────────────┐
│  java-path-rs   │     │ minecraft-core-rs  │
│  which Java,    │     │ which build,       │
│  and where      │     │ and where          │
└────────┬────────┘     └─────────┬──────────┘
         └──────────┬─────────────┘
                    ▼
            ┌───────────────┐
            │   guardian    │  process lifecycle: spawn, stdio,
            │               │  status machine, graceful stop,
            └───────┬───────┘  crash detection, auto restart
                    ▼
            ┌───────────────┐
            │     panel     │  REST + WebSocket API, auth, files,
            │   (mcpanel)   │  embedded Playit and web UI
            └───────────────┘
```

`minecraft-server-rs` does what Pterodactyl does for Minecraft specifically — install Java, download the server, run it, stream the console, edit files, restart on crash — without PHP, Laravel, MySQL, Redis, Docker or a separate node daemon. One binary, one config file, no runtime dependencies.

## Layout

| Path                        | What it is                                         |
| --------------------------- | -------------------------------------------------- |
| `crates/guardian`           | Lifecycle library. No HTTP, no panel concepts.     |
| `crates/panel`              | `mcpanel` binary: API, auth, embedded frontend.    |
| `crates/playit-integration` | Embedded Playit runtime and optional daemon IPC.  |
| `web`                       | Vite + Preact + TypeScript + Tailwind v4 frontend. |
| `vendor/java-path-rs`       | Submodule: Java discovery and provisioning.        |
| `vendor/minecraft-core-rs`  | Submodule: server artifact resolution and download.|

Runtime layout is described in [Getting Started — Data directory](getting-started.md#data-directory).

## Why this stack

**The frontend is a web app because the panel is multi-user and remote.** A TUI cannot be shared with a friend who is not on your box, and a desktop app has to be installed on every machine you want to administrate from.

**It is embedded in the binary** (`rust-embed`), so shipping is one file. There is no nginx to configure, no static directory to keep in sync with the executable, and no Node on the server.

Preact + TypeScript + Tailwind is the right size: the whole UI compiles to ~40 KB JavaScript and ~20 KB CSS. React would triple the runtime for nothing this app needs, and hand-written CSS would cost more time than Tailwind saves.

## Related

- [Frontend](frontend.md) — icons, CSP, accessibility.
- [Platforms](platforms.md) — OS abstractions.
- [Security](security.md) — sandbox boundary per platform.
