# Finding — Production deploy without migration 034 (blocker)

| Field | Value |
|-------|-------|
| Date | 2026-07-20 |
| Surface | API + Contracts UI |
| Severity | **blocker** (production outage for list/create/quarantine) |
| Env | fly `contractgate-api` + Supabase |

## Observed

After a Fly deploy around `2026-07-20T20:29Z`, production returned **500 Database error** on:

- `GET /contracts`
- `GET /quarantine`
- `POST /contracts` (create)

UI: red banner **“Failed to load contracts: Database error”**.

Fly logs:

```
column qe.payload_redacted does not exist
column "store_event_payloads" does not exist
column c.store_event_payloads does not exist
```

## Root cause

App binary expected RFC-086 gated payload storage columns from
`supabase/migrations/034_gated_payload_storage.sql`, but the migration had
**not** been applied to production Supabase.

## Fix applied (ops)

Migration 034 applied manually via `psql` on the Fly machine against
`DATABASE_URL` at ~2026-07-20T20:32Z. Columns verified present.

## Expected process

Deploy checklist: **run pending Supabase migrations before or with** every
Fly release that depends on schema. Prefer automating `034` (and future) in CI
or a release script that fails the deploy if schema lag is detected.

## Resolution

- [x] Migration applied; contracts/quarantine/create restored
- [ ] Codify migration step in release runbook / CI
- [ ] Optional: `/ready` or startup check for required columns
