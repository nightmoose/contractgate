-- RFC-083 Phase 2: O(1) per-org monthly event counter for plan enforcement.
-- Incremented once per ingest batch after audit write is scheduled.
-- Reads are primary-key lookups (no full audit_log scan on the hot path).

CREATE TABLE IF NOT EXISTS public.org_monthly_usage (
    org_id     UUID NOT NULL REFERENCES public.orgs(id) ON DELETE CASCADE,
    -- UTC calendar month as 'YYYY-MM' (e.g. '2026-07').
    period     TEXT NOT NULL,
    events     BIGINT NOT NULL DEFAULT 0 CHECK (events >= 0),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (org_id, period)
);

CREATE INDEX IF NOT EXISTS org_monthly_usage_period_idx
    ON public.org_monthly_usage (period);

COMMENT ON TABLE public.org_monthly_usage IS
    'RFC-083 Phase 2: cached monthly billable event counts per org. Service-role only.';

-- Intentionally no authenticated policies: gateway uses service-role DATABASE_URL.
ALTER TABLE public.org_monthly_usage ENABLE ROW LEVEL SECURITY;
