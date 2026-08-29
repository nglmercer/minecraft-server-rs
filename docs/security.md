# Security Model

## Trust levels

The panel has four materially different trust levels:

* A **panel administrator** can manage every server, account, tunnel, and panel setting. Treat an administrator as a host administrator.
* A **server operator** can use only the server ids assigned to that account: its power controls, console, files, backups, and supported Modrinth installs. The API consistently hides unassigned servers as `404`.
* **Minecraft plugins and mods are not trusted code.** Installing one gives arbitrary Java code the authority available to that server process. A plugin can read or modify everything visible inside its process sandbox and can consume its CPU, memory, disk, process, and network budget. Do not install an untrusted plugin merely because it came from Modrinth.
* The **host administrator** owns the operating system, the panel account, its data directory, and any external services. No application can protect a server from a host administrator.

See `SECURITY.md` for the supported-versions and disclosure policy.

## File and archive hardening

Server-scoped file operations use descriptor-relative capabilities and reject path traversal, symlink parents, final symlinks, hard links, and archive link entries. Uploaded and restored archives are bounded by entry, per-file, and expanded-byte limits. Uploads, editor writes, Modrinth installs, and retained backups are quota checked before publication.

Browser download and console flows use short-lived scoped tickets rather than long-lived session tokens in URLs. See [API — Downloads](api.md#downloads--ticket-model).

## Process isolation

On Linux, `bwrap` places each Minecraft process in a separate PID/filesystem view, exposes its server directory and the selected JDK, and mounts the host runtime trees `/usr`, `/bin`, `/sbin`, `/lib`, `/lib64`, and `/etc` read-only when present. The process gets a server-local home and temporary directory; the panel data directory and other servers are not mounted. These system-tree mounts let Java resolve its runtime and certificates, but they also allow plugins to inspect files in those trees that the panel OS account can read, and network access remains available. On macOS the available `sandbox-exec` profile provides a comparable filesystem boundary. If the helper is absent, or on Windows where this binary does not currently create a kernel job/container identity, the panel refuses to start Minecraft unless the operator explicitly passes `--allow-unsandboxed-servers` or sets `MCPANEL_ALLOW_UNSANDBOXED_SERVERS=true`. With that acknowledgement, the panel still sanitizes environment variables and uses race-resistant server paths, but Java runs as the panel's OS user. Filesystem/API containment is not the same as OS tenant isolation: plugins and mods execute arbitrary JVM code. CPU, process-count, file-descriptor, and native-memory limits are not a substitute for cgroups/job objects on those platforms.

On Linux, install `bubblewrap` (`bwrap`) so Minecraft processes receive the strongest sandbox this binary can use. Do not use the explicit unsandboxed acknowledgement for mutually untrusted operators unless an external OS/container policy supplies the missing tenant boundary.

The selected JVM heap, request bodies, archive expansion, downloads, server files, and backups have finite defaults. Tune them with the `--max-*` options for the host, and use OS-level cgroups, quotas, or a container when hard host resource isolation is required.

## Secure remote deployment

The binary serves HTTP and does not terminate TLS itself. Keep it on loopback:

```sh
mcpanel --data-dir /var/lib/mcpanel --bind 127.0.0.1:8080
```

Terminate HTTPS in a reverse proxy, forward only to that loopback listener, and configure the proxy to pass WebSocket upgrades. For example, a Caddy `reverse_proxy 127.0.0.1:8080` site provides TLS and WebSocket forwarding by default. Do not expose `0.0.0.0:8080` directly. The `--allow-insecure-http` option exists for development or an explicitly isolated network and is not a production security control.

By default the panel uses the direct TCP peer for login rate limiting and does
not trust forwarded client-IP headers. When the reverse proxy is the direct
peer, configure its address explicitly, for example:

```sh
mcpanel --trusted-proxy 127.0.0.1
```

The proxy must overwrite `X-Forwarded-For`, `Forwarded`, and
`X-Forwarded-Proto` at its public boundary rather than append untrusted client
values. Never add an address that untrusted users can connect from. If trusted
proxy configuration is not possible, apply login rate limiting in the proxy as
well, because all clients will otherwise share the proxy's rate-limit bucket.

The data directory should be owned by the panel service account with mode `0700`; `panel.json` and the embedded Playit secret are written with owner-only permissions on Unix. Do not put backups or the data directory in a shared directory, and do not pass credentials through command-line arguments or logs.

On a normal panel shutdown, managed Minecraft processes are asked to save and
stop before Playit and the panel exit. Linux `bwrap --die-with-parent` remains
as a crash fallback; it should not be relied on as the normal shutdown path.

## Secrets handling

Session credentials are held in memory and browser sessions use an `HttpOnly` cookie. Playit secrets are kept in `<data-dir>/playit/secret.toml` and are never returned by the panel API. Never include `Authorization` headers, cookies, tickets, passwords, or Playit secrets in bug reports or logs. Rotate credentials after any suspected exposure.

## Resource limits

Uploads default to 256 MiB. Archive extraction defaults to 10,000 entries, 1 GiB expanded output, and 256 MiB per file. Each server and its retained backups have configurable 50 GiB quotas by default. Change these with `--max-upload-bytes`, `--max-extracted-bytes`, `--max-archive-entries`, `--max-extracted-file-bytes`, `--max-server-disk-bytes`, and `--max-backup-disk-bytes`; limits apply while bytes are being written, not just to archive metadata. See [Operations — Backups & limits](operations.md).

## Related

- [API — Accounts and access](api.md#accounts-and-access)
- [Operations](operations.md)
- [Platforms](platforms.md)
