# Authentication Reference

**Last updated:** 2026-09-29 (Supabase-enforced Turnstile on signup/login/reset; forgot/reset pages reachable)

DataContractGate's Rust backend supports two authentication mechanisms for the
management API: a Supabase Bearer JWT and a DB-backed API key.  The validation
hot path (`/ingest`, `/v1/ingest`, `/egress`) follows the same rules but its
scope is additionally bounded by the key's `allowed_contract_ids` list, which
is now enforced on every hot path (RFC-065).

---

## Mechanisms

### Bearer JWT (recommended for dashboard / browser sessions)

Supabase issues an RS256 session token after sign-in.  Send it as:

```
Authorization: Bearer <token>
```

The backend verifies the token against the project's JWKS endpoint, looks up
the user's primary org membership, and treats that org as the request's
authoritative `org_id`.  No additional header is needed.

### DB-backed API key

Issue keys through the dashboard (Account → API Keys).  Key issuance is
**server-side only** (RFC-056): the dashboard calls `POST /api/keys` on the
Next.js server, which generates the raw key with a CSPRNG, hashes it
server-side, and returns the raw key exactly once.  The raw key is never
stored anywhere and cannot be retrieved again.

Send the raw key on every API request as:

```
x-api-key: cg_live_<48 hex chars>
```

Each key row stores its owning `org_id`.  All management API calls are
automatically scoped to that org.  See [Key Management reference](./key-management-reference.md)
for the full issuance and revocation API.

### Local-dev escape hatch (`CONTRACTGATE_DEV_NO_AUTH`)

There is **no env-var master key** (the legacy `API_KEY` was removed in
RFC-066). For local development, `make demo`, and the compose smoke test, set
`CONTRACTGATE_DEV_NO_AUTH=1` to run the authenticated surface with no auth (the
backend then trusts the `x-org-id` header for org context). This is the only
way to disable auth, it defaults off, and it must never be set in production.

---

## Org scoping — by-ID routes (RFC-047)

Every `GET / PATCH / DELETE /contracts/{id}/…` and version route is now
org-scoped at the application layer.  The backend connects as the Supabase
service role (bypassing RLS), so the application enforces isolation itself:

- A request with a valid token whose org does not own the target contract
  receives **404 Not Found** (never 403 — UUID existence is not revealed).
- A request with no resolvable org on a prod deployment receives
  **401 Unauthorized**.
- In dev mode (`CONTRACTGATE_DEV_NO_AUTH=1`) org scoping is disabled — all
  contracts are visible regardless of org, preserving `make demo` behaviour.

Routes that are intentionally unscoped (use their own scoping mechanism):

| Route | Scoping |
|---|---|
| `POST /ingest/{id}` | key's `allowed_contract_ids` |
| `POST /v1/ingest/{id}` | key's `allowed_contract_ids` |
| `POST /egress/{id}` | key's `allowed_contract_ids` |
| `GET /public-contracts/*` | public — no auth |
| `GET /published/{ref}` | public or link-token |
| `GET /catalog` | public |

---

## Breaking change — `x-org-id` header removed (RFC-048)

**Prior to 2026-05-22**, the backend accepted a client-supplied `x-org-id`
header as a fallback when no `ValidatedKey` was present.  This header is
**no longer accepted or trusted**.  Any client that was sending `x-org-id`
must now rely on the Bearer JWT or a DB-backed API key for org context.

| Before | After |
|---|---|
| `Authorization: Bearer <jwt>` + `x-org-id: <uuid>` → org from JWT | `Authorization: Bearer <jwt>` → org from JWT ✓ (header ignored) |
| `x-api-key: <db-key>` + `x-org-id: <uuid>` → org from DB key | `x-api-key: <db-key>` → org from DB key ✓ (header ignored) |

(The `x-org-id` header is still honoured **only** in dev mode,
`CONTRACTGATE_DEV_NO_AUTH=1`, where no `ValidatedKey` is injected.)

The dashboard's `OrgProvider` has been updated accordingly — it no longer
calls `setApiOrgId` or sends `x-org-id`.

---

## CORS policy — `DASHBOARD_ORIGIN` (RFC-050)

DataContractGate uses two CORS layers with different scopes:

**Authenticated surface** (`/contracts/*`, `/ingest/*`, `/egress/*`, `/v1/*`,
`/audit`, `/stats`, `/playground/*`, `/contracts/infer/*`): only origins listed
in `DASHBOARD_ORIGIN` receive an `Access-Control-Allow-Origin` header.  Requests
from any other origin receive no CORS header and are rejected by the browser.

Allowed methods: `GET, POST, PATCH, DELETE, OPTIONS`.  
Allowed headers: `Authorization, Content-Type, Accept, x-api-key`.

> **Note on browser “CORS” errors:** If Fly returns **502 Bad Gateway**
> (machine restart, deploy, OOM), the proxy response has **no** CORS
> headers. Chrome then reports a CORS failure even though the root cause is
> the 502. Check `https://api.datacontractgate.com/health` and Fly logs
> before changing CORS config.

**Public surface** (`/health`, `/metrics`, `/openapi.json`, `/demo/*`,
`/public-contracts`, `/catalog`, `/published/*`): wildcard `*` — these routes
expose no tenant data and are safe to embed from any origin.

### `DASHBOARD_ORIGIN` environment variable

| Value | Behaviour |
|---|---|
| Unset | Startup warning; falls back to `http://localhost:3000` (dev only). |
| `https://app.datacontractgate.com` | Single origin allowed. |
| `https://app.example.com,https://staging.example.com` | Multiple origins, comma-separated. |

Set in production via Fly secrets — **do not commit the value to source control**.
Prod carries three origins per RFC-096 (one backend, two front doors):
DataContractGate's dashboard, and AISchemaGate's marketing + docs sites.

```bash
fly secrets set DASHBOARD_ORIGIN=https://app.datacontractgate.com,https://aischemagate.com,https://aischemagate.dev
```

For local development with `docker compose`, leave `DASHBOARD_ORIGIN` unset
(the fallback `http://localhost:3000` matches the default dashboard port).

---

## Signup abuse protection (2026-07-22)

An audit found 50 of 57 `auth.users` rows were automated signup-bot noise:
zero logins ever, and a random-token `display_name` (e.g.
`MyOHfgDwwVJmHXxrHs`) instead of a real name. See
`supabase/migrations/035_bot_signup_cleanup.sql` for the removal (not applied
automatically — run it via `supabase db push` when ready). Three defenses now
sit in front of `/auth/signup`:

> **Revised 2026-08-13.** Two of the three original defenses were removed after
> the captcha caused a six-day signup outage (2026-08-07 → 08-13). See
> "Removed defenses" below before re-adding anything here.

1. **Honeypot field** — an `hp_field` input, hidden via CSS and skipped by
   `tabIndex={-1}`/`autoComplete="off"`/`aria-hidden`, sits in the signup form.
   Real users never fill it; scripts that blindly fill every field do. If it's
   non-empty on submit, the client pretends signup succeeded without calling
   Supabase — no error is shown, so the bot gets no signal to adapt to.

   The field name is deliberately meaningless. It was originally `website`,
   which is exactly what password managers and browser autofill populate — a
   genuine user whose manager filled it would see "Check your email" and never
   receive one, with nothing logged anywhere. **Do not rename it to anything
   semantic** (`website`, `url`, `company`, `phone`).

2. **Email confirmation** — an account is unusable until the link in the
   confirmation email is clicked, so an unconfirmed row grants no access.

3. **Cloudflare Turnstile, verified by Supabase Auth (2026-09-29).** Bots were
   calling Supabase's signup API directly (≈380 signups Jul–Sep, zero logins,
   many `vtext.com`/ISP addresses — confirmation-email bombing), which skips
   any form-level check. So the check now lives in Supabase: **Auth → Bot and
   Abuse Protection → CAPTCHA (Turnstile)**. Once on, Supabase rejects
   `signUp`, `signInWithPassword` and `resetPasswordForEmail` without a valid
   token. `components/AuthCaptcha.tsx` renders the widget on signup, login and
   forgot-password and passes `captchaToken`. GitHub OAuth is not affected.

   | Setting | Where | Value |
   |---|---|---|
   | `NEXT_PUBLIC_TURNSTILE_SITE_KEY` | Vercel (Production + Preview) | Turnstile **site** key |
   | Turnstile secret key | Supabase → Auth → Bot and Abuse Protection | Turnstile **secret** key |
   | Turnstile hostnames | Cloudflare → Turnstile → widget | `app.datacontractgate.com`, preview domains, `localhost` |

   **Rollout order — do not reverse it:**
   1. Create the Turnstile widget in Cloudflare.
   2. Set `NEXT_PUBLIC_TURNSTILE_SITE_KEY` in Vercel and redeploy. With
      Supabase CAPTCHA still off, the token is sent and ignored; confirm the
      widget renders and login still works.
   3. Enable CAPTCHA in Supabase with the secret key. Test signup, login and
      password reset in a private window.

   **Rollback:** switch CAPTCHA off in Supabase. Takes effect immediately; no
   redeploy.

   **Guards against the August outage:** no fallback key in code (unset key →
   no widget, forms work exactly as before); `scripts/check-required-env.mjs`
   fails a production build if the key is missing or is a Cloudflare test key
   (`1x0000…`/`2x0000…`/`3x0000…`). The old app-side
   `/api/auth/verify-turnstile` route was removed — bots bypassed it.
   `TURNSTILE_SECRET_KEY` is no longer read by the dashboard; the secret belongs
   in Supabase only.

   Local dev against the production Supabase project needs `localhost` in the
   widget's hostname list and the real site key in `.env.local`.

**Password reset routing (fixed 2026-09-29).** `/auth/forgot` and
`/auth/reset` were missing from `PUBLIC_ROUTES` in `proxy.ts`, so logged-out
users were bounced to login, and the "redirect signed-in users away from
/auth/*" rule bounced the recovery session away from `/auth/reset`. Both are now
public and `/auth/reset` is exempt from that redirect.

**Email links must not use the PKCE `ConfirmationURL` (fixed 2026-10-13).**
`resetPasswordForEmail` from the browser stores a code verifier in that
browser only. The default template link
(`https://<project>.supabase.co/auth/v1/verify?token=pkce_…`) redirects to
`/auth/reset?code=…`, and exchanging the code fails everywhere else: another
browser, a phone, a private window, or an email scanner that prefetches the
URL. Signup confirmation and magic links have the same shape.

Hosted templates (Authentication → Emails) must use `{{ .TokenHash }}` and
point at the app. Source HTML is `supabase/templates/`. Recovery:

```html
<a href="{{ .SiteURL }}/auth/confirm?token_hash={{ .TokenHash }}&type=recovery&next=/auth/reset">Reset Password</a>
```

Confirmation uses `type=signup&next=/`, magic link `type=magiclink&next=/`,
email change `type=email_change&next=/account`, invite `type=invite&next=/`.
`/auth/confirm` only renders a form. Submitting it calls
`verifyOtp({ type, token_hash })` on the server, which does not need the code
verifier, and then redirects to `next`. The form is not submitted on GET, so a
scanner that only opens the link does not consume the token. Do not switch
these templates back to `{{ .ConfirmationURL }}`. Apply them to the hosted
project only after `/auth/confirm` is deployed:

```bash
SUPABASE_ACCESS_TOKEN=… python3 scripts/apply-auth-email-templates.py --apply
```

`/auth/confirm` is public and exempt from the signed-in redirect. A failed
recovery lands on `/auth/reset?error=link_invalid` ("opened in a different
browser or has expired"). Other failures land on
`/auth/login?error=confirmation_failed`.

### Removed defenses

- **Cloudflare Turnstile, first attempt (removed 2026-08-13; re-added
  correctly 2026-09-29, see above).** The signup form rendered a
  Turnstile widget whose site key came from `NEXT_PUBLIC_TURNSTILE_SITE_KEY`,
  falling back to Cloudflare's always-pass *test* key when unset. No key was
  ever added to Vercel and no widget existed in the Cloudflare account, so the
  production bundle shipped the dummy key, its token failed real `siteverify`,
  and **every signup was blocked from 2026-08-07 to 08-13**. `NEXT_PUBLIC_*` is
  inlined at build time, so this was invisible in dev and in CI. If a captcha is
  ever re-added: create the widget first, and never give a public key a dev
  fallback that silently changes production behavior.
  `dashboard/scripts/check-required-env.mjs` now fails a production build on a
  missing or localhost-valued required public var.

- **IP rate limiting (removed 2026-08-13).** `dashboard/proxy.ts` capped
  `/api/auth/*` and `/auth/callback` at 10 per 5 minutes per IP via an
  in-memory bucket. Per-instance memory means it was never a real ceiling for a
  distributed bot, while `/auth/callback` is the email-confirmation hop and
  carrier/office NAT puts many users on one IP — real availability risk for
  negligible protection. Supabase applies its own server-side rate limits to
  auth endpoints; tune them there (Auth → Rate Limits) if bots return.

## Summary — resolution order

1. **DB-backed API key** (`x-api-key`) — org from the key row (authoritative).
2. **Bearer JWT** (`Authorization: Bearer`) — org from the Supabase user's
   primary org membership (authoritative).
3. **Dev mode** (`CONTRACTGATE_DEV_NO_AUTH=1`, local only) — no auth; `x-org-id`
   header trusted for org context. Never enabled in production.

## Account self-service (2026-10-14)

- `/account` → **Profile & security**: change name (`user_metadata.full_name`),
  change password (`supabase.auth.updateUser`), delete account.
- **Delete account**: `POST /api/account/delete` with `{"confirm": "<your email>"}`.
  Calls `delete_user_account(uuid)` (migration 040, service role only): 409 if
  you are the last owner of an org that has other members; otherwise purges
  orgs where you are the only member, clears `invited_by`-style references,
  cancels those orgs' Stripe subscriptions, then deletes the auth user.
- Signed-out `/account` redirects to `/auth/login?next=/account`; sign-out
  hard-navigates to `/auth/login`.
- `/auth/reset?error=link_invalid|otp_expired|…` always shows "This reset link
  has expired or was already used", even if a session exists.
- Unknown top-level paths return a real 404 (not a 307 to sign-in).
- Response headers: `X-Content-Type-Options: nosniff`, `X-Frame-Options: DENY`,
  enforced CSP `frame-ancestors 'none'; object-src 'none'; base-uri 'self'`,
  plus a full `Content-Security-Policy-Report-Only` allow-list (see
  `dashboard/next.config.ts`).

## `GET /me` (2026-10-14)

Who am I. Also served at `GET /v1/me`. Works with an API key (`x-api-key`)
or a dashboard session (`Authorization: Bearer <jwt>`).

```json
{
  "org_id": "7d0c…",
  "org_name": "acme",
  "plan": "growth",
  "plan_display": "Pro",
  "paid_features": true,
  "self_hosted": false,
  "auth": "api_key",
  "api_key_id": "b41e…",
  "monthly_event_limit": 25000000,
  "rate_limit_rps": 100,
  "rate_limit_burst": 200
}
```

- `plan` is the org's stored plan, which sets the key's limits; `rate_limit_*` are
  the key's own override when set, else the plan default.
- `paid_features` is true for a paid plan (`growth`, `aisg_pro`, `enterprise`)
  and for every plan when `CONTRACTGATE_SELF_HOSTED` is `1`, `true`, or `TRUE`.
- `self_hosted` is true only when that variable is set. The stored `plan` and
  `plan_display` stay on the org row. A self-hosted free org reports
  `"plan": "free"`, `"paid_features": true`, `"self_hosted": true`,
  `"monthly_event_limit": null`, and the paid rate limit (100 / 200) unless
  the key overrides it. Passed-event retention is 365 days. Streaming ingress
  is allowed. Hosted free stays capped, and hosted Pro does not gain
  Enterprise streaming from `paid_features` alone.
- `auth` is `api_key` or `session`; `api_key_id` is `null` for sessions.
- No emails or user ids are returned. `org_name` is `null` if the stored name
  looks like an email.
- `401` on missing or invalid credentials.
