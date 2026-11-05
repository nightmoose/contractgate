# Email Sending Setup — AWS SES (datacontractgate.com)

**Goal:** send the early-access notification mail from the dashboard
(`/api/early-access`) via **Amazon SES v2**, authenticated as the
**`datacontractgate.com`** domain. The "From" address is
`no-reply@datacontractgate.com`; the submitter's address is used as `Reply-To`.

Region: **`us-east-1`** (set via `AWS_REGION`, defaults to `us-east-1`).

## 1. Verify the domain identity (`datacontractgate.com`)

In the SES console (us-east-1) → **Identities → Create identity → Domain** and
enter `datacontractgate.com`. Enable **Easy DKIM** (2048-bit).

### DKIM — 3 CNAME records

SES generates three selector tokens. Add all three at your DNS host for
`datacontractgate.com`:

| Type | Host / Name | Value |
|---|---|---|
| `CNAME` | `<token1>._domainkey` | `<token1>.dkim.amazonses.com` |
| `CNAME` | `<token2>._domainkey` | `<token2>.dkim.amazonses.com` |
| `CNAME` | `<token3>._domainkey` | `<token3>.dkim.amazonses.com` |

SES marks the identity **Verified** once it resolves the CNAMEs (minutes to
~72h). DKIM then signs outbound mail as `datacontractgate.com`.

### Custom MAIL FROM (for SPF alignment)

In the identity → **Custom MAIL FROM domain** → set `mail.datacontractgate.com`.
Add the two records SES shows:

| Type | Host / Name | Value |
|---|---|---|
| `MX` | `mail` | `feedback-smtp.us-east-1.amazonses.com` (priority `10`) |
| `TXT` | `mail` | `v=spf1 include:amazonses.com ~all` |

> This subdomain is the bounce/complaint return path. Using a custom MAIL FROM
> makes SPF **align** with the `From` domain. Keep `~all` (softfail) until
> delivery is confirmed.

### SPF + DMARC on the root

| Type | Host / Name | Value |
|---|---|---|
| `TXT` | `@` (root — `datacontractgate.com`) | `v=spf1 include:amazonses.com ~all` |
| `TXT` | `_dmarc` (`_dmarc.datacontractgate.com`) | `v=DMARC1; p=none; rua=mailto:dmarc@datacontractgate.com` |

> `p=none` = monitor only while alignment beds in. Tighten to `p=quarantine`
> after a week of clean `rua` reports. **Do not touch the MX records** used for
> inbound mail — the `mail.` MX above is a separate subdomain only for SES
> bounces.

## 2. Leave the sandbox (production access)

New SES accounts are in the **sandbox**: you can only send to verified
addresses, with a low quota. To send to arbitrary recipients:

SES console → **Account dashboard → Request production access**. Provide:

- **Mail type:** Transactional
- **Website:** `https://datacontractgate.com`
- **Use case:** "Early-access request notifications from our marketing site
  contact form to an internal inbox."
- **Bounce/complaint handling:** describe monitoring the DMARC `rua` reports and
  the SES bounce/complaint metrics.

Until production access is granted, verify the recipient address
(`EARLY_ACCESS_NOTIFY_TO`) as a separate email identity so the form works in the
sandbox.

## 3. IAM — least-privilege sending policy

Create an IAM user (programmatic access) whose only permission is to send from
this one identity. Attach:

```json
{
  "Version": "2012-10-17",
  "Statement": [
    {
      "Sid": "SendFromDataContractGateIdentity",
      "Effect": "Allow",
      "Action": "ses:SendEmail",
      "Resource": "arn:aws:ses:us-east-1:<ACCOUNT_ID>:identity/datacontractgate.com"
    }
  ]
}
```

> Scope `Resource` to the identity ARN so the key cannot send as any other
> domain. `ses:SendEmail` covers the SES v2 `SendEmail` API used by the route.
> Generate an access key for this user and store it as the AWS env vars below.

## 4. Vercel environment variables

Dashboard → **Project → Settings → Environment Variables** (Production, and
Preview if you want the form live there):

| Name | Value | Notes |
|---|---|---|
| `AWS_REGION` | `us-east-1` | Optional — defaults to `us-east-1` in code. |
| `AWS_ACCESS_KEY_ID` | `AKIA…` | From the least-privilege IAM user above. |
| `AWS_SECRET_ACCESS_KEY` | `…` | Server secret — never expose client-side. |
| `AWS_SESSION_TOKEN` | `…` | Optional — only when using temporary STS credentials; signed as `X-Amz-Security-Token`. |
| `EARLY_ACCESS_NOTIFY_TO` | `early-access@datacontractgate.com` | Internal inbox that receives requests. If unset, the route returns a 500 and logs — it never crashes. |

The route calls the SES v2 `SendEmail` HTTP API directly, signing each request
with AWS Signature Version 4 (a small `node:crypto` signer in
`dashboard/lib/aws-sigv4.ts`) — no AWS SDK dependency. Credentials are read from
the env vars above at request time; the code never hardcodes them.

## 5. Verify

1. After DNS propagates, confirm the identity shows **Verified** and DKIM
   **Enabled** in the SES console, and:
   ```
   dig +short TXT datacontractgate.com            # SPF line
   dig +short TXT _dmarc.datacontractgate.com     # DMARC line
   dig +short MX  mail.datacontractgate.com        # SES feedback MX
   ```
2. Submit the early-access form (or `POST /api/early-access`) and confirm the
   notification lands in `EARLY_ACCESS_NOTIFY_TO`.
3. Optional: send a test to https://www.mail-tester.com and confirm
   SPF=pass, DKIM=pass, DMARC=pass.
