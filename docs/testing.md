# Testing

## Quick commands

For a quick local Rust loop, use `cargo check` instead of a full build — it type-checks without linking:

```sh
cargo dev-check                         # fast workspace check (alias)
cargo dev-clippy                        # fast workspace-only Clippy (alias)
cargo workspace-test                    # full Rust test suite (alias)
```

On a memory-constrained machine:

```sh
CARGO_BUILD_JOBS=2 cargo dev-check      # Bash
$env:CARGO_BUILD_JOBS = "2"; cargo dev-check  # PowerShell
```

Fast vs full scripts:

- `MCPANEL_FAST=1 ./check.sh` — format, `cargo check`, workspace-only Clippy.
- `MCPANEL_FAST=1 ./build.sh` — debuggable `target/debug/mcpanel` without release LTO/single codegen unit.
- Default `./check.sh` / `./build.sh` — complete CI/release workflow. On Windows run Cargo commands directly.

Full suite:

```sh
cargo test --workspace     # 89 backend tests
cd web && npm test         # 29 frontend tests
```

`./check.sh` runs everything CI runs, in the same order.

## CI

CI runs the Rust suite on Linux **and** Windows, plus clippy (`-D warnings`), `cargo fmt --check`, frontend tests, and a release build that then starts the binary and asserts it is serving the embedded frontend — a build that quietly ships the "frontend not built" page would otherwise look like a success. See `.github/workflows/ci.yml`.

The `release` job needs `backend` and `frontend` to be green, exercises `build.sh` itself, and uploads the `mcpanel-linux-x86_64` artifact (14-day retention).

## Supervisor tests

The supervisor tests spawn real child processes against a stand-in for the JVM, so the status machine, stdio pumps, graceful stop, kill fallback and restart policy are exercised for real rather than mocked. Only the Java/jar provisioning is stubbed — downloading a JDK is not a unit test's job.

That stand-in validates its `-jar` argument and fails with the real launcher's message, which matters more than it sounds: an earlier version ignored its arguments entirely, and a launch bug shipped straight past a green test suite. When a test double is more forgiving than the real thing, it stops testing. It is a Rust binary rather than a shell script so the same tests run on Windows, and its behaviour is driven through `server_args` — the same path a real server's arguments take.

## Frontend tests

The frontend tests cover the parts that have actually broken: the API client's session handling and ticketed downloads, the contextual menu that makes rows usable without hover, the dialogs that replaced `confirm`/`prompt`, and the two formatting helpers. The dictionary test asserts that both languages carry the same keys *and the same placeholders*, so a translation cannot quietly drop a `{count}`. See [Internationalisation](i18n.md).

## Related

- [Development](development.md)
- [Platforms — Testing on Windows](platforms.md#testing-on-windows)
