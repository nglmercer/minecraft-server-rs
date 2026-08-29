# Development

## Backend + frontend side by side

```sh
cargo run -p panel -- --data-dir ./data --bind 127.0.0.1:8080 --dev-cors
cd web && npm run dev     # http://localhost:5173, proxies /api to :8080
```

`--dev-cors` enables a permissive CORS policy for the Vite dev server. Do not use it in production.

## Fast checks

For a quick local Rust loop use `cargo check` (type-checks without linking):

```sh
cargo dev-check                         # fast workspace check (alias)
cargo dev-clippy                        # fast workspace-only Clippy (alias)
cargo workspace-test                    # full Rust test suite (alias)
```

On a memory-constrained machine limit Cargo parallelism:

```sh
CARGO_BUILD_JOBS=2 cargo dev-check      # Bash
$env:CARGO_BUILD_JOBS = "2"; cargo dev-check  # PowerShell
```

## Scripts

- `MCPANEL_FAST=1 ./check.sh` — format, `cargo check`, workspace-only Clippy (fast).
- `MCPANEL_FAST=1 ./build.sh` — builds `target/debug/mcpanel` without release LTO / single codegen unit.
- Default `./check.sh` / `./build.sh` run the complete CI/release workflow.

On Windows the wrapper scripts are Bash; run the equivalent Cargo/npm commands directly. See [Platforms](platforms.md#windows).

## Frontend workspace

```sh
cd web
npm install
npm run dev    # Vite dev server
npm test       # 29 frontend tests
npm run build  # also type-checks via tsc
```

See [Testing](testing.md) for the full suite and [Frontend](frontend.md) for UI conventions.
