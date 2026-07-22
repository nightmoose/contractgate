# Dogfood run log

Newest entries at the top.

## 2026-07-20 — Full browser + API green (26/26) after password + migration fix

**Credentials:** `testing123@nightmoose.com` (password reset completed by user).  
**Playwright:** `26 passed (29.6s)` — public site, auth UI, authenticated API.

### Auth UI covered

- Login (email/password submit, not GitHub)
- Contracts list
- **New Contract → Start Blank → Create** with dogfood YAML
- Playground, Audit, Scorecard, Catalog, Workbench, Scaffold, Account (no 500s)
- Dogfood contract names visible in list

### Critical production incident caught by dogfood

Deploy without `034_gated_payload_storage.sql` → 500s on contracts/quarantine.  
**Fixed:** migration applied on prod DB. Details:
`findings/2026-07-20-missing-migration-034.md`.

### Security note

Password + API key were shared in chat — **rotate both** when dogfood is done.

---

## 2026-07-20 — Production API dogfood with live key (testing123 org)

**Auth:** `CG_API_KEY` for `testing123@nightmoose.com` org (Free plan).  
**Password UI:** recovery email requested via Supabase `/auth/v1/recover` (check inbox). No in-app “Forgot password” link today. Local `.env.local` has placeholder Supabase — cannot admin-reset without production `SERVICE_ROLE`.

### Results

| Check | Result |
|-------|--------|
| Playwright `test:api` | **9/9 passed** |
| Local scenarios | **5/5** |
| Cloud USGS pass/fail/mixed | 50/50 pass · 5/5 fail (422 + quarantine) · 36/40 mixed (207) |
| Cloud mri / open_meteo / github / nyc_311 | all pass/fail/mixed as expected |
| Usage after run | **553 / 1M** events |
| Contracts created | usgs, mri_tenancy, open_meteo, github, nyc_311 (+ dogfood copies) |
| Quarantine | **52** rows |
| Pilot report (usgs) | pass_rate ~0.91; top: mag type_mismatch, status enum, missing id |

### Product notes

1. All-fail ingest returns **HTTP 422** (not 207) — harness updated to accept.
2. Deploy blocked while quarantine pending — correct, but wizard UX should explain.
3. Login has **no forgot-password** control — recovery only via Supabase email API.

### Next

1. User completes password reset from email → `CG_EMAIL` + `CG_PASSWORD` → `npm run test:auth` for full UI.
2. Rotate API key (shared in chat).

---

## 2026-07-20 — Browser + API public suite green

**Result:** Playwright `npm run test:public` → **14 passed, 4 skipped** (auth API needs `CG_API_KEY`)

| Suite | Coverage |
|-------|----------|
| Public API | `/health`, `/ready`, `/openapi.json`, `/catalog`, 401 on `/usage` |
| Public web | home, login, signup, pricing, stream-demo, docs, privacy, terms |
| Auth gate | `/contracts`, `/playground`, `/audit`, `/scorecard` require login |
| Marketing | datacontractgate.com home |

**Auth UI suite** (`browser/auth.spec.ts`) ready — skipped until `CG_EMAIL` + `CG_PASSWORD`.  
**Auth API suite** ready — skipped until `CG_API_KEY`.

---

## 2026-07-20 — Local suite green (iteration 1)

**Result:** `5/5` scenarios PASS on local Python SDK validator  
**Log:** `findings/runs/20260720T201419Z`

| Scenario | Pass | Fail caught | Mixed |
|----------|------|-------------|-------|
| github_events | 30/30 | 5/5 | 27/30 |
| mri_tenancy | 40/40 | 5/5 | 36/40 |
| nyc_311 | 100/100 | 5/5 | 36/40 |
| open_meteo | 24/24 | 5/5 | 22/24 |
| usgs_earthquake | 50/50 | 5/5 | 36/40 |

**Findings filed**

- `findings/2026-07-20-null-optional-fields.md` — JSON `null` on optional fields → type_mismatch (gateway + SDK). Harness now omits null keys in fixtures.

**Also learned while green-running**

- Open-data IDs that look numeric must stay strings (`unique_key`, USGS `code`, GitHub `id`).
- Socrata lat/long arrive as strings — coerce only for known numeric fields.
- GitHub `type` should not be enum’d from a 30-event sample (high cardinality long-term).

**Still blocked for cloud/UI**

- Need `CG_API_KEY` from app.datacontractgate.com to run `scripts/run_cloud.py` and complete UI checklist Path A.

### Next iteration

1. Export API key → cloud run `usgs_earthquake` + `mri_tenancy`.
2. UI checklist Path A (USGS) + Path B (MRI) with screenshots.
3. Decide product stance on null-as-absent (RFC?).
4. Mark scenarios `proven` after cloud+UI.

---

## 2026-07-20 — Protocol bootstrap

- Created `ops/dogfood` harness + 5 scenarios (USGS, NYC 311, Open-Meteo, GitHub events, MRI tenancy).
- Local path: fetch → author → validate with Python SDK.
- UI checklist written for app.datacontractgate.com.
