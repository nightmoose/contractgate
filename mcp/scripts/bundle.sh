#!/usr/bin/env bash
# Build server.mcpb (MCPB bundle) for Smithery and Claude Desktop.
set -euo pipefail
cd "$(dirname "$0")/.."

npm run build
stage="$(mktemp -d)"
trap 'rm -rf "$stage"' EXIT

cp -R dist package.json package-lock.json manifest.json README.md "$stage/"
(cd "$stage" && npm ci --omit=dev --ignore-scripts --silent)
npx -y @anthropic-ai/mcpb@2 validate "$stage/manifest.json"
npx -y @anthropic-ai/mcpb@2 pack "$stage" server.mcpb
