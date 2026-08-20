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
            │   (mcpanel)   │  and the embedded web UI
            └───────────────┘
```

## Quick start

Download a binary from [Releases](https://github.com/nglmercer/minecraft-server-rs/releases)
— it is self-contained, with the web UI inside it — or build from source:

```sh
git clone --recurse-submodules https://github.com/nglmercer/minecraft-server-rs
cd minecraft-server-rs
./build.sh
./target/release/mcpanel --data-dir ./data --bind 0.0.0.0:8080
```

Two Linux builds are published: `linux-x86_64` links against glibc, and
`linux-x86_64-static` is statically linked, for Alpine or any distro older than
the one the glibc build was made on.

The first run prints a generated `admin` password. It is shown once.

## Layout

| Path                        | What it is                                            |
| --------------------------- | ----------------------------------------------------- |
| `crates/guardian`           | The lifecycle library. No HTTP, no panel concepts.    |
| `crates/panel`              | The `mcpanel` binary: API, auth, embedded frontend.   |
| `web`                       | Vite + Preact + TypeScript + Tailwind v4 frontend.    |
| `vendor/java-path-rs`       | Submodule: Java discovery and provisioning.           |
| `vendor/minecraft-core-rs`  | Submodule: server artifact resolution and download.   |

Everything the panel owns at runtime lives under `--data-dir`:

```
data/
├── panel.json          users and server records
├── jdks/               JDKs downloaded on demand, shared by all servers
└── servers/<uuid>/     one server's working directory (worlds, plugins, jar)
```

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

## API

Everything is under `/api`. Authenticate with `Authorization: Bearer <token>`;
the WebSocket takes `?token=` instead, because browsers cannot set headers on a
handshake.

| Method              | Path                                   | Purpose                            |
| ------------------- | -------------------------------------- | ---------------------------------- |
| `POST`              | `/auth/login`                          | Exchange credentials for a token   |
| `POST`              | `/auth/logout`                         | Invalidate the current token       |
| `GET`               | `/auth/me`                             | The current account                |
| `POST`              | `/auth/password`                       | Change your password               |
| `GET` `POST`        | `/servers`                             | List / create                      |
| `GET` `PATCH` `DELETE` | `/servers/{id}`                     | Inspect / reconfigure / remove     |
| `POST`              | `/servers/{id}/power`                  | `start`, `stop`, `restart`, `kill` |
| `POST`              | `/servers/{id}/command`                | Send a console command             |
| `POST`              | `/servers/{id}/reinstall`              | Re-resolve and download the artifact |
| `GET`               | `/servers/{id}/logs`                   | The retained console buffer        |
| `WS`                | `/servers/{id}/ws`                     | Live console, both directions      |
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
one backup, expires after a minute, and grants nothing else. `?token=` is
accepted on the WebSocket route alone, where there is no alternative.

### Accounts and access

Admins see and manage everything. A regular account only reaches the servers it
has been granted, across every endpoint including the console socket and the
file manager. Changing an account's password or permissions revokes its existing
sessions, so a demotion takes effect immediately rather than at the next login.

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

Not built yet:

1. **Scheduled backups.** Backups are on demand; a cron-style schedule per
   server is the obvious next step.
2. **Tunnelling for players outside the LAN.** This needs a third-party service
   and per-user credentials, which is a different kind of decision from the
   rest of the panel — worth choosing deliberately rather than defaulting to
   whatever integrates fastest.
3. **Hangar and CurseForge** as add-on sources alongside Modrinth.
4. **Console log persistence.** The buffer is in memory, so it resets when the
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
