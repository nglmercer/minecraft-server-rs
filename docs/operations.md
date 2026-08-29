# Operations

## Starting a server

`POST /servers/{id}/power` with `start` returns as soon as the work is accepted, not when the server is up. Provisioning downloads a JDK and a server jar and can take minutes, so it runs detached from the request: a browser that navigates away mid-download cannot strand the server in `preparing`.

Watch the WebSocket for `starting` and then `online`. A failure reports `offline` with the reason on the console, and `stop` or `kill` during `preparing` abandons the download and returns the server to `offline`.

See [API](api.md) for `power`, `command`, `logs`, and `ws`.

## What is installed, and when it changes

Each server directory carries a `.mcpanel-install.json` record of what was actually provisioned: core, version, the resolved build, and the Java it was installed for. A start compares that record against the config and downloads only when they disagree — so an ordinary restart touches no network and takes milliseconds.

Two consequences worth knowing:

**An unpinned version does not drift.** Creating a server without naming a build resolves the newest one *once*, then records it. Restarting never silently moves a server onto a build published since. Taking a newer one is `POST /servers/{id}/reinstall`, or Update in Settings.

**Only artifact-defining changes cost a download.** Core, version, pinned build and Java version force a reinstall, and the console says which of them changed. Name, port, memory, JVM flags and the supervision policy do not — they apply on the next start for free. Worlds, configuration and plugins are never touched by a reinstall; only the jar is replaced.

## Backups

A backup is a gzipped tarball of the worlds, configuration, plugins and player data. `server.jar` and the `libraries/`, `versions/`, `cache/`, `logs/` and `.paper/` trees are skipped, because the panel can fetch them again — a backup that is mostly redownloadable bytes is one people stop taking.

Taking a backup of a running server flushes chunks to disk first and leaves it online. Restoring requires the server to be stopped: unpacking a world under a live JVM corrupts it.

Relevant endpoints: `GET`/`POST /servers/{id}/backups`, `DELETE /servers/{id}/backups/{backup}`, `POST .../restore`, ticketed `.../download`. See [API](api.md).

## Plugins and mods

Modrinth searches are scoped to the server's flavour and Minecraft version, so every result will actually load. Bukkit-family servers install to `plugins/`, Fabric and Forge to `mods/`, and vanilla is refused with a suggestion rather than a silent no-op.

See `GET /servers/{id}/mods`, `GET .../mods/search`, `POST .../mods/install`.

## Resource limits

Uploads default to 256 MiB. Archive extraction defaults to 10,000 entries, 1 GiB expanded output, and 256 MiB per file. Each server and its retained backups have configurable 50 GiB quotas by default. Change these with:

- `--max-upload-bytes`
- `--max-extracted-bytes`
- `--max-archive-entries`
- `--max-extracted-file-bytes`
- `--max-server-disk-bytes`
- `--max-backup-disk-bytes`

Limits apply while bytes are being written, not just to archive metadata. The selected JVM heap, request bodies, archive expansion, downloads, server files, and backups have finite defaults — tune them for the host, and use OS-level cgroups, quotas, or a container when hard host isolation is required. See [Security](security.md).
