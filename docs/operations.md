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

**Edits to a running server are pending, not live.** The JVM keeps the heap and
port it was launched with until it is restarted, so the panel keeps reserving
those values: the aggregate memory budget counts the larger of the stored and
running heap, and both the stored and running port stay unavailable to other
servers. The server view reports this as `pending_restart`, and the settings
page says so, until the process is restarted.

## Backups

A backup is a gzipped tarball of the worlds, configuration, plugins and player data. `server.jar` and the `libraries/`, `versions/`, `cache/`, `logs/` and `.paper/` trees are skipped, because the panel can fetch them again — a backup that is mostly redownloadable bytes is one people stop taking.

Taking a backup of a running server flushes chunks to disk first and leaves it online. Restoring requires the server to be stopped: unpacking a world under a live JVM corrupts it.

Restore is a whole-archive transaction. Every entry is expanded into a staging
directory inside the server folder and checked against the archive and
server-disk quotas first; only then are the staged files published. If any step
fails, the files already published are rolled back from the originals they
displaced, so the server tree is left as it was rather than as a mixture of old
and restored files. Publishing needs the old and new copy of a file to coexist
briefly, so a restore's transient disk peak is higher than the tree it produces.

A restore from a remote provider streams the archive into staging under a byte
counter bounded by the recorded backup size and `--max-backup-archive-bytes`,
and verifies the recorded SHA-256 before anything touches the server directory.
Staging artifacts are removed on every exit path.

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
- `--max-backup-archive-bytes` (compressed bytes accepted from one restore
  download, 8 GiB by default)

Limits apply while bytes are being written, not just to archive metadata. The selected JVM heap, request bodies, archive expansion, downloads, server files, and backups have finite defaults — tune them for the host, and use OS-level cgroups, quotas, or a container when hard host isolation is required. See [Security](security.md).
