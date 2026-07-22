# ContractGate — Internal Admin (RFC-088)

A **standalone, god-mode** ops console. Read-only view of every org with plan +
live Stripe subscription status and member emails, with deep links to the Stripe
customer and the Supabase row where you make the real change. First tenant of a
broader internal-tools app.

**This is not the customer dashboard.** It holds the Supabase **service-role**
key and the **Stripe secret** key (server-side only) and must never be publicly
reachable.

## Access model

1. Deploy behind Vercel Access / SSO — never expose it publicly.
2. App login: Supabase email/password, then an `ADMIN_EMAILS` allowlist check
   (middleware + per-page `requireAdmin()`). Non-allowlisted sessions are denied.
3. Secrets (`SUPABASE_SERVICE_ROLE_KEY`, `STRIPE_SECRET_KEY`) live only in this
   app's server env. Nothing privileged is sent to the browser.

## Setup

```bash
cd internal-admin
cp .env.example .env.local   # fill in the values (see below)
npm install
npm run type-check           # verify the scaffold
npm run dev                  # http://localhost:3100
```

Env (all server-only except the two NEXT_PUBLIC anon values used for the login
form): `SUPABASE_URL`, `SUPABASE_SERVICE_ROLE_KEY`,
`NEXT_PUBLIC_SUPABASE_URL`, `NEXT_PUBLIC_SUPABASE_ANON_KEY`, `STRIPE_SECRET_KEY`,
`ADMIN_EMAILS`, `SUPABASE_PROJECT_REF`.

## Deploy

Separate Vercel project, root `internal-admin/`. Set the env vars in Vercel,
enable Vercel Access, add your email(s) to `ADMIN_EMAILS`.

## Status

v1 scaffold (RFC-088): Users & Subscriptions view + auth gate + data layer. It
has **not** been built/run in the dev sandbox — run `npm install` +
`npm run type-check` and a test login before trusting the gate. Read-only: all
mutations happen in Stripe/Supabase via the links.

Future tools (e.g. Mercenary) become additional routes here.
