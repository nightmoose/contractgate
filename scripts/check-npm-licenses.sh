#!/usr/bin/env bash
# Install production dependencies in one npm directory and require MIT.
# ISC, BSD, and 0BSD are not allow-listed. Named packages are excluded.
set -euo pipefail

dir="${1:?npm directory, for example mcp}"
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root/$dir"

case "$dir" in
  sdks/aischemagate/typescript)
    # Release publish installs this package without a lockfile. Match that.
    npm install --omit=dev --no-package-lock --no-audit --no-fund
    ;;
  *)
    npm ci --omit=dev
    ;;
esac

excl="$(grep -v '^[[:space:]]*#' "$root/scripts/license/npm-exclude.txt" | grep -v '^[[:space:]]*$' | paste -sd ';' -)"

npx --yes license-checker-rseidelsohn@5.0.1 \
  --production \
  --excludePrivatePackages \
  --onlyAllow 'MIT' \
  --excludePackages "$excl"
