#!/usr/bin/env bash
# Build MCPB bundles:
#   server.mcpb   — spec-valid bundle (Claude Desktop)
#   smithery.mcpb — same bundle, manifest tools carry inputSchema (Smithery requires it)
set -euo pipefail
cd "$(dirname "$0")/.."

npm run build
stage="$(mktemp -d)"
trap 'rm -rf "$stage"' EXIT

cp -R dist package.json package-lock.json manifest.json README.md "$stage/"
(cd "$stage" && npm ci --omit=dev --ignore-scripts --silent)
npx -y @anthropic-ai/mcpb@2 validate "$stage/manifest.json"
npx -y @anthropic-ai/mcpb@2 pack "$stage" server.mcpb

cp server.mcpb smithery.mcpb
node scripts/inject-tools.mjs "$stage"
(cd "$stage" && zip -q "$OLDPWD/smithery.mcpb" manifest.json)
