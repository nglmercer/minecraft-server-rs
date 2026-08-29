# minecraft-server-rs

[![CI](https://github.com/nglmercer/minecraft-server-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/nglmercer/minecraft-server-rs/actions/workflows/ci.yml)

A Minecraft server control panel that is one binary, one config file, and no
dependencies at runtime.

It does what Pterodactyl does for Minecraft specifically — install Java,
download the server, run it, stream the console, edit the files, restart it
when it crashes — without PHP, Laravel, MySQL, Redis, Docker or a separate
node daemon.

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

Download a binary from [Releases](https://github.com/nglmercer/minecraft-server-rs/releases)
— it is self-contained, with the web UI inside it — or build from source:

```sh
git clone --recurse-submodules https://github.com/nglmercer/minecraft-server-rs
cd minecraft-server-rs
./build.sh
./target/release/mcpanel --data-dir ./data --bind 127.0.0.1:8080
```

The default bind is loopback. For remote access, keep the panel on loopback
and put it behind an HTTPS reverse proxy. Direct non-loopback plaintext HTTP is
refused unless `--allow-insecure-http` is supplied explicitly for a trusted,
isolated network.

Two Linux builds are published: `linux-x86_64` links against glibc, and
`linux-x86_64-static` is statically linked, for Alpine or any distro older than
the one the glibc build was made on.

The first run prints a generated `admin` password. It is shown once.

## Layout

| Path                        | What it is                                            |
| --------------------------- | ----------------------------------------------------- |
| `crates/guardian`           | The lifecycle library. No HTTP, no panel concepts.    |
| `crates/panel`              | The `mcpanel` binary: API, auth, embedded frontend.   |
| `crates/playit-integration` | Embedded Playit runtime and optional daemon IPC.     |
| `web`                       | Vite + Preact + TypeScript + Tailwind v4 frontend.    |
| `vendor/java-path-rs`       | Submodule: Java discovery and provisioning.           |
| `vendor/minecraft-core-rs`  | Submodule: server artifact resolution and download.   |

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

On Linux, install `bubblewrap` (`bwrap`) so Minecraft processes receive the
strongest sandbox this binary can use. The panel falls back to application-level
path and environment restrictions when the helper is unavailable; see the
[security model](#security-model) before using that fallback for multiple
untrusted operators.

## Why this stack

**The frontend is a web app because the panel is multi-user and remote.** A TUI
cannot be shared with a friend who is not on your box, and a desktop app has to
be installed on every machine you want to administrate from.

**It is embedded in the binary** (`rust-embed`), so shipping is one file. There
is no nginx to configure, no static directory to keep in sync with the
executable, and no Node on the server.

Preact + TypeScript + Tailwind is the right size for it: the whole UI compiles
to ~40 KB of JavaScript and ~20 KB of CSS. React would triple the runtime for
nothing this app needs, and hand-written CSS would cost more time than Tailwind
saves.

## Development

Run the backend and the Vite dev server side by side:

```sh
cargo run -p panel -- --data-dir ./data --bind 127.0.0.1:8080 --dev-cors
cd web && npm run dev     # http://localhost:5173, proxies /api to :8080
```

`--dev-cors` enables a permissive CORS policy. Do not use it in production.

## Playit setup

No separate Playit installation is required. Start `mcpanel`, open the
admin-only **Playit** section in the web UI (`#/playit`), and click **Connect**.
The panel starts an embedded Playit runtime and stores its dedicated secret
under `<data-dir>/playit/secret.toml`. Open the generated claim link and return
to the panel when approval is complete.

Once Playit is connected, choose a server and create its tunnel. The panel uses
TCP to `127.0.0.1:<server-port>`, stores the Playit tunnel id in `panel.json`,
and polls the service so provisioning, disabled, drifted, and connected states
are visible. The server settings page also exposes the same attach/detach
controls. Deleting a server or tunnel removes a panel-managed tunnel first
when the service is available.

Operators who intentionally run a compatible external `playitd` can select the
legacy IPC backend explicitly. Keep the listener on loopback, or explicitly
acknowledge the risk with `--allow-insecure-http` on an isolated network:

```sh
mcpanel --playit-mode external --data-dir ./data --bind 127.0.0.1:8080
```

The `MCPANEL_PLAYIT_MODE=external` environment variable is equivalent. External
mode does not stop the independently managed daemon when the panel exits.

Embedded and external modes keep separate Playit credentials. Switching from an
external daemon does not import its secret automatically, so embedded mode may
require a new claim. Existing panel tunnel bindings are preserved and are
reconciled against whichever Playit account is active.

The runtime boundary is:

```
panel (mcpanel)
    │
    └── PlayitManager
          ├── embedded PlayitRuntime (default)
          └── external playitd IPC (optional)
```

## API

Everything is under `/api`. Browser sessions use an `HttpOnly` session cookie
and a separate CSRF cookie. API clients may continue to use
`Authorization: Bearer <token>`. A browser first calls
`POST /servers/{id}/ws/ticket`, then connects to the WebSocket with its
short-lived one-use `?ticket=`.

| Method              | Path                                   | Purpose                            |
| ------------------- | -------------------------------------- | ---------------------------------- |
| `POST`              | `/auth/login`                          | Create a cookie session             |
| `POST`              | `/auth/logout`                         | Revoke the current session          |
| `GET`               | `/auth/me`                             | The current account                |
| `POST`              | `/auth/password`                       | Change your password               |
| `GET`               | `/playit/status`                       | Inspect Playit service state       |
| `GET`               | `/playit/account`                      | Inspect Playit account state (admin) |
| `POST`              | `/playit/claim`                       | Start the browser-based claim flow (admin) |
| `GET` `POST`        | `/playit/tunnels`                     | List / create tunnels (admin)      |
| `DELETE`            | `/playit/tunnels/{id}`                | Delete a tunnel (admin)            |
| `GET` `POST`        | `/servers`                             | List / create                      |
| `GET` `PATCH` `DELETE` | `/servers/{id}`                     | Inspect / reconfigure / remove     |
| `GET` `POST` `DELETE` | `/servers/{id}/playit`              | Inspect / attach / detach its tunnel |
| `POST`              | `/servers/{id}/power`                  | `start`, `stop`, `restart`, `kill` |
| `POST`              | `/servers/{id}/command`                | Send a console command             |
| `POST`              | `/servers/{id}/reinstall`              | Re-resolve and download the artifact |
| `GET`               | `/servers/{id}/logs`                   | The retained console buffer        |
| `WS`                | `/servers/{id}/ws`                     | Live console, both directions      |
| `POST`              | `/servers/{id}/ws/ticket`              | Short-lived one-use console grant  |
| `GET` `PUT` `DELETE`| `/servers/{id}/files`                  | List / write / delete              |
| `GET`               | `/servers/{id}/files/read`             | Read a text file                   |
| `GET`               | `/servers/{id}/files/sizes`            | Measure the subdirectories of a path |
| `POST`              | `/servers/{id}/files/ticket`           | Short-lived grant for one download  |
| `GET`               | `/servers/{id}/files/download`         | Stream a file out, given a ticket   |
| `POST`              | `/servers/{id}/files/upload`           | Multipart upload into a directory  |
| `POST`              | `/servers/{id}/files/extract`          | Unpack a `.zip`/`.jar`/`.tar.gz`   |
| `POST`              | `/servers/{id}/files/rename`           | Rename or move                     |
| `POST`              | `/servers/{id}/files/mkdir`            | Create a directory                 |
| `GET` `POST`        | `/servers/{id}/backups`                | List / take a backup               |
| `DELETE`            | `/servers/{id}/backups/{backup}`       | Delete a backup                    |
| `POST`              | `/servers/{id}/backups/{backup}/restore` | Restore (server must be stopped) |
| `POST`              | `/servers/{id}/backups/{backup}/ticket` | Short-lived grant for one download |
| `GET`               | `/servers/{id}/backups/{backup}/download` | Stream the archive, given a ticket |
| `GET`               | `/servers/{id}/mods`                   | Installed plugins or mods          |
| `GET`               | `/servers/{id}/mods/search`            | Search Modrinth, scoped to this server |
| `POST`              | `/servers/{id}/mods/install`           | Install a Modrinth project         |
| `GET` `POST`        | `/users`                               | List / create accounts (admin)     |
| `PATCH` `DELETE`    | `/users/{username}`                    | Update / delete an account (admin) |
| `GET`               | `/catalog/providers`                   | Installable server flavours        |
| `GET`               | `/catalog/{provider}/versions`         | Versions for a flavour             |
| `GET`               | `/catalog/{provider}/{version}/builds` | Builds for a version               |
| `GET`               | `/catalog/javas`                       | Java installations on the host      |
| `GET`               | `/system`                              | Host CPU, memory, servers online   |

Deleting a server removes it from the panel and leaves its files on disk. That
is deliberate: a world should not be destroyable by a misclick in a browser.

### Downloads

A download is a browser navigation, and a browser cannot attach an
`Authorization` header to one. Rather than putting the session token in the
query string — where it lands in browser history, proxy logs and the panel's own
request log — the client asks for a *ticket* first. A ticket names one file or
one backup, expires after a minute, and grants nothing else. Console tickets
expire faster, are bound to the issuing session, and are consumed at the
handshake. Long-lived session tokens are never accepted in query strings.

### Accounts and access

Admins see and manage everything. A regular account only reaches the servers it
has been granted, across every endpoint including the console socket and the
file manager. Changing an account's password or permissions revokes its existing
sessions, so a demotion takes effect immediately rather than at the next login.

## Security model

The panel has four materially different trust levels:

* A **panel administrator** can manage every server, account, tunnel, and
  panel setting. Treat an administrator as a host administrator.
* A **server operator** can use only the server ids assigned to that account:
  its power controls, console, files, backups, and supported Modrinth installs.
  The API consistently hides unassigned servers as `404`.
* **Minecraft plugins and mods are not trusted code.** Installing one gives
  arbitrary Java code the authority available to that server process. A plugin
  can read or modify everything visible inside its process sandbox and can
  consume its CPU, memory, disk, process, and network budget. Do not install an
  untrusted plugin merely because it came from Modrinth.
* The **host administrator** owns the operating system, the panel account, its
  data directory, and any external services. No application can protect a
  server from a host administrator.

Server-scoped file operations use descriptor-relative capabilities and reject
path traversal, symlink parents, final symlinks, hard links, and archive link
entries. Uploaded and restored archives are bounded by entry, per-file, and
expanded-byte limits. Uploads, editor writes, Modrinth installs, and retained
backups are quota checked before publication.

On Linux, `bwrap` places each Minecraft process in a separate PID/filesystem
view, exposes only its server directory and the selected JDK read-only, and
uses a server-local home and temporary directory. On macOS the available
`sandbox-exec` profile provides a comparable filesystem boundary. If the helper
is absent, or on Windows where this binary does not currently create a kernel
job/container identity, the panel still sanitizes environment variables and
uses race-resistant server paths, but a Java process runs as the panel's OS
user. That fallback is **not strong tenant isolation**: deploy the panel under
a dedicated low-privilege service account and do not grant mutually untrusted
operators access to it. CPU, process-count, file-descriptor, and native-memory
limits are not a substitute for cgroups/job objects on those platforms.

The selected JVM heap, request bodies, archive expansion, downloads, server
files, and backups have finite defaults. Tune them with the `--max-*` options
for the host, and use OS-level cgroups, quotas, or a container when hard host
resource isolation is required.

### Secure remote deployment

The binary serves HTTP and does not terminate TLS itself. Keep it on loopback:

```sh
mcpanel --data-dir /var/lib/mcpanel --bind 127.0.0.1:8080
```

Terminate HTTPS in a reverse proxy, forward only to that loopback listener,
and configure the proxy to pass WebSocket upgrades. For example, a Caddy
`reverse_proxy 127.0.0.1:8080` site provides TLS and WebSocket forwarding by
default. Do not expose `0.0.0.0:8080` directly. The
`--allow-insecure-http` option exists for development or an explicitly isolated
network and is not a production security control.

The data directory should be owned by the panel service account with mode
`0700`; `panel.json` and the embedded Playit secret are written with owner-only
permissions on Unix. Do not put backups or the data directory in a shared
directory, and do not pass credentials through command-line arguments or logs.

### Starting a server

`POST /servers/{id}/power` with `start` returns as soon as the work is accepted,
not when the server is up. Provisioning downloads a JDK and a server jar and can
take minutes, so it runs detached from the request: a browser that navigates
away mid-download cannot strand the server in `preparing`.

Watch the WebSocket for `starting` and then `online`. A failure reports
`offline` with the reason on the console, and `stop` or `kill` during
`preparing` abandons the download and returns the server to `offline`.

### What is installed, and when it changes

Each server directory carries a `.mcpanel-install.json` record of what was
actually provisioned: core, version, the resolved build, and the Java it was
installed for. A start compares that record against the config and downloads
only when they disagree — so an ordinary restart touches no network and takes
milliseconds.

Two consequences worth knowing:

**An unpinned version does not drift.** Creating a server without naming a build
resolves the newest one *once*, then records it. Restarting never silently moves
a server onto a build published since. Taking a newer one is
`POST /servers/{id}/reinstall`, or Update in Settings.

**Only artifact-defining changes cost a download.** Core, version, pinned build
and Java version force a reinstall, and the console says which of them changed.
Name, port, memory, JVM flags and the supervision policy do not — they apply on
the next start for free. Worlds, configuration and plugins are never touched by
a reinstall; only the jar is replaced.

### Backups

A backup is a gzipped tarball of the worlds, configuration, plugins and player
data. `server.jar` and the `libraries/`, `versions/`, `cache/`, `logs/` and
`.paper/` trees are skipped, because the panel can fetch them again — a backup
that is mostly redownloadable bytes is one people stop taking.

Taking a backup of a running server flushes chunks to disk first and leaves it
online. Restoring requires the server to be stopped: unpacking a world under a
live JVM corrupts it.

### Plugins and mods

Modrinth searches are scoped to the server's flavour and Minecraft version, so
every result will actually load. Bukkit-family servers install to `plugins/`,
Fabric and Forge to `mods/`, and vanilla is refused with a suggestion rather
than a silent no-op.

Uploads default to 256 MiB. Archive extraction defaults to 10,000 entries,
1 GiB expanded output, and 256 MiB per file. Each server and its retained
backups have configurable 50 GiB quotas by default. Change these with
`--max-upload-bytes`, `--max-extracted-bytes`, `--max-archive-entries`,
`--max-extracted-file-bytes`, `--max-server-disk-bytes`, and
`--max-backup-disk-bytes`; limits apply while bytes are being written, not just
to archive metadata.

## Interface

Icons are inline SVG rather than an icon font or a sprite sheet: the frontend is
embedded in the binary and served under a strict CSP, so anything fetched from
elsewhere would not load. They inherit `currentColor`, so a control's icon and
its text always match.

Icon-only controls carry a tooltip *and* an `aria-label`. The tooltip is a
convenience for pointer users; the label is what makes the control usable on a
touchscreen, where hover does not exist, and in a screen reader.

## Platforms

Linux, macOS and Windows. `cargo check --workspace --all-targets --target
x86_64-pc-windows-gnu` is clean, and the platform-specific pieces are handled
rather than assumed:

* System statistics come from [`sysinfo`], which already abstracts Linux,
  Windows, macOS and the BSDs. No second library is needed, and none of the
  panel's own code reads `/proc` or shells out to `ps`, `du` or `df`.
* Java discovery and installation are `java-path`'s problem, and it knows about
  `java.exe` and platform-specific archive layouts.
* Shutdown handles `SIGTERM` on Unix and falls back to Ctrl-C elsewhere.
* File-manager paths reject Windows drive prefixes along with `..`, and API
  responses always use forward slashes whatever the host uses.
* Backup archives store forward-slash entry names, so an archive taken on
  Windows restores on Linux and back.

Two caveats worth knowing:

The Rust test suite runs on Windows too — the stand-in for the JVM is a small
Rust binary rather than a shell script, so process supervision is genuinely
exercised there rather than skipped. Verified by running the Windows test
binaries under Wine.

One caveat: `build.sh` is a shell script. On Windows run the two steps it wraps:
`cd web && npm install && npm run build`, then `cargo build --release`.

On Windows, the panel can enforce application-level quotas and containment but
does not yet create a per-server Windows job object or restricted OS identity.
Use a separate Windows service account, container, or job-object infrastructure
per trust domain when hard isolation is required.

[`sysinfo`]: https://crates.io/crates/sysinfo

## Mobile

Row actions live in a contextual menu rather than on hover, because a hover
target does not exist on a touchscreen. It opens three ways: the always-visible
`⋯` button, a right-click, or a long-press — the last two arrive as the same
`contextmenu` event, which is suppressed so the browser's own menu does not
appear instead.

Below 640px the menu becomes a bottom sheet with larger hit targets, tables drop
their less important columns rather than scrolling sideways, and toolbars wrap.

## Internationalisation

The UI ships in English and Spanish, picked from a stored choice, then the
browser's `Accept-Language`, then English. Switch it from the header.

`web/src/i18n/en.ts` is the source of truth. Its keys are typed, so a
translation that is missing a key — or carries one that no longer exists — is a
compile error rather than a `{missing}` in the UI. To add a language, copy
`es.ts`, translate the values, and add it to `LANGUAGES` in `i18n/index.tsx`.

## Testing

For a quick local Rust loop, use `cargo check` instead of a full build: it
type-checks without linking an executable. The workspace also provides aliases
for the common commands:

```sh
cargo dev-check                         # fast workspace check
cargo dev-clippy                        # fast workspace-only Clippy
cargo workspace-test                    # full Rust test suite
```

On a memory-constrained machine, limit Cargo's parallel jobs for a more
responsive desktop. Two jobs is a useful starting point; increase it if the
machine remains responsive:

```sh
CARGO_BUILD_JOBS=2 cargo dev-check      # Bash
$env:CARGO_BUILD_JOBS = "2"; cargo dev-check  # PowerShell
```

`MCPANEL_FAST=1 ./check.sh` runs format, `cargo check`, and workspace-only
Clippy. `MCPANEL_FAST=1 ./build.sh` builds a debuggable `target/debug/mcpanel`
without the release profile's LTO and single codegen unit. The default scripts
still run the complete CI/release workflow. On Windows, run the equivalent
Cargo commands directly because the wrapper scripts are Bash scripts.

```sh
cargo test --workspace     # 89 backend tests
cd web && npm test         # 29 frontend tests
```

`./check.sh` runs everything CI runs, in the same order.

CI runs the Rust suite on Linux **and** Windows, plus clippy as an error,
`cargo fmt --check`, the frontend tests, and a release build that then starts
the binary and asserts it is serving the embedded frontend — a build that
quietly ships the "frontend not built" page would otherwise look like a success.

The supervisor tests spawn real child processes against a stand-in for the JVM,
so the status machine, stdio pumps, graceful stop, kill fallback and restart
policy are exercised for real rather than mocked. Only the Java/jar provisioning
is stubbed — downloading a JDK is not a unit test's job.

That stand-in validates its `-jar` argument and fails with the real launcher's
message, which matters more than it sounds: an earlier version ignored its
arguments entirely, and a launch bug shipped straight past a green test suite.
When a test double is more forgiving than the real thing, it stops testing.
It is a Rust binary rather than a shell script so the same tests run on Windows,
and its behaviour is driven through `server_args` — the same path a real
server's arguments take.

The frontend tests cover the parts that have actually broken: the API client's
session handling and ticketed downloads, the contextual menu that makes rows
usable without hover, the dialogs that replaced `confirm`/`prompt`, and the two
formatting helpers. The dictionary test asserts that both languages carry the
same keys *and the same placeholders*, so a translation cannot quietly drop a
`{count}`.

## Status

Working end to end, and verified against a live Paper server: provisioning,
supervision, crash recovery, console, file manager with upload and archive
extraction, backups with restore, Modrinth plugin and mod installation,
per-server resource metrics, multi-user accounts with per-server access.
Playit account claiming and per-server TCP tunnels are available from the
admin web UI through the embedded runtime by default, or through an external
agent in explicit external mode.

Not built yet:

1. **Scheduled backups.** Backups are on demand; a cron-style schedule per
   server is the obvious next step.
2. **Hangar and CurseForge** as add-on sources alongside Modrinth.
3. **Console log persistence.** The buffer is in memory, so it resets when the
   panel restarts. The server's own `logs/` directory is untouched and remains
   the durable record.

## Releasing

```sh
git tag v0.1.0
git push origin v0.1.0
```

That builds for Linux (glibc and static), Windows and both macOS architectures,
starts each native binary to confirm it serves the embedded frontend, and
publishes them to a GitHub Release with checksums and generated notes. A target
that fails to build stops the release rather than publishing a partial one.

## Licence

MIT
