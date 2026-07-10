-- 030_early_access.sql
--
-- RETROACTIVE FILE (2026-07-09 drift reconciliation). This table was created
-- directly in prod on 2026-05-04 (tracked in supabase_migrations as
-- "create_early_access") but never committed as a repo file. DDL below is
-- reconstructed from the live prod schema. Idempotent — a no-op on prod.
--
-- Serves the landing-page early-access form (dashboard/app/api/early-access).

CREATE TABLE IF NOT EXISTS public.early_access (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name        TEXT NOT NULL,
    email       TEXT NOT NULL,
    company     TEXT,
    stack       TEXT,
    message     TEXT,
    created_at  TIMESTAMPTZ DEFAULT now()
);

ALTER TABLE public.early_access ENABLE ROW LEVEL SECURITY;

-- Public insert is intentional: unauthenticated visitors submit the form.
-- No read policy — rows are read with the service role only.
DO $$ BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_policies
        WHERE schemaname = 'public' AND tablename = 'early_access'
          AND policyname = 'anyone can insert'
    ) THEN
        CREATE POLICY "anyone can insert" ON public.early_access
            FOR INSERT TO public WITH CHECK (true);
    END IF;
END $$;
