# Documentation

Welcome to the `minecraft-server-rs` documentation. The root [`README.md`](../README.md) is the entry point; this directory contains the modular guides extracted from it.

## Contents

| Guide | What it covers |
|-------|---------------|
| [Getting Started](getting-started.md) | Install, run, first login, data directory |
| [Architecture](architecture.md) | Crate layout, embedded stack, why this stack, diagram |
| [Development](development.md) | Local backend + frontend workflow, `--dev-cors`, aliases |
| [Playit](playit.md) | Embedded runtime, claim flow, tunnel lifecycle, external mode |
| [API](api.md) | REST + WebSocket endpoints, auth, tickets, accounts |
| [Security](security.md) | Trust levels, isolation, path safety, secure deployment |
| [Operations](operations.md) | Starting servers, install record, backups, plugins & limits |
| [Frontend](frontend.md) | Icons, accessibility, mobile, layout |
| [Internationalisation](i18n.md) | Languages, adding a translation |
| [Platforms](platforms.md) | Linux / macOS / Windows notes |
| [Testing](testing.md) | Rust + frontend tests, CI, supervisor stand-in |
| [Releasing](releasing.md) | Tag, build matrix, release verification |

## Conventions

- All paths in guides are relative to the repository root unless noted.
- `mcpanel` is the `panel` crate binary (`crates/panel`).
- `--data-dir` defaults vary by guide; examples use `./data` or `/var/lib/mcpanel`.

See also [`SECURITY.md`](../SECURITY.md) for the supported-versions and disclosure policy.
