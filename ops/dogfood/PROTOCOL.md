# ContractGate Dogfood Protocol

**Purpose:** End-to-end product testing with **real contracts** and **real data**, without waiting for beta volunteers. This is a living protocol: every run produces findings, and those findings drive the next scenario iteration.

**Owner:** whoever is dogfooding (human or agent).  
**Primary surfaces:** [app.datacontractgate.com](https://app.datacontractgate.com) + Fly API (`https://contractgate-api.fly.dev`).  
**Local fast path:** Python SDK validator (`ops/dogfood` venv) — same rule semantics for types/enums/patterns/required.  
**Website path:** Playwright Chromium (`ops/dogfood/browser/*.spec.ts`) — public + authenticated UI, plus API suite. The agent does **not** have a live click-through browser tool; it runs this suite and reads traces/screenshots on failure.

---

## North-star questions

Every scenario must answer product questions, not just “does curl 200?”

| # | Question | Why it matters |
|---|----------|----------------|
| Q1 | Can a new user get from **sample data → draft contract** in under 10 minutes? | Time-to-value / wizard + inference |
| Q2 | Does the draft catch **realistic bad data** without being so strict it rejects good data? | Precision/recall of inference + human refine |
| Q3 | Do **pass** and **fail** paths both feel obvious in the UI (playground, quarantine, audit)? | Sales demo + operator UX |
| Q4 | Can we **promote**, **ingest at volume**, and read a **pilot report** that a buyer would forward? | Enterprise proof |
| Q5 | When the source schema **drifts**, do we detect it before production burns? | Diff / scorecard / version story |
| Q6 | Which vertical stories (gov, weather, proptech, dev platforms) sell themselves? | GTM narrative from real feeds |

If a run cannot answer a Q, log a finding with severity and a next experiment.

---

## Lifecycle (one iteration)

```
┌─────────────┐    ┌──────────────┐    ┌─────────────┐    ┌──────────────┐
│ 0. Pick     │ →  │ 1. Fetch     │ →  │ 2. Ask      │ →  │ 3. Author    │
│   scenario  │    │   real data  │    │   questions │    │   contract   │
└─────────────┘    └──────────────┘    └─────────────┘    └──────────────┘
                                                                  │
       ┌──────────────────────────────────────────────────────────┘
       ▼
┌─────────────┐    ┌──────────────┐    ┌─────────────┐    ┌──────────────┐
│ 4. Local    │ →  │ 5. Cloud UI  │ →  │ 6. Inject   │ →  │ 7. Observe   │
│   validate  │    │   deploy     │    │   pass/fail │    │   + report   │
└─────────────┘    └──────────────┘    └─────────────┘    └──────────────┘
       │                                                        │
       └──────────────────── 8. Findings → next scenario ───────┘
```

### Step 0 — Pick scenario

Scenarios live in `scenarios/*.yaml`. Start with `status: ready`. Prefer scenarios that stress a **product surface** you have not covered recently (CSV wizard vs API URL vs blank YAML vs workbench).

### Step 1 — Fetch real data

```bash
cd ops/dogfood
source .venv/bin/activate
python scripts/fetch_sources.py --scenario all
```

Writes under `fixtures/<scenario_id>/` (gitignored large files; small snapshots may be committed).

Rules for “real”:

- Public APIs / open data only (no customer data).
- Prefer live fetch; fall back to `fixtures/*/snapshot.json` if offline.
- Cap samples (default 50–200) so inference and UI stay responsive.

### Step 2 — Questions

Each scenario lists `product_questions`. Before authoring, restate them in one sentence each in the run log. If the data cannot answer a question, **change the data or the question** — do not fake a pass.

### Step 3 — Author contract

Preferred order:

1. **Infer** (JSON samples / CSV / URL) via UI or API  
2. **Human refine** enums, required, min/max, patterns, glossary  
3. **Save** reviewed YAML to `contracts/<scenario_id>.yaml`

```bash
python scripts/author_contract.py --scenario usgs_earthquake
```

### Step 4 — Local validate (gate)

```bash
python scripts/run_local.py --scenario all
```

Generates pass fixtures from clean samples and fail fixtures via controlled mutations (`scripts/mutate.py`).  
**Exit non-zero** if expected pass fails or expected fail passes.

### Step 5 — Website + cloud

**Browser (preferred for product UX):**

```bash
cd ops/dogfood && npm install && npx playwright install chromium
npm run test:public                          # no secrets
CG_EMAIL=… CG_PASSWORD=… npm run test:auth  # full dashboard
CG_API_KEY=… npm run test:api               # gateway
```

**API harness (Python):**

```bash
export CG_API_KEY=cg_live_...
export CG_API_URL=https://contractgate-api.fly.dev
python scripts/run_cloud.py --scenario usgs_earthquake
```

Manual notes: `ui/CHECKLIST.md`. On Playwright failure, open `findings/runs/browser-report` and `browser-artifacts` (screenshots/traces).

### Step 6 — Inject

| Batch | Intent |
|-------|--------|
| `pass.ndjson` | Clean production-like events |
| `fail.ndjson` | One deliberate violation class per record |
| `mixed.ndjson` | ~90% pass / ~10% fail (pilot realism) |

### Step 7 — Observe

UI: Contracts → Versions / Quarantine / Audit · Scorecard · Report export  
API: `GET /contracts/{id}/report`, `GET /quarantine`, `GET /usage`

Capture screenshots or JSON into `findings/runs/<date>-<scenario>/`.

### Step 8 — Findings → iterate

Use `findings/TEMPLATE.md`. Every finding needs:

- Observed behavior  
- Expected behavior  
- Severity (`blocker` / `major` / `minor` / `nit` / `story`)  
- Surface (wizard, infer, ingest, quarantine, UI copy, metering…)  
- Next experiment (concrete)

Promote scenarios to `status: proven` only after local + cloud (or local + UI checklist) pass.

---

## Scenario portfolio (v1)

| ID | Source | Shape | Product surface |
|----|--------|-------|-----------------|
| `usgs_earthquake` | USGS GeoJSON feed | Flat event props | JSON infer → Live ingest |
| `nyc_311` | NYC Open Data | Service requests | CSV / JSON high-cardinality enums |
| `open_meteo` | Open-Meteo forecast | Hourly time series | Nested → flat rows, numeric ranges |
| `github_events` | GitHub public events | Activity stream | Enum-heavy types, optional fields |
| `mri_tenancy` | Synthetic MRI MIX-style | Proptech vertical | Hand-authored semantic contract (Findigs story) |

Add scenarios when a GTM conversation needs a new vertical — copy `scenarios/_TEMPLATE.yaml`.

---

## Auth & environments

| Env | Use |
|-----|-----|
| Local venv | Fast loop, no network to CG |
| Cloud Free/Growth | Real multi-tenant path, metering, quarantine |
| Self-hosted `make demo` | Offline full stack if cloud key unavailable |

Secrets: `CG_API_KEY` in environment only — never commit. See `config.example.env`.

---

## Definition of done (protocol healthy)

- [ ] ≥3 scenarios `proven` on local validator  
- [ ] ≥1 scenario deployed via **UI wizard** (not only API)  
- [ ] ≥1 scenario shows **quarantine** entries for fail batch  
- [ ] ≥1 **pilot report** JSON/CSV saved under findings  
- [ ] Findings log has at least one item that improved product or docs  

---

## Cadence

| Cadence | Action |
|---------|--------|
| Daily (agent) | Fetch + local run all `ready` scenarios |
| Per release | Cloud run + UI checklist smoke |
| Weekly | Add/retire scenario; write pilot narrative for sales |

This protocol is intentionally **agent-operable**: source selection, contract authoring, mutation, validation, and finding capture are scripted. UI steps remain explicit so we still exercise the product a human buyer sees.
