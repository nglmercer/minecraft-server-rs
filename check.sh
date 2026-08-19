#!/usr/bin/env bash
# Run everything CI runs, locally.
#
# Useful on its own, and necessary while GitHub Actions is unavailable: without
# it the only thing standing between a mistake and `main` is remembering to run
# five commands in the right order.
set -euo pipefail

cd "$(dirname "$0")"

step() { printf '\n\033[1;36m==> %s\033[0m\n' "$1"; }

step "Format"
cargo fmt --all --check

step "Clippy"
cargo clippy --workspace --all-targets -- -D warnings

step "Rust tests"
cargo test --workspace

step "Frontend tests"
(cd web && npm test)

step "Frontend build"
(cd web && npm run build)

step "Release build"
cargo build --release -p panel

step "Embedded frontend"
# A release binary that quietly serves the "frontend not built" page would
# otherwise look like a successful build.
data=$(mktemp -d)
./target/release/mcpanel --data-dir "$data" --bind 127.0.0.1:8099 >/dev/null 2>&1 &
panel=$!
trap 'kill $panel 2>/dev/null || true; rm -rf "$data"' EXIT

for _ in $(seq 1 30); do
  curl -sf http://127.0.0.1:8099/ >/dev/null && break
  sleep 1
done

if curl -s http://127.0.0.1:8099/ | grep -q 'assets/index-'; then
  echo "the release binary is serving the embedded frontend"
else
  echo "ERROR: the release binary is not serving the built frontend" >&2
  exit 1
fi

printf '\n\033[1;32mAll checks passed.\033[0m\n'
