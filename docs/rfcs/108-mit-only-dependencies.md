# RFC-108 — MIT-only dependency policy

**Status:** Accepted
**Date:** 2026-10-30
**Depends on:** RFC-090 (MCP server)

## Problem

DataContractGate and AISchemaGate pull third-party code through Cargo, npm, and pip. The previous `deny.toml` allow list included Apache-2.0, BSD, ISC, Unicode, MPL, and several other permissive licenses. The MCP server depended on `@modelcontextprotocol/server`. Releases from 2.3.0 of that package, and of `@modelcontextprotocol/core`, are Apache-2.0.

## Decision

The allow list is MIT, ISC, BSD-2-Clause, BSD-3-Clause, and 0BSD.

- Rust: `deny.toml` `[licenses] allow` is those five SPDX identifiers. A crate whose SPDX expression is `MIT OR …` is accepted, because MIT can be selected. `MIT OR Apache-2.0` is not an allow-list entry, and neither is `Apache-2.0`. Every crate that still cannot be used under an allowed license has a `[[licenses.exceptions]]` entry.
- npm: `license-checker-rseidelsohn --production --onlyAllow 'MIT;ISC;BSD-2-Clause;BSD-3-Clause;0BSD'` in `mcp`, `dashboard`, `zapier`, `sdks/typescript`, `sdks/aischemagate/typescript`, and `ops/dogfood`. Each other package is named in `scripts/license/npm-exclude.txt`.
- Python: `pip-licenses --allow-only` for the root hooks package, `sdks/python`, and `sdks/aischemagate/python` (required dependencies). The allow list uses the classifier names `MIT License`, `BSD License`, `ISC License (ISCL)`, and `Zero-Clause BSD (0BSD)`, plus `MIT`, `BSD-2-Clause`, `BSD-3-Clause`, `ISC`, and `0BSD`, because a distribution that sets `License-Expression` is reported as that SPDX string. Remaining distributions are named in `scripts/license/python-ignore.txt`.

The review record is [`docs/LICENSE-EXCEPTIONS.md`](../LICENSE-EXCEPTIONS.md). `about.toml` stays the wider diligence inventory. It is not the CI allow list.

The MCP server is `tmcp` plus `@tmcp/transport-stdio` and `@tmcp/adapter-zod`, all MIT. Zod 4 stays. Tool names, prompt name, schemas, and gateway behavior are unchanged.

## CI

`.github/workflows/license-check.yml` runs on pull requests that touch a manifest or lockfile, and on a weekly schedule. The existing security job still runs `cargo deny check licenses`, so the Rust rule also applies to other pull requests.

## Non-goals

- Replacing the excepted crates and packages in this change.
- Scanning optional Python extras (`langchain`, `llamaindex`, `instructor`).
- Regenerating `docs/data-room/dependency-licenses.md`.
