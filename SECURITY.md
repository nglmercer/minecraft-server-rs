# Security policy

## Supported versions

Only the latest commit on `main` and the latest published release are
supported for security fixes. Upgrade before reporting a finding when
possible.

## Reporting a vulnerability

Please report security issues privately through the GitHub Security Advisories
“Report a vulnerability” flow for `nglmercer/minecraft-server-rs`. Include the
affected commit or release, a minimal reproduction, impact, and any proposed
mitigation. Do not open a public issue for an undisclosed vulnerability. The
maintainer will acknowledge the report, coordinate a fix and disclosure
timeline, and credit reporters who want attribution.

## Deployment requirements

* Keep the listener on loopback and terminate remote access with HTTPS in a
  reverse proxy. Direct non-loopback plaintext HTTP requires the explicit
  `--allow-insecure-http` acknowledgement and is suitable only for an isolated
  network.
* Run the panel as a dedicated low-privilege service account. Protect its data
  directory (`0700` on Unix), `panel.json`, Playit credentials, backups, and
  service environment from other users.
* Install Linux `bubblewrap` (`bwrap`) when using separate untrusted server
  operators. On macOS, use the available `sandbox-exec` helper. If the
  configured platform helper is unavailable, the panel refuses to start a
  Minecraft JVM unless the operator explicitly passes
  `--allow-unsandboxed-servers` or sets
  `MCPANEL_ALLOW_UNSANDBOXED_SERVERS=true`. Windows currently has no
  per-server kernel sandbox in this binary, so the same acknowledgement is
  required there; use OS accounts, containers, or job-object infrastructure
  for hard isolation.
* Set upload, extraction, server-disk, backup-disk, download, and heap limits
  appropriate to the host. Use cgroups, filesystem quotas, or containers for
  hard CPU, RAM, process-count, descriptor, and native-memory limits.

## Trust and plugin warning

Panel administrators are equivalent to host administrators. A server operator
is restricted by the API to assigned server ids, but a Minecraft plugin or mod
is arbitrary JVM code. Installing one is equivalent to executing code supplied
by that plugin author. Review and pin add-ons, and do not treat a Modrinth
listing or checksum as a security review.

## Containment guarantees and limitations

Server file APIs use capability-based, descriptor-relative operations and do
not follow escaping symlinks. Archive extraction rejects symlinks, hard links,
special files, traversal, and unbounded expansion. Download and console
browser flows use short-lived scoped tickets rather than long-lived session
tokens in URLs.

When `bwrap` is active, Minecraft receives its server directory and required
JDK, along with read-only host runtime trees (`/usr`, `/bin`, `/sbin`, `/lib`,
`/lib64`, and `/etc` when present). Plugins can inspect files in those trees
that the panel account can read, and network access is available. macOS uses
its available `sandbox-exec` profile. Filesystem/API containment is not the
same as OS tenant isolation: plugins and mods execute arbitrary JVM code. If
unsandboxed execution is explicitly acknowledged, Java code runs with the
panel service account's remaining OS authority. CPU, process, memory, and
native-resource limits require OS-level controls such as cgroups, containers,
or job objects.

On a normal panel shutdown, managed Minecraft processes are asked to save and
stop before Playit and the panel exit. Linux `bwrap --die-with-parent` is a
crash fallback, not the normal lifecycle policy.

When deploying behind a reverse proxy, pass its listener address with
`--trusted-proxy` (or `MCPANEL_TRUSTED_PROXIES`) only if the proxy overwrites
the `X-Forwarded-For`, `Forwarded`, and `X-Forwarded-Proto` headers. Without
that configuration all clients share the proxy's login rate-limit bucket;
never trust forwarded headers from an untrusted peer.

## Secrets handling

Session credentials are held in memory and browser sessions use an `HttpOnly`
cookie. Playit secrets are kept in the data directory and are never returned
by the panel API. Never include `Authorization` headers, cookies, tickets,
passwords, or Playit secrets in bug reports or logs. Rotate credentials after
any suspected exposure.

## Maintainer repository settings

Application code cannot enforce GitHub branch rules. Protect `main` in the
repository settings or rulesets with these requirements:

* require a pull request before merging;
* require the backend checks `Rust (ubuntu-latest)` and `Rust
  (windows-latest)`, plus the `Frontend` check;
* require the branch to be up to date before merging;
* require conversation resolution;
* disallow force pushes; and
* disallow branch deletion.

Keep the required checks aligned with the jobs defined in the repository's CI
workflow; do not add a source-code substitute for these GitHub controls.
