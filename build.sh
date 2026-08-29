#!/usr/bin/env bash
# Build the panel into a single self-contained binary.
#
# The frontend must be built first: `rust-embed` bakes web/dist into the
# executable at compile time, so a stale dist means a stale UI.
set -euo pipefail

cd "$(dirname "$0")"

echo "==> building frontend"
(cd web && npm ci --silent && npm run build)

if [[ ${MCPANEL_FAST:-0} == 1 ]]; then
  echo "==> building panel (debug profile)"
  cargo build --locked -p panel
  binary="target/debug/mcpanel"
else
  echo "==> building panel (release profile)"
  cargo build --release --locked -p panel
  binary="target/release/mcpanel"
fi

echo
echo "Built ${binary} ($(du -h "${binary}" | cut -f1))"
echo "Run it with: ${binary} --data-dir ./data --bind 127.0.0.1:8080"
echo "For remote access, use an HTTPS reverse proxy in front of the loopback listener."
