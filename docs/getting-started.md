# Getting Started

## Install

Download a self-contained binary from [Releases](https://github.com/nglmercer/minecraft-server-rs/releases) — the web UI is embedded, no nginx or Node needed at runtime — or build from source:

```sh
git clone --recurse-submodules https://github.com/nglmercer/minecraft-server-rs
cd minecraft-server-rs
./build.sh
./target/release/mcpanel --data-dir ./data --bind 127.0.0.1:8080
```

On Windows run the two steps `build.sh` wraps:

```sh
cd web && npm install && npm run build
cargo build --release
```

Two Linux builds are published: `linux-x86_64` (glibc) and `linux-x86_64-static` (musl/static) for Alpine or older distros.

## First run

The first run prints a generated `admin` password. It is shown once — save it.

- Default bind is loopback (`127.0.0.1:8080`). For remote access keep it on loopback behind an HTTPS reverse proxy; see [Security — Secure remote deployment](security.md#secure-remote-deployment).
- Direct non-loopback plaintext HTTP is refused unless `--allow-insecure-http` is supplied explicitly for a trusted, isolated network.
- On Linux install `bubblewrap` (`bwrap`) for the strongest sandbox; macOS uses `sandbox-exec` when available. If the helper is unavailable, Minecraft startup is refused unless `--allow-unsandboxed-servers` is supplied (or `MCPANEL_ALLOW_UNSANDBOXED_SERVERS=true` is set). See [Security](security.md) and [Platforms](platforms.md).

## Data directory

Everything the panel owns at runtime lives under `--data-dir`:

```
data/
├── panel.json          users and server records
├── playit/             embedded Playit state
│   └── secret.toml     dedicated Playit secret (never in panel.json)
├── jdks/               JDKs downloaded on demand, shared by all servers
├── servers/<uuid>/     one server's working directory (worlds, plugins, jar)
└── backups/<uuid>/     server backups
```

On Unix the directory should be `0700` and owned by the panel service account; `panel.json` and `playit/secret.toml` are written owner-only. Do not place it in a shared directory.

## Next steps

- [Architecture](architecture.md) — crate layout and why the stack is this size.
- [Development](development.md) — run backend + Vite dev server side by side.
- [Playit](playit.md) — expose a server without port-forwarding.
- [Security](security.md) — threat model before granting operator access.
