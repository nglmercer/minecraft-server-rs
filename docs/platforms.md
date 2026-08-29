# Platforms

Linux, macOS and Windows. `cargo check --workspace --all-targets --target x86_64-pc-windows-gnu` is clean, and the platform-specific pieces are handled rather than assumed:

* System statistics come from [`sysinfo`](https://crates.io/crates/sysinfo), which already abstracts Linux, Windows, macOS and the BSDs. No second library is needed, and none of the panel's own code reads `/proc` or shells out to `ps`, `du` or `df`.
* Java discovery and installation are `java-path`'s problem, and it knows about `java.exe` and platform-specific archive layouts.
* Shutdown handles `SIGTERM` on Unix and falls back to Ctrl-C elsewhere.
* File-manager paths reject Windows drive prefixes along with `..`, and API responses always use forward slashes whatever the host uses.
* Backup archives store forward-slash entry names, so an archive taken on Windows restores on Linux and back.

## Testing on Windows

The Rust test suite runs on Windows too — the stand-in for the JVM is a small Rust binary rather than a shell script, so process supervision is genuinely exercised there rather than skipped. Verified by running the Windows test binaries under Wine. See [Testing](testing.md).

## Build notes

`build.sh` is a shell script. On Windows run the two steps it wraps:

```sh
cd web && npm install && npm run build
cargo build --release
```

Two Linux builds are published: `linux-x86_64` (glibc) and `linux-x86_64-static` (static) for Alpine or older distros. See [Releasing](releasing.md) and [Getting Started](getting-started.md).

## Sandbox per OS

- **Linux:** `bwrap` (bubblewrap) gives the strongest isolation — separate PID/filesystem view, read-only JDK, server-local home/tmp. Install `bubblewrap` when hosting untrusted operators.
- **macOS:** `sandbox-exec` provides a comparable filesystem boundary.
- **Windows:** application-level quotas and containment, but no per-server Windows job object or restricted OS identity in this binary yet. Use a separate Windows service account, container, or job-object infrastructure per trust domain when hard isolation is required. See [Security](security.md).

CPU, process-count, file-descriptor, and native-memory limits are not a substitute for cgroups/job objects on those platforms.
