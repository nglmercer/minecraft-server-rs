#!/usr/bin/env bash
# Build the panel into a single self-contained binary.
#
# The frontend must be built first: `rust-embed` bakes web/dist into the
# executable at compile time, so a stale dist means a stale UI.
set -euo pipefail

cd "$(dirname "$0")"

echo "==> building frontend"
(cd web && npm install --silent && npm run build)

echo "==> building panel"
cargo build --release -p panel

echo
echo "Built target/release/mcpanel ($(du -h target/release/mcpanel | cut -f1))"
echo "Run it with: ./target/release/mcpanel --data-dir ./data --bind 0.0.0.0:8080"
