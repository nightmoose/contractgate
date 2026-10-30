#!/usr/bin/env bash
# Typecheck and build one npm directory after the license install.
# tsc and test-only tools are devDependencies, so this step installs them.
# The license scan has already run against the tree that omits dev and peers.
set -euo pipefail

dir="${1:?npm directory, for example mcp}"
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root/$dir"

case "$dir" in
  mcp|sdks/typescript)
    npm install --no-audit --no-fund
    npm run typecheck
    npm run build
    ;;
  sdks/aischemagate/typescript)
    npm install --no-package-lock --no-audit --no-fund
    npm run typecheck
    npm run build
    ;;
  dashboard)
    npm install --legacy-peer-deps --no-audit --no-fund
    npm run type-check
    npm run build
    ;;
  zapier)
    npm install --no-audit --no-fund
    npm test
    ;;
  ops/dogfood)
    # No production package and no typecheck. The omit install is the check.
    ;;
  *)
    echo "unknown npm directory: $dir" >&2
    exit 1
    ;;
esac
