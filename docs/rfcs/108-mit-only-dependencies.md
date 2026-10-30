# RFC-108 — MIT-only dependency policy

**Status:** Accepted
**Date:** 2026-10-30
**Depends on:** RFC-090 (MCP server)

## Problem

DataContractGate and AISchemaGate pull third-party code through Cargo, npm, and pip. The previous `deny.toml` allow list included Apache-2.0, BSD, ISC, Unicode, MPL, and several other permissive licenses. The MCP server depended on `@modelcontextprotocol/server`. Releases from 2.3.0 of that package, and of `@modelcontextprotocol/core`, are Apache-2.0.

## Decision

The enforced license is MIT.

- Rust: `deny.toml` `[licenses] allow` is `MIT` only. A crate whose SPDX expression is `MIT OR …` is accepted, because MIT can be selected. `MIT OR Apache-2.0` is not an allow-list entry, and neither is `Apache-2.0`. Every crate that cannot be used under MIT alone has a `[[licenses.exceptions]]` entry.
- npm: `license-checker-rseidelsohn --production --onlyAllow MIT` in `mcp`, `dashboard`, `zapier`, `sdks/typescript`, `sdks/aischemagate/typescript`, and `ops/dogfood`. ISC, BSD-2-Clause, BSD-3-Clause, and 0BSD are not added to `--onlyAllow`. Each non-MIT package is named in `scripts/license/npm-exclude.txt`.
- Python: `pip-licenses --allow-only 'MIT;MIT License'` for the root hooks package, `sdks/python`, and `sdks/aischemagate/python` (required dependencies). The second string is the MIT classifier text the tool prints for some MIT distributions. Non-MIT distributions are named in `scripts/license/python-ignore.txt`.

The review record is [`docs/LICENSE-EXCEPTIONS.md`](../LICENSE-EXCEPTIONS.md). `about.toml` stays the wider diligence inventory. It is not the CI allow list.

The MCP server is `tmcp` plus `@tmcp/transport-stdio` and `@tmcp/adapter-zod`, all MIT. Zod 4 stays. Tool names, prompt name, schemas, and gateway behavior are unchanged.

## CI

`.github/workflows/license-check.yml` runs on pull requests that touch a manifest or lockfile, and on a weekly schedule. The existing security job still runs `cargo deny check licenses`, so the Rust rule also applies to other pull requests.

## Non-goals

- Replacing the excepted crates and packages in this change.
- Scanning optional Python extras (`langchain`, `llamaindex`, `instructor`).
- Regenerating `docs/data-room/dependency-licenses.md`.
