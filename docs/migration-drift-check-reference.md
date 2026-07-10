# Migration Drift Check Reference

Daily scheduled job (`.github/workflows/migration-drift.yml`) that compares
prod's migration ledger against the files in `supabase/migrations/`. Exists
because file count alone (the PR-time Sentinel B in `ci.yml`) can't catch
"a migration file exists but was never applied to prod" — the exact failure
mode behind the 2026-06-05 Stripe webhook incident, where migration
`026_plan_tier.sql` had a file, passed CI, but was never run against prod.

## What it checks

`scripts/check_migration_drift.sh` reads:

- **Files:** every `supabase/migrations/*.sql` basename (minus `.sql`).
- **Ledger:** `SELECT name FROM supabase_migrations.schema_migrations` from prod.

It fails if either side has an entry the other doesn't, after resolving known
cosmetic name mismatches through a hard-coded alias map (currently:
`create_early_access` ↔ `030_early_access`, from the 2026-07-09 drift
reconciliation — see `scripts/backfill_schema_migrations.sql`). Add new
entries to `ALIAS_MAP` in the script if a future migration is ever applied by
hand under a different ledger name.

## Where it runs

Scheduled (`cron: '15 6 * * *'`, 06:15 UTC daily) + manual `workflow_dispatch`.
**Not** on `pull_request`: this check needs a prod DB credential, and PRs from
forks must never see repo secrets. The existing PR-time file-count check
(Sentinel B in `ci.yml`'s `migrations-check` job) stays as the fast,
secret-free guard on every PR.

## One-time setup (Alex — not done by this PR)

The workflow needs a **read-only** Postgres role that can only see the
migrations ledger, nothing else:

```sql
-- Run once against prod, as an admin.
CREATE ROLE migration_drift_reader LOGIN PASSWORD '<generate-a-strong-password>';
GRANT USAGE ON SCHEMA supabase_migrations TO migration_drift_reader;
GRANT SELECT ON supabase_migrations.schema_migrations TO migration_drift_reader;
```

Then add the connection string as a repo secret:

- **Settings → Secrets and variables → Actions → New repository secret**
- Name: `PROD_DATABASE_URL`
- Value: `postgres://migration_drift_reader:<password>@<prod-host>:5432/postgres`
  (use the prod Supabase connection string with this role's credentials —
  session pooler is fine since this is one query a day, not a hot path)

Without this secret, the workflow fails fast with a pointer back to this doc
instead of a confusing psql connection error.

## Local dry run (no prod access needed)

`check_migration_drift.sh --ledger-file <path>` skips the database entirely
and reads ledger names from a plain text file (one per line) — useful for
testing the diff logic itself:

```bash
# Baseline: feed it the exact file list (aliased) — should pass.
ls supabase/migrations/*.sql | xargs -n1 basename | sed 's/\.sql$//' \
  | sed 's/^030_early_access$/create_early_access/' > /tmp/ledger.txt
./scripts/check_migration_drift.sh --ledger-file /tmp/ledger.txt   # OK

# Drift: drop a line, confirm it fails.
grep -v '^020_contract_publication$' /tmp/ledger.txt > /tmp/ledger_missing.txt
./scripts/check_migration_drift.sh --ledger-file /tmp/ledger_missing.txt   # fails

# Drift the other way: add a name with no file, confirm it fails.
(cat /tmp/ledger.txt; echo "999_phantom") > /tmp/ledger_extra.txt
./scripts/check_migration_drift.sh --ledger-file /tmp/ledger_extra.txt   # fails
```

## On failure

The workflow run shows exactly which basenames are missing on which side.
Typical next step: apply the missing migration to prod (if a file exists with
no ledger row) or commit the missing file (if prod has an untracked change —
see `scripts/backfill_schema_migrations.sql` for the pattern used during the
2026-07-09 reconciliation).
