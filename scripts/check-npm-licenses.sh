#!/usr/bin/env bash
# Install production dependencies in one npm directory and require an allowed license.
# Allowed: MIT, ISC, BSD-2-Clause, BSD-3-Clause, 0BSD.
# --omit=dev skips dev and test packages. --omit=peer skips optional peers
# (Playwright on Next, TypeScript on valibot). Named packages are excluded.
set -euo pipefail

dir="${1:?npm directory, for example mcp}"
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root/$dir"

case "$dir" in
  sdks/aischemagate/typescript)
    # Release publish installs this package without a committed lockfile.
    # A temporary lock is only there so the peer prune can read the tree.
    npm install --omit=dev --omit=peer --no-audit --no-fund
    ;;
  dashboard)
    npm ci --omit=dev --omit=peer --legacy-peer-deps
    ;;
  *)
    npm ci --omit=dev --omit=peer
    ;;
esac

node "$root/scripts/prune-npm-license-tree.mjs"
if [ "$dir" = "sdks/aischemagate/typescript" ]; then
  rm -f package-lock.json
fi

excl="$(grep -v '^[[:space:]]*#' "$root/scripts/license/npm-exclude.txt" | grep -v '^[[:space:]]*$' | paste -sd ';' -)"

npx --yes license-checker-rseidelsohn@5.0.1 \
  --production \
  --excludePrivatePackages \
  --onlyAllow 'MIT;ISC;BSD-2-Clause;BSD-3-Clause;0BSD' \
  --excludePackages "$excl"
