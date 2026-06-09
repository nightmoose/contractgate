# Email Sending Setup — nightmoose.com (shared auth sender)

**Goal:** make Supabase auth email (signup confirmation, password reset, etc.)
actually deliver. All apps (ContractGate, Crafty528hz, future) send auth mail
from **`noreply@nightmoose.com`** — Nightmoose is the consulting co that built
them, so a shared sender is fine for now.

## Root cause of the broken signup email (2026-06-08)

Resend is verified against **`nightmoose.com` (root)**, so it sends as
`@nightmoose.com`. But the root domain had **no SPF and no DMARC** record — the
SPF + bounce MX were sitting on a `send.nightmoose.com` subdomain, orphaned from
the verified identity. Result: receiver checks SPF on `nightmoose.com`, finds
nothing → SPF fails → unauthenticated, DMARC-less mail → **Hotmail/Gmail silently
junk it.** The signup code and ImprovMX forwarding were both fine; this was purely
a DNS authentication gap on the sending domain.

ImprovMX (inbound forwarding) is unaffected — it only governs the **MX** records,
which stay pointed at ImprovMX. SPF/DKIM/DMARC govern **outbound** and are
independent.

## Fix — add 2 DNS records on nightmoose.com

Add these at your DNS host for `nightmoose.com`. **Do not touch the MX records**
(they must stay ImprovMX for inbound to keep working).

### 1. SPF (TXT on the root)

| Field | Value |
|---|---|
| Type | `TXT` |
| Host / Name | `@` (root — `nightmoose.com`) |
| Value | `v=spf1 include:amazonses.com ~all` |

> Resend sends via Amazon SES, so `include:amazonses.com` is the authorization
> (confirmed by resolving Resend's own SPF chain for this domain). If Resend's
> dashboard for nightmoose shows a different SPF value, use **that** — it's the
> source of truth. `~all` (softfail) is correct to start; don't use `-all` until
> delivery is confirmed.

### 2. DMARC (TXT)

| Field | Value |
|---|---|
| Type | `TXT` |
| Host / Name | `_dmarc` (i.e. `_dmarc.nightmoose.com`) |
| Value | `v=DMARC1; p=none; rua=mailto:dmarc@nightmoose.com` |

> `p=none` = monitor only, don't quarantine, while alignment beds in. Tighten to
> `p=quarantine` (matching the other domains) after a week of clean `rua` reports.

**DKIM is already present** (`resend._domainkey.nightmoose.com`) — leave it as is.

Propagation: up to ~24h, usually minutes. Resend re-checks for 72h.

## Supabase SMTP settings (each project)

For **every** Supabase project that sends auth mail (ContractGate =
`nmhoehpveqkkpfegkzpn`, plus Crafty etc.):

Dashboard → **Authentication → Emails → SMTP Settings** → Enable custom SMTP:

| Field | Value |
|---|---|
| Host | `smtp.resend.com` |
| Port | `465` |
| Username | `resend` |
| Password | your **Resend API key** (`re_...`) — server secret, never client-side |
| Sender email | `noreply@nightmoose.com` |
| Sender name | e.g. `ContractGate` (per project) |

> The sender domain **must** match the Resend-verified domain (`nightmoose.com`)
> or alignment fails again. Since Resend is verified on the root, use the root —
> `noreply@nightmoose.com`, **not** `@send.nightmoose.com`.

Custom SMTP also lifts the default rate limit (2/hour → 30 new users/hour,
tunable under Authentication → Rate Limits).

## Verify

1. After DNS propagates:
   ```
   dig +short TXT nightmoose.com            # should show the SPF line
   dig +short TXT _dmarc.nightmoose.com     # should show the DMARC line
   ```
2. Do a real fresh signup with a Hotmail/Gmail address. The confirmation email
   should arrive (check spam once; after SPF+DMARC it should land in inbox).
3. Optional: send a test from Resend's dashboard to https://www.mail-tester.com
   and confirm SPF=pass, DKIM=pass, DMARC=pass.

## Notes / future

- One Resend domain on the free tier (3,000/mo, 100/day). All apps share
  `nightmoose.com` for now. If per-brand sending is wanted later
  (`noreply@datacontractgate.com`), Resend Pro ($20/mo) allows 10 domains —
  cheaper than juggling a second provider.
- `datacontractgate.com` and `crafty528hz.com` currently have **no working
  outbound sender** (SPF only authorizes ImprovMX, which can't send). They're
  fine as long as their apps send auth mail via the shared nightmoose sender.
