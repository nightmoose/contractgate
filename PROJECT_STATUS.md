# Project Status — ContractGate

**As of:** 2026-08-23  
**GitHub:** https://github.com/nightmoose/contractgate (**public**)  
**Local:** `~/contractgate`  
**RFC index:** [`docs/STATUS.md`](docs/STATUS.md)

## What this is

Semantic contract enforcement at ingestion (patent pending). Rust gateway +
Next.js dashboard + Python SDK + Kafka Connect SMT. Hosted API on Fly.io;
dashboard on Vercel.

## Current state

Last three weeks:

- **2026-08-07→13 signup outage:** Cloudflare Turnstile blocked **all** signups.
  Fix: remove Turnstile (`f1ed3b1` / PR #180).
- Dashboard: agent docs served anonymously; OG cards + analytics
- CI: clippy `useless_format`; pin toolchain **1.98.0**; `h2` 0.4.16 for
  RUSTSEC-2026-0258
- Marketing automation branch merged earlier in August

Local `main` matches `origin/main` at the merge of
`nightly-maintenance-2026-08-13-signup-outage-fix`. Checkout was on that
maintenance branch during the audit (fully pushed).

Stale local branches with **gone** upstreams are historical nightly branches —
do not revive them.

`dev/p1-abuse-prevention` is **244 commits behind main** — treat as abandoned
unless you rebase on purpose.

## Open / next

- Confirm production signup still works without Turnstile (bot abuse vs. growth)
- JWT CryptoProvider incident write-up is in git history (2026-07-14)
- iOS status app is a **separate repo**: `contractgate-status-ios`

## Notes for humans and AIs

Do not commit `dashboard/node_modules`, `target/`, or real API keys.
`docs/STATUS.md` is the RFC ledger, not day-to-day ops — this file is ops.
