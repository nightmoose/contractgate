---
title: Open the Quarantine listing to Free; keep bodies/replay content-gated
status: Accepted
author: product owner
date: 2026-10-12
---

# RFC-099 — Quarantine listing visible on Free (cluster 4 of 6)

## Problem

The 2026-10-12 E2E test (B6) reported: "Quarantine tab Pro-locked on free,
failed events not listable at all." The test brief, by contrast, says a
failed event "appears in Quarantine (payload body is not stored on Free;
that is expected)."

Current behavior:

- `dashboard/app/contracts/page.tsx:2113-2127` wraps the entire Quarantine
  tab in `<PlanGate minTier="growth" feature="Quarantine & Replay">`. Any
  Free account hitting the tab sees only the SimpleUpsell lock card and
  cannot see the list of their own failed events.
- `dashboard/app/contracts/_tabs/quarantine.tsx:488-493` already handles
  per-event body visibility correctly: when `event.payload_redacted` is
  true (always the case on Free per RFC-086), the Replay button is
  replaced with a 🚫 tooltip ("Body not stored — Replay is unavailable
  until storage is enabled"). The Payload drawer shows the body as
  redacted.

So the content-level gating is already right. The outer PlanGate is
redundant and lies to Free customers about their own failed events — a
Free user can see the Audit Log row for the failure (B6 Audit Log: PASS)
but can't see the same event in Quarantine, which is confusing.

## Decision

Remove the `<PlanGate>` wrapper around the Quarantine tab render. Keep
all per-event content gating (`payload_redacted` → disabled replay +
redacted drawer) exactly as it is. Update the subtitle copy so Free users
understand why their events show 🚫 instead of a Replay button, without a
hard paywall.

### Backend safety

Verified `src/replay.rs:212`: when a replay batch includes an event with
`payload_redacted || payload.is_none()`, the batch processor marks it
`ReplayItemOutcome::Redacted` and skips validation. Even a crafted
request from a Free account cannot replay a redacted event. No backend
change needed.

### What stays Pro-only

- Visual Builder (another PlanGate, unchanged).
- CSV → YAML inference (another PlanGate, unchanged).
- Generate from Sample for DCG (unchanged; stays Pro-only for DCG, open
  for `aisg_free`).

## Non-goals

- B1 ("Generate from Sample locked"). Downstream of A7 (migration 036
  not applied). `page.tsx:2100` correctly uses
  `<PlanGate minTier={aisg ? "aisg_free" : "growth"}>` — the test
  account's plan was `free`, not `aisg_free`, so even the AISG branch
  didn't help.
- B2 ("Kafka/Kinesis tabs locked, not hidden"). Also downstream of A7.
  `page.tsx:404-432` already renders those tabs inside
  `{!aisgModal && (...)}` — AISG accounts never see them. Resolves once
  the account's plan is actually `aisg_free`.

Both B1 and B2 are already correctly implemented in cbb0c0c; they fail
in the test only because the test account was never migrated. The
operational fix is applying migration 036 (RFC-097 §3), not a code
change.

## Rollout

- Files: `dashboard/app/contracts/page.tsx` only. ~15 lines changed.
- Verification: `npm run build` under `dashboard/`.
- Manual: sign in to a Free account, visit `/contracts/<id>` → Quarantine
  tab → confirm listing is visible, Replay buttons show 🚫, Payload
  drawer shows redacted.
- Rollback: wrap `QuarantineTab` back in `<PlanGate>` — one chunk.

## Risks

- **No feature behavior change for Pro customers** — the wrapper only
  gated Free users out; Pro users see no visible difference beyond the
  updated subtitle copy.
- **Discoverability**: Free users might now attempt to click Replay on
  redacted events and find it disabled. The 🚫 tooltip already explains
  this; the updated subtitle copy prefaces it.
