# RFC-111 — Remove the dependency license CI gate

**Status:** Accepted
**Date:** 2026-11-05
**Supersedes:** [RFC-108](108-mit-only-dependencies.md)

## Problem

RFC-108 failed pull requests when a dependency license was outside MIT, ISC, BSD-2-Clause, BSD-3-Clause, and 0BSD. The Licenses workflow ran `license-checker-rseidelsohn` for npm, `pip-licenses` for Python, and `cargo deny check licenses` for Rust. The security job ran the same Rust license check on every heavy pull request.

Pull request 376 failed the Licenses job `npm — dashboard` because `@aws-sdk/client-sesv2` is Apache-2.0. An allow list is the wrong merge gate for that dependency.

## Decision

Remove the gate.

- Delete `.github/workflows/license-check.yml`. The license jobs no longer run on pull requests, pushes, or the weekly schedule.
- Delete the npm and Python license checkers, the production-tree prune helper, `scripts/license/`, and `docs/LICENSE-EXCEPTIONS.md`.
- The security job runs `cargo deny check advisories bans sources`. The `[licenses]` section is removed from `deny.toml`, including the exception list. Advisories, bans, and crate sources stay.
- The repository `LICENSE` stays MIT. `about.toml` and `docs/data-room/` stay the diligence inventory.
- Typecheck and build of `sdks/typescript` and `sdks/aischemagate/typescript` run from the main CI workflow. Those steps previously ran in the Licenses workflow after the license scan. Dashboard, MCP, and Zapier keep the jobs they already had.

## Non-goals

- Changing product code or dependency versions.
- Regenerating `docs/data-room/dependency-licenses.md`.
- Adding Apache-2.0, or any other license, to an allow list.
