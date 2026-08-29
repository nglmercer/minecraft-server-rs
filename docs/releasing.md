# Releasing

## Status

Working end to end, verified against a live Paper server: provisioning, supervision, crash recovery, console, file manager with upload and archive extraction, backups with restore, Modrinth plugin and mod installation, per-server resource metrics, multi-user accounts with per-server access. Playit account claiming and per-server TCP tunnels are available from the admin web UI through the embedded runtime by default, or through an external agent in explicit external mode.

Not built yet:

1. **Scheduled backups.** Backups are on demand; a cron-style schedule per server is the obvious next step.
2. **Hangar and CurseForge** as add-on sources alongside Modrinth.
3. **Console log persistence.** The buffer is in memory, so it resets when the panel restarts. The server's own `logs/` directory is untouched and remains the durable record.

## Releasing

```sh
git tag v0.1.0
git push origin v0.1.0
```

That builds for Linux (glibc and static), Windows and both macOS architectures, starts each native binary to confirm it serves the embedded frontend, and publishes them to a GitHub Release with checksums and generated notes. A target that fails to build stops the release rather than publishing a partial one.

The release workflow verifies the built binary actually serves the compiled frontend (checks for `assets/index-` in the served page); a release that quietly ships the "frontend not built" stub is treated as a failure. See `.github/workflows/` and [Testing](testing.md).

## Licence

MIT — see `LICENSE`.

## Related

- [Getting Started](getting-started.md)
- [Architecture](architecture.md)
- [Security](security.md)
