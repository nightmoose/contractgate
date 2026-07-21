# Dogfood harness

Runnable end-to-end testing for ContractGate: **real public data**, **API**, and **full website** via Playwright (headless Chromium).

There is no interactive “browser MCP” in the agent — website coverage is **Playwright driven from this folder**, which the agent can run and iterate on the same way as the Python harness.

## Quick start

```bash
cd ops/dogfood

# --- Local contract gate (Python) ---
python3 -m venv .venv && source .venv/bin/activate
pip install -e ../../sdks/python httpx PyYAML
./scripts/run_iteration.sh

# --- Website + API (Playwright) ---
npm install
npx playwright install chromium

# Public pages + public API (no secrets)
npm run test:public

# Full product UI (dashboard login)
export CG_EMAIL='you@example.com'
export CG_PASSWORD='…'
npm run test:auth

# Authenticated API (ingest, usage, report)
export CG_API_KEY=cg_live_…
npm run test:api

# Everything browser-related
npm test
```

| Layer | How | Secrets |
|-------|-----|---------|
| Local validate | `scripts/run_*.py` | none |
| Public site | `browser/public.spec.ts` | none |
| Public API | `browser/api.spec.ts` | none |
| Dashboard UI | `browser/auth.spec.ts` | `CG_EMAIL` + `CG_PASSWORD` |
| Private API | `browser/api.spec.ts` | `CG_API_KEY` |
| Manual UI notes | `ui/CHECKLIST.md` | session |

Artifacts: `findings/runs/browser-report/`, `browser-artifacts/`, `browser-last.json`.

Read **[PROTOCOL.md](PROTOCOL.md)** for the methodology.
