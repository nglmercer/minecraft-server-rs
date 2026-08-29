# minecraft-server-rs

[![CI](https://github.com/nglmercer/minecraft-server-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/nglmercer/minecraft-server-rs/actions/workflows/ci.yml)

A Minecraft server control panel that is one binary, one config file, and no dependencies at runtime.

It does what Pterodactyl does for Minecraft specifically — install Java, download the server, run it, stream the console, edit the files, restart it when it crashes — without PHP, Laravel, MySQL, Redis, Docker or a separate node daemon.

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

## Quick start

```sh
git clone --recurse-submodules https://github.com/nglmercer/minecraft-server-rs
cd minecraft-server-rs
./build.sh
./target/release/mcpanel --data-dir ./data --bind 127.0.0.1:8080
```

Or download a self-contained binary from [Releases](https://github.com/nglmercer/minecraft-server-rs/releases) (web UI is embedded). The first run prints a generated `admin` password — shown once.

Default bind is loopback. For remote access keep it on loopback behind an HTTPS reverse proxy. Direct non-loopback plaintext HTTP requires `--allow-insecure-http` explicitly and is only for isolated networks.

> Full install, data-dir layout, and static vs glibc builds → [docs/getting-started.md](docs/getting-started.md)

## Documentation

| Guide | Content |
|-------|---------|
| [Getting Started](docs/getting-started.md) | Install, first run, data directory |
| [Architecture](docs/architecture.md) | Crate layout, why this stack, diagram |
| [Development](docs/development.md) | `cargo run` + Vite dev server, `--dev-cors`, aliases |
| [Playit](docs/playit.md) | Embedded runtime, claim flow, tunnel lifecycle, external mode |
| [API](docs/api.md) | REST + WebSocket, auth, tickets, accounts |
| [Security](docs/security.md) | Trust levels, sandboxing, path safety, secure deployment |
| [Operations](docs/operations.md) | Starting servers, install record, backups, plugins & limits |
| [Frontend](docs/frontend.md) | Icons, a11y, mobile |
| [Internationalisation](docs/i18n.md) | Languages, adding a translation |
| [Platforms](docs/platforms.md) | Linux / macOS / Windows notes |
| [Testing](docs/testing.md) | Rust + frontend tests, CI, supervisor stand-in |
| [Releasing](docs/releasing.md) | Status, versioning, release workflow |

Browse all guides in [`docs/`](docs/README.md). Security disclosure policy is in [`SECURITY.md`](SECURITY.md).

## Layout

| Path | What it is |
|------|------------|
| `crates/guardian` | Lifecycle library. No HTTP, no panel concepts. |
| `crates/panel` | `mcpanel` binary: API, auth, embedded frontend. |
| `crates/playit-integration` | Embedded Playit runtime and optional daemon IPC. |
| `web` | Vite + Preact + TypeScript + Tailwind v4 frontend. |
| `vendor/java-path-rs` | Submodule: Java discovery and provisioning. |
| `vendor/minecraft-core-rs` | Submodule: server artifact resolution and download. |

Runtime lives under `--data-dir` (`panel.json`, `playit/secret.toml`, `jdks/`, `servers/<uuid>/`, `backups/<uuid>/`). Details in [Getting Started](docs/getting-started.md) and [Architecture](docs/architecture.md).

On Linux install `bubblewrap` (`bwrap`) for the strongest sandbox; see [Security](docs/security.md).

## Development

```sh
cargo run -p panel -- --data-dir ./data --bind 127.0.0.1:8080 --dev-cors
cd web && npm run dev     # http://localhost:5173, proxies /api to :8080
```

`cargo dev-check` / `cargo dev-clippy` / `cargo workspace-test` are workspace aliases for the fast loop. See [Development](docs/development.md) and [Testing](docs/testing.md).

## API at a glance

Everything under `/api`. Browser sessions use `HttpOnly` session + CSRF cookies; API clients can use `Authorization: Bearer <token>`. Console and file downloads use short-lived one-use tickets (`POST .../ticket` → `GET .../download?ticket=`). Full table in [API](docs/api.md).

## Security

Four trust levels: panel admin ≈ host admin; server operator (scoped to assigned ids); untrusted plugin/mod (arbitrary JVM code); host administrator. File APIs are capability-based and reject traversal/symlink/hard-link attacks; Linux `bwrap` / macOS `sandbox-exec` sandbox each Minecraft process. See [Security](docs/security.md) before granting untrusted operators access.

## Status

Working end-to-end against a live Paper server — provisioning, supervision, crash recovery, console, file manager, backups, Modrinth installs, multi-user access, and Playit tunnels (embedded by default). Scheduled backups, Hangar/CurseForge, and persisted console buffer are not built yet. See [Releasing](docs/releasing.md).

## Licence

MIT — see [LICENSE](LICENSE).
