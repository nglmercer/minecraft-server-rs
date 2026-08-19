# minecraft-server-rs

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

```sh
git clone --recurse-submodules https://github.com/nglmercer/minecraft-server-rs
cd minecraft-server-rs
./build.sh
./target/release/mcpanel --data-dir ./data --bind 0.0.0.0:8080
```

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
| `GET`               | `/servers/{id}/logs`                   | The retained console buffer        |
| `WS`                | `/servers/{id}/ws`                     | Live console, both directions      |
| `GET` `PUT` `DELETE`| `/servers/{id}/files`                  | List / write / delete              |
| `GET`               | `/servers/{id}/files/read`             | Read a text file                   |
| `GET`               | `/servers/{id}/files/download`         | Stream any file out                |
| `POST`              | `/servers/{id}/files/upload`           | Multipart upload into a directory  |
| `POST`              | `/servers/{id}/files/extract`          | Unpack a `.zip`/`.jar`/`.tar.gz`   |
| `POST`              | `/servers/{id}/files/rename`           | Rename or move                     |
| `POST`              | `/servers/{id}/files/mkdir`            | Create a directory                 |
| `GET` `POST`        | `/servers/{id}/backups`                | List / take a backup               |
| `DELETE`            | `/servers/{id}/backups/{backup}`       | Delete a backup                    |
| `POST`              | `/servers/{id}/backups/{backup}/restore` | Restore (server must be stopped) |
| `GET`               | `/servers/{id}/backups/{backup}/download` | Stream the archive out          |
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

### Accounts and access

Admins see and manage everything. A regular account only reaches the servers it
has been granted, across every endpoint including the console socket and the
file manager. Changing an account's password or permissions revokes its existing
sessions, so a demotion takes effect immediately rather than at the next login.

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

## Testing

```sh
cargo test --workspace
```

51 tests. The supervisor ones spawn real child processes against a shell script
standing in for the JVM, so the status machine, stdio pumps, graceful stop, kill
fallback and restart policy are exercised for real rather than mocked. Only the
Java/jar provisioning is stubbed — downloading a JDK is not a unit test's job.

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

## Licence

MIT
