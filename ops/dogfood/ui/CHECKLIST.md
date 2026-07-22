# UI checklist — app.datacontractgate.com

Use this when exercising the **product a buyer sees**. Pair with a scenario from `scenarios/`.

**Base:** https://app.datacontractgate.com  
**Sign-in:** same dashboard account as CG Status / production org.

---

## Pre-flight

- [ ] Logged into the correct org (not demo-only if you need persistence)
- [ ] Plan allows contract create + ingest (Free ok for basic; Growth for visual builder / some infer)
- [ ] Local harness already green: `python scripts/run_local.py --scenario <id>`
- [ ] Fixtures ready: `fixtures/<id>/pass.ndjson` and `fail.ndjson`

---

## Path A — Wizard from sample (preferred for usgs / github / weather)

1. **Contracts** → **New contract** (or equivalent CTA)
2. Choose source:
   - **Sample JSON / blank YAML** — paste first 5 objects from `fixtures/<id>/events.json`
   - **CSV** — for nyc_311, convert events to CSV if CSV path is the target surface
   - **From catalog** — only if an open-data contract matches
3. Review inferred fields:
   - [ ] Required flags look right
   - [ ] Enums not absurdly large
   - [ ] Numbers not typed as strings
4. Refine 2–3 fields manually (prove visual builder / YAML tab)
5. **Save** → note contract name + id
6. **Versions** → promote **stable** (or Deploy)
7. **Playground** — paste one pass event → expect pass
8. **Playground** — paste one fail event → expect violations readable
9. **Ingest** (Workbench curl, API keys page, or `run_cloud.py`)
10. **Quarantine** tab — fail events visible with field/kind
11. **Report** / scorecard — pass rate moves
12. Screenshot or export JSON into `findings/runs/<date>-ui/`

### Product questions to answer out loud

- How long did Q1 (sample → draft) take?
- Would a non-YAML person succeed?
- Any dead-end copy, 500s, or empty states?

---

## Path B — Blank YAML (mri_tenancy / vertical story)

1. New contract → **Blank YAML**
2. Paste `contracts/mri_tenancy.yaml`
3. Glossary + metrics visible?
4. Promote + playground + ingest mixed batch
5. Generate **pilot report** (`GET /contracts/{id}/report` or UI export)
6. Would you email that report to a design partner? Y/N + why

---

## Path C — API Workbench (optional stretch)

1. Open **Workbench** / API explorer
2. Seed with Open-Meteo or GitHub URL (CORS permitting)
3. Infer → refine → deploy
4. Log CORS failures as findings (expected for some APIs)

---

## Regression smoke (every release)

| Click | Expect |
|-------|--------|
| Stream demo | Live pass/fail animation |
| Playground | Validate without save |
| Contracts list | New dogfood contracts appear |
| Usage | Count increased after ingest |
| CG Status Live | Login shows usage after cloud traffic |

---

## Finding capture

For each friction:

```text
Surface: wizard | playground | quarantine | report | workbench
Severity: blocker | major | minor | nit | story
Observed:
Expected:
Screenshot/log:
Next experiment:
```

Append to `findings/RUNLOG.md`.
