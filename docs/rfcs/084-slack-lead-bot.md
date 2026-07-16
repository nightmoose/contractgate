# RFC-084 — Slack lead-intake bot

**Status:** Implemented (pending merge + prod migration apply)
**Date:** 2026-07-15
**Branch:** TBD (currently uncommitted on `fix/hero-demo-script` — move to its own)
**Migration:** `033_slack_leads.sql`

---

## Problem

Inbound interest from Slack communities has no capture path — a prospect who DMs
or mentions the bot should be walked through a lightweight intake and land as a
lead we can follow up on. This is a GTM/funnel feature (advances the first-sale
motion), not part of the validation product.

## Design

Slack app → Vercel routes on the dashboard → Supabase (service-role):

- **`POST /api/slack/events`** — the Slack Events API endpoint. Verifies the
  request HMAC-SHA256 signature (`SLACK_SIGNING_SECRET`, timing-safe) before doing
  anything, handles the URL-verification challenge, then dispatches to the message
  handler.
- **`message-handler.ts`** — conversational intake. Loads/updates per-thread state
  from `slack_conversations`, runs the exchange (Claude via `ANTHROPIC_API_KEY`),
  and on completion inserts a row into `slack_leads`. Uses the **service-role**
  Supabase client (`SUPABASE_SERVICE_ROLE_KEY`).
- **`POST /api/slack/announce`** — outbound helper, guarded by a shared secret
  (`SLACK_ANNOUNCE_SECRET`, timing-safe).

Serverless functions are stateless, so `slack_conversations` is the only durable
per-thread store.

## Data model (migration 033)

- **`slack_leads`** — one row per completed intake: `slack_user_id`,
  `slack_workspace`, `name`, `email`, `company`, `stack`, `use_case`,
  `onboarding_pref`, `status` (`new|contacted|qualified|closed`). Contains **lead
  PII**.
- **`slack_conversations`** — PK `(workspace, thread_ts)`; short message history +
  `intake_state` JSON; `lead_id` FK once completed.

## Security

- **Request auth:** Slack signature verification on `/events`; shared-secret on
  `/announce`. ✓
- **DB access:** service-role key only (never the anon/publishable key). ✓
- **RLS (the fix):** both tables `ENABLE ROW LEVEL SECURITY` with **no policy** →
  deny-by-default for anon/authenticated; the bot's service-role key bypasses RLS.
  This matches the `idempotency_keys` / `stripe_*` service-role-only pattern.
  Without it, lead PII (name/email/company) is readable via PostgREST by anon
  (advisor ERROR `rls_disabled_in_public`). CI Sentinel A8 asserts RLS stays on.
- **Privacy:** lead data is customer/prospect PII — fold into the DPA / data
  handling review (see `docs/data-room/ip-assignment-checklist.md`).

## Env vars

`SLACK_BOT_TOKEN`, `SLACK_SIGNING_SECRET`, `SLACK_ANNOUNCE_SECRET`,
`SUPABASE_SERVICE_ROLE_KEY`, `ANTHROPIC_API_KEY` (+ `NEXT_PUBLIC_SUPABASE_URL`).
See `.env.example` and `docs/slack-bot-setup.md`.

## Rollout checklist

1. Migration file committed; **`EXPECTED_MIGRATION_COUNT` bumped 32 → 33** + Sentinel
   A8 (done).
2. Apply `033_slack_leads.sql` to prod Supabase (after commit, to avoid drift).
3. Set the Slack + Anthropic env vars in Vercel (never commit them).
4. Deploy the dashboard; point the Slack app's Events URL at `/api/slack/events`.
5. Re-run Supabase advisors — the two tables should show INFO
   (`rls_enabled_no_policy`, by-design), not ERROR.

## Out of scope / follow-ups

- Lead → CRM sync; qualification scoring; rate/abuse limiting on `/events`.
- DPA / retention policy for `slack_leads`.
