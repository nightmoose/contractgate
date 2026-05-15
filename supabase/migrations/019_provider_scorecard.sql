-- ─────────────────────────────────────────────────────────────────────────────
-- Migration 019: Provider Data-Quality Scorecard (RFC-031)
--
-- Adds three scorecard surfaces over existing audit/quarantine data.
-- No new writes to the ingest path — the <15ms p99 budget is untouched.
--
-- Adds:
--   provider_scorecard       — view: per-provider pass/quarantine summary
--   provider_field_health    — view: per-provider, per-field violation breakdown
--   provider_field_baseline  — table: rolling 30-day baseline for drift detection
--
-- Notes on schema adaptation vs. RFC spec:
--   • RFC SQL used `audit_log.contract_name` / `quarantine_events.contract_name`
--     which do not exist.  Joins go through `contract_versions` using the
--     (contract_id, contract_version) pair that is present on both tables.
--   • RFC SQL used `audit_log.outcome IN ('passed','quarantined')`.  The actual
--     column is `passed BOOLEAN`.  Adapted accordingly.
--   • RFC SQL used `v.code` in the lateral.  The stored JSON key is `kind`
--     (from the Rust `ViolationKind` enum).  Adapted and aliased as `code`.
--   • `source` is on `contract_versions` (added in migration 016), not on
--     `contracts`.  LEFT JOIN through contract_versions; NULL source is binned
--     as '(unsourced)'.
-- ─────────────────────────────────────────────────────────────────────────────

-- ── 1. provider_scorecard view ────────────────────────────────────────────────
--
-- Per-provider, per-contract pass/quarantine summary.
-- Consumers query: SELECT * FROM provider_scorecard WHERE source = 'my-vendor'

CREATE OR REPLACE VIEW provider_scorecard AS
SELECT
    COALESCE(cv.source, '(unsourced)')           AS source,
    c.name                                        AS contract_name,
    count(*)                                      AS total_events,
    count(*) FILTER (WHERE a.passed = TRUE)       AS passed,
    count(*) FILTER (WHERE a.passed = FALSE)      AS quarantined,
    round(
        100.0
        * count(*) FILTER (WHERE a.passed = FALSE)
        / NULLIF(count(*), 0),
        2
    )                                             AS quarantine_pct
FROM audit_log a
JOIN contracts c ON c.id = a.contract_id
LEFT JOIN contract_versions cv
    ON  cv.contract_id = a.contract_id
    AND cv.version     = a.contract_version
GROUP BY
    COALESCE(cv.source, '(unsourced)'),
    c.name;

-- ── 2. provider_field_health view ─────────────────────────────────────────────
--
-- Per-provider, per-field violation breakdown.  The lateral unpacks the
-- JSON violation_details array stored in quarantine_events.
--
-- Each violation object has the shape:
--   { "field": "...", "message": "...", "kind": "..." }
-- `kind` is aliased as `code` to match the RFC surface.

CREATE OR REPLACE VIEW provider_field_health AS
SELECT
    COALESCE(cv.source, '(unsourced)')  AS source,
    c.name                              AS contract_name,
    v.field,
    v.kind                              AS code,
    count(*)                            AS violations
FROM quarantine_events q
JOIN contracts c ON c.id = q.contract_id
LEFT JOIN contract_versions cv
    ON  cv.contract_id = q.contract_id
    AND cv.version     = q.contract_version,
LATERAL jsonb_to_recordset(q.violation_details) AS v(field text, kind text)
WHERE v.field IS NOT NULL
GROUP BY
    COALESCE(cv.source, '(unsourced)'),
    c.name,
    v.field,
    v.kind
ORDER BY violations DESC;

-- ── 3. provider_field_baseline table ──────────────────────────────────────────
--
-- Holds the rolling 30-day per-field baseline used by the drift detector.
-- Populated (and refreshed) daily by the `scorecard-rollup` job
-- (`cargo run -- scorecard-rollup` or cron).
--
-- null_rate:      fraction of events in the window where the field is absent.
-- violation_rate: fraction of events in the window where the field tripped a rule.
--
-- PRIMARY KEY on (source, contract_name, field, window_start) so the daily
-- rollup can UPSERT idempotently.

CREATE TABLE IF NOT EXISTS provider_field_baseline (
    source          text    NOT NULL,
    contract_name   text    NOT NULL,
    field           text    NOT NULL,
    window_start    date    NOT NULL,
    null_rate       numeric NOT NULL DEFAULT 0,
    violation_rate  numeric NOT NULL DEFAULT 0,
    PRIMARY KEY (source, contract_name, field, window_start)
);

-- Index for drift queries: "give me the latest baseline for this source"
CREATE INDEX IF NOT EXISTS idx_pfb_source_window
    ON provider_field_baseline (source, window_start DESC);

-- ── 4. RLS — no new policies needed ──────────────────────────────────────────
--
-- The views inherit the RLS of their underlying tables.  The baseline table
-- contains only aggregated (non-event-level) data; it is safe to expose to
-- authenticated users under the same blanket policy as audit_log.

ALTER TABLE provider_field_baseline ENABLE ROW LEVEL SECURITY;

CREATE POLICY "auth_all" ON provider_field_baseline
    FOR ALL TO authenticated USING (TRUE) WITH CHECK (TRUE);

CREATE POLICY "service_all" ON provider_field_baseline
    FOR ALL TO service_role USING (TRUE) WITH CHECK (TRUE);
