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
  operators. On macOS, use the available sandbox profile. Windows currently
  has application-level containment but no per-server kernel sandbox in this
  binary; use OS accounts, containers, or job-object infrastructure for hard
  isolation.
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

When `bwrap` or macOS sandboxing is active, Minecraft receives only its server
directory and required runtime files. Without a kernel sandbox, the application
still rejects filesystem traversal and strips panel secrets from the child
environment, but Java code runs with the panel service account's remaining OS
authority. This is a known limitation, not a tenant-isolation guarantee.

## Secrets handling

Session credentials are held in memory and browser sessions use an `HttpOnly`
cookie. Playit secrets are kept in the data directory and are never returned
by the panel API. Never include `Authorization` headers, cookies, tickets,
passwords, or Playit secrets in bug reports or logs. Rotate credentials after
any suspected exposure.
