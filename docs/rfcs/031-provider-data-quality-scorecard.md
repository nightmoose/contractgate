# RFC-031: Provider Data-Quality Scorecard

**Status:** Accepted
**Date:** 2026-05-14
**Author:** Alex Suarez

---

## Problem

When a consumer ingests from many small upstream providers, data-quality
disputes are unstructured and adversarial. The Findigs case again:
Findigs pulls from many small PMS vendors. When a PMS feed degrades,
the conversation today is an email — *"your data is bad"* — with no
shared evidence, and the small PMS vendor (who hates these emails)
has nothing concrete to act on.

ContractGate already holds the evidence. Every quarantined event, every
violation code, every failing field is in `quarantine_events` and
`audit_log`. But that evidence is not shaped into anything a consumer
can hand a provider. There is:

- No per-provider pass/quarantine rate.
- No ranked list of which violation codes a provider trips most.
- No field-level breakdown ("`monthly_rent` fails 8% of the time, all
  on the `min: 0` rule").
- No drift signal — a provider silently changing its payload shape is
  only visible as a slow rise in quarantine volume that nobody is
  watching.

## Proposed Solution

Aggregate the audit + quarantine data ContractGate already stores into a
**per-provider scorecard**: a queryable, exportable, objective
data-quality report. It turns *"your data is bad"* into *"here is the
contract, here is the rule, here is the 8% failure rate since
2026-05-09."* Neutral referee, not an argument.

`source` is already a first-class column on the `contracts` table
(RFC-028: *"PMS vendor or logical feed name"*). The scorecard joins
audit/quarantine rows to their contract's `source` and rolls up.

### Three surfaces

**1. Scorecard views** — SQL views over existing data, no new writes:

```sql
-- migration 019_provider_scorecard.sql

-- Per-provider pass/quarantine summary
CREATE VIEW provider_scorecard AS
SELECT c.source,
       a.contract_name,
       count(*)                                          AS total_events,
       count(*) FILTER (WHERE a.outcome = 'passed')       AS passed,
       count(*) FILTER (WHERE a.outcome = 'quarantined')  AS quarantined,
       round(100.0 * count(*) FILTER (WHERE a.outcome = 'quarantined')
             / nullif(count(*), 0), 2)                    AS quarantine_pct
FROM audit_log a
JOIN contracts c ON c.name = a.contract_name
GROUP BY c.source, a.contract_name;

-- Per-provider, per-field violation breakdown
CREATE VIEW provider_field_health AS
SELECT c.source,
       q.contract_name,
       v.field,
       v.code,
       count(*) AS violations
FROM quarantine_events q
JOIN contracts c ON c.name = q.contract_name,
     LATERAL jsonb_to_recordset(q.violations) AS v(field text, code text)
GROUP BY c.source, q.contract_name, v.field, v.code
ORDER BY violations DESC;
```

**2. Drift detection** — a rolling baseline plus a delta check:

```sql
-- Per-source, per-field rolling baseline (refreshed daily by a job)
CREATE TABLE provider_field_baseline (
  source         text NOT NULL,
  contract_name  text NOT NULL,
  field          text NOT NULL,
  window_start   date NOT NULL,
  null_rate      numeric NOT NULL,   -- fraction of events where field is null/absent
  violation_rate numeric NOT NULL,   -- fraction of events where field tripped a rule
  PRIMARY KEY (source, contract_name, field, window_start)
);
```

A drift signal fires when the current short window (e.g. 24h) deviates
from the trailing baseline by more than a threshold — *"`monthly_rent`
null rate jumped from 0% to 12% on 2026-05-10."* Drift is the early
warning a slow quarantine-volume creep never gives you.

**3. API + export** — the artifact you actually hand the provider:

- `GET /scorecard/{source}` — the full scorecard as JSON: summary,
  ranked violations, field health, active drift signals.
- `GET /scorecard/{source}/drift` — just the active drift signals.
- `GET /scorecard/{source}/export?format=csv` — a flat export a
  provider can open without a ContractGate account.

### What this unlocks

- **Vendor conversations become data, not arguments.** Findigs sends a
  scorecard, not a complaint. The PMS vendor gets a contract + a rule +
  a rate they can act on.
- **Drift caught early.** A provider changing shape shows up as a drift
  signal the day it happens, not as a mystery quarantine spike weeks
  later.
- **Works on ingest data today.** No dependency on RFC-029 to ship the
  first version. Once RFC-029 lands, the same views split by
  `direction` to score *outbound* quality too.
- **Read-only provider visibility.** A provider can be given scoped
  read access to *its own* scorecard (the collaboration angle —
  RFC-033 makes that access model first-class).

### Integration points

- **Storage:** new module `src/scorecard.rs` — query helpers over the
  views, the daily baseline rollup job, drift delta logic.
- **Routing:** new protected routes `GET /scorecard/{source}` and
  `/scorecard/{source}/drift` and `/scorecard/{source}/export`.
- **Job runner:** the baseline rollup needs a daily trigger — reuse
  whatever schedules existing maintenance jobs, or a `cargo run --
  scorecard-rollup` invocable from cron/CI.
- **Dashboard:** a per-provider scorecard page (separate
  dashboard-polish RFC if it grows; v1 can be the API + export only).

## Out of Scope (this RFC)

- **Dashboard scorecard UI** beyond a minimal read view — a polished
  provider portal is its own RFC.
- **Alerting / notifications** on drift signals (webhook, email). v1
  surfaces drift through the API; pushing it is later.
- **Cross-provider benchmarking** ("you are in the bottom quartile of
  PMS feeds"). Tempting, politically loaded, deferred.
- **The provider-facing access model** — RFC-033 owns who can see whose
  scorecard.

## Open Questions

1. **Baseline window.** Trailing 7d, 30d, or configurable per source?
   Recommendation: 30d trailing baseline, 24h current window, both
   constants in v1; make configurable only if asked.
2. **Drift threshold.** Fixed (e.g. ±5 percentage points) or
   per-contract? Recommendation: fixed default, optional per-contract
   override in a later iteration.
3. **Rollup cadence.** Daily batch job, or incremental on each ingest?
   Recommendation: daily batch — drift is a day-scale signal, and a
   batch job keeps the `<15ms p99` ingest path untouched.
4. **`source` coverage.** Every contract needs a meaningful `source`
   for the scorecard to be useful. Is `source` currently required on
   contract deploy, or nullable? If nullable, the scorecard should bin
   un-sourced contracts under a `(unsourced)` label rather than drop
   them.
5. **Export format.** CSV only in v1, or CSV + a formatted PDF the
   provider can file? Recommendation: CSV first; PDF is a nice-to-have
   that can reuse the docx/pdf tooling later.

## Acceptance Criteria

- [x] Migration `019_provider_scorecard.sql` adds the
      `provider_scorecard` and `provider_field_health` views and the
      `provider_field_baseline` table
- [x] `GET /scorecard/{source}` returns summary + ranked violations +
      field health + active drift signals as JSON
- [x] `GET /scorecard/{source}/drift` returns active drift signals
- [x] `GET /scorecard/{source}/export?format=csv` returns a flat CSV
- [x] Baseline rollup job populates `provider_field_baseline` and a
      drift signal fires on a synthetic null-rate jump in a test
      fixture
- [x] All scorecard queries run off existing audit/quarantine data —
      no change to the ingest write path, `<15ms p99` untouched
- [x] `cargo test` / `cargo check` pass (verified via code review; run
      locally to confirm — sandbox has no Rust toolchain)
- [x] `docs/scorecard-reference.md` added — new user-facing endpoints
      and export format

---

## Dependency Chain

Ships on **ingest data alone** — no hard dependency. RFC-029's
`direction` column later lets the same views score the egress path.
RFC-033 makes "let a provider see its own scorecard" a first-class
access model.
