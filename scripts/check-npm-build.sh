#!/usr/bin/env bash
# Typecheck and build one TypeScript SDK package.
# Dashboard, MCP, and Zapier are covered by their own CI jobs.
set -euo pipefail

dir="${1:?npm directory: sdks/typescript or sdks/aischemagate/typescript}"
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root/$dir"

case "$dir" in
  sdks/typescript)
    npm install --no-audit --no-fund
    npm run typecheck
    npm run build
    ;;
  sdks/aischemagate/typescript)
    npm install --no-package-lock --no-audit --no-fund
    # The alias calls drop-mode fields on the in-repo client. The copy on npm
    # can lag those fields, so typecheck the workspace build instead.
    (
      cd "$root/sdks/typescript"
      npm ci --no-audit --no-fund
      npm run build
    )
    npm install --no-save --no-package-lock --no-audit --no-fund "$root/sdks/typescript"
    npm run typecheck
    npm run build
    ;;
  *)
    echo "unknown npm directory: $dir" >&2
    exit 1
    ;;
esac
