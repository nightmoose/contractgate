# Project Status — ContractGate

**As of:** 2026-10-07  
**GitHub:** https://github.com/nightmoose/contractgate (**public**)  
**Local:** `~/contractgate`  
**RFC index:** [`docs/STATUS.md`](docs/STATUS.md)

## What this is

Semantic contract enforcement at ingestion and egress (patent pending). Rust
gateway + Next.js dashboard + Python and TypeScript SDKs + MCP server + Kafka
Connect SMT. Hosted API on Fly.io (`contractgate-api`); dashboard on Vercel
(`app.datacontractgate.com`); marketing site is the separate
`datacontractgate_website` repo.

## Current state

- **v0.2.0 released 2026-10-01** on every channel: GitHub Release (CLI for 4
  targets), PyPI `contractgate`, npm `@nightmoose/contractgate-sdk` and
  `@nightmoose/contractgate-mcp-server`, MCP Registry
  `io.github.nightmoose/contractgate`. The old `@contractgate/mcp-server` npm
  package is deprecated and points at the new name.
- **Release pipeline** (`.github/workflows/release.yml`) is idempotent: retry a
  partial release with `gh workflow run release.yml -f tag=vX.Y.Z`; anything
  already published is skipped.
- **Signup and Stripe checkout** confirmed working end to end (2026-10-07).
- **Users:** none yet. Production has 11 auth users (all internal/test).
- **CI** runs on `ubuntu-24.04` (pinned ahead of the 2026-10-19
  `ubuntu-latest` → Ubuntu 26 move) with Node 24 action majors.

## Known data hygiene issues (prod)

- `public.early_access` (~500 rows) is almost entirely bot spam: one-word
  names, ~10-char random messages, 91% default `stack=kafka`, up to 91/day.
  The marketing-site form posts straight to Supabase REST with no bot check.
  Fix before any launch traffic.
- `public.orgs` has ~356 orgs with no members — orgs auto-provisioned by
  `handle_new_user` for accounts that were later deleted (memberships cascade,
  orgs do not). Last one 2026-09-08. Safe to prune after a review.

## Releasing

Bump the version in `Cargo.toml`, `sdks/python/pyproject.toml`,
`mcp/package.json` + `mcp/server.json`, `sdks/typescript/package.json`, and
`dashboard/package.json` (the sidebar reads it), then tag `vX.Y.Z`. One-time
registry setup (npm Trusted Publisher, PyPI publisher) is documented at the
top of each publish job in `release.yml`.

## Notes for humans and AIs

Do not commit `dashboard/node_modules`, `target/`, real API keys, or business
documents (grant pitches, pricing strategy) — the repo is public.
`docs/STATUS.md` is the RFC ledger, not day-to-day ops — this file is ops.
iOS status app is a **separate repo**: `contractgate-status-ios`.
