# Self-host

Two local stacks. They are not interchangeable.

| | Zero-auth demo | Real login |
|---|---|---|
| Command | `make demo` | the compose command below |
| Auth | off (`CONTRACTGATE_DEV_NO_AUTH=1`) | on |
| Dashboard | demo build, demo API key | sign-up and password login |
| `CONTRACTGATE_SELF_HOSTED` | unset | `true` |

`make demo` stays the zero-auth demo. Paid gates in that dashboard follow the demo build, not this page.

## Cold start

From a fresh clone, with Docker Compose and no existing `.env` in the repo root:

```bash
git clone https://github.com/nightmoose/contractgate.git
cd contractgate
cp .env.selfhost.example .env
docker compose -f docker-compose.yml -f docker-compose.selfhost.yml up --build
```

Do not add `--profile demo`. That profile starts a second dashboard on port 3000.

If this machine already ran `make demo` or `docker compose up`, the Postgres volume is the auth-off database. Wipe it first:

```bash
docker compose -f docker-compose.yml -f docker-compose.selfhost.yml down -v
```

Then run the `up --build` command again.

First boot builds the gateway and the dashboard. Later boots reuse those images.

Open http://localhost:3000/auth/signup. Create an account (password at least 8 characters). The page says to check email. Mail is not configured. Confirmation is automatic, so sign in at http://localhost:3000/auth/login with the same email and password.

The new org's stored plan is `free`. With the flag on, `GET /v1/me` reports `paid_features: true`, `self_hosted: true`, a null monthly limit, and the paid rate limit (100 requests/second, burst 200) unless a key overrides it. The dashboard opens every paid gate, including Visual Builder, Generate from Sample, From CSV, and Connect Fivetran. Kafka and Kinesis ingress are allowed. Passed-event retention follows the Enterprise window (365 days). The stored `plan` string stays `free`.

## What the flag changes

`CONTRACTGATE_SELF_HOSTED` accepts only `1`, `true`, or `TRUE`. Any other value, including unset, is the hosted path.

Hosted behavior is unchanged when the variable is unset: free plans stay capped, Pro gates follow `paid_features` from the stored plan, and Enterprise-only gates stay Enterprise-only.

On a self-hosted process the flag grants the Enterprise entitlement without rewriting `orgs.plan`:

- unlimited monthly events (usage is still counted)
- paid default rate limit, 100 / 200, unless the key sets its own
- 365-day passed-event cursor window
- payload storage, passed-event export, and Connect Fivetran
- Visual Builder, sample generation, CSV import, and the other Pro gates
- Kafka and Kinesis streaming ingress

`GET /v1/me` (and `GET /me`) includes `self_hosted`. The dashboard treats a missing `self_hosted` as false, so an older gateway does not open Enterprise gates. A missing `paid_features` still falls back to the stored plan.

## Services

`docker-compose.selfhost.yml` adds:

- GoTrue (`auth`) on the compose network
- PostgREST (`rest`)
- nginx (`api`) on http://localhost:8000, routing `/auth/v1/` and `/rest/v1/`
- a one-shot `selfhost-migrate` that applies `supabase/migrations` after GoTrue creates `auth.users`
- a gateway image built from this repo, auth on, flag on
- a dashboard image (`web`) built with the local Supabase URL and anon key

The browser uses http://localhost:8000. The Next.js server inside the dashboard container uses `SUPABASE_SERVER_URL=http://api:8000` for the same API. Hosted deploys leave `SUPABASE_SERVER_URL` unset and keep using `NEXT_PUBLIC_SUPABASE_URL`.

Postgres first boot with `CONTRACTGATE_SELF_HOST=1` creates the `anon`, `authenticated`, `service_role`, `authenticator`, `postgres`, and `supabase_auth_admin` roles, creates the empty `auth` schema, and does not apply migrations. GoTrue connects as `supabase_auth_admin` (search_path `auth, public`) and creates the auth tables. Migrations run after `auth.users.encrypted_password` exists, so migration 006 does not replace a real GoTrue user table. 006 still installs `auth.uid()` as `SELECT NULL::uuid`. Migration 044 replaces that stub with a function that reads the request JWT. On hosted Supabase, 044 is a no-op when `auth.uid()` is already the real function.

044 is new. An owner applies it on production, then removes its line from `supabase/unapplied-migrations.txt`. Until then the drift check treats it as intentionally unapplied. 006 is unchanged. A fresh hosted apply can still fail inside 006 with permission denied for schema `auth`.

## Placeholders

`.env.selfhost.example` holds a JWT secret and two HS256 keys (anon and service role) signed with that secret. They are public placeholders for a private machine. Before any other host can reach the stack, replace `SUPABASE_JWT_SECRET` (at least 32 characters) and regenerate both keys. The `authenticator` and `supabase_auth_admin` database passwords are the local placeholders in `ops/postgres/selfhost-roles.sql`.

Signup does not send mail (`GOTRUE_MAILER_AUTOCONFIRM=true`). Put a real SMTP configuration on GoTrue if you need confirmation or reset email.

Fivetran cloud cannot call localhost. Keep auth on, set `CANONICAL_API_ORIGIN` to a public https origin, and put TLS in front of the gateway before connecting Fivetran.

## Flag off

To see the hosted upgrade prompts on this same database, recreate only the gateway with the flag cleared:

```yaml
# /tmp/flag-off.yml
services:
  gateway:
    environment:
      CONTRACTGATE_SELF_HOSTED: "0"
```

```bash
docker compose -f docker-compose.yml -f docker-compose.selfhost.yml -f /tmp/flag-off.yml up -d --no-deps gateway
```

`GET /v1/me` then reports `paid_features: false` and `self_hosted: false` for a free org, and the dashboard shows the upgrade prompts. Bring the flag back with the original compose command (`up -d --no-deps gateway` is enough if the image is already built).
