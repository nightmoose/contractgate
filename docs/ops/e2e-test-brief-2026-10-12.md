# End-to-end test brief — DataContractGate + AISchemaGate (2026-10-12)

For the tester. Everything below shipped on 2026-10-12 and has been checked by
unit tests, CI, and server-rendered page checks, but **not by a real user flow**.
Your job is to walk the real flows in production and report concrete findings
(what you did, what you expected, what happened, screenshot or response body).

Use a **fresh test account** for each sign-up section. Never use Alex's account.
Do not share API keys in the report; quote only the `cg_live_` prefix.

## What changed (so you know what to look for)

- **Two brands, one product.** DataContractGate (data pipelines) and AISchemaGate
  (LLM output validation) share one backend, one account system and one
  dashboard at `app.datacontractgate.com`. Visitors and accounts that come from
  aischemagate.com see AISchemaGate branding and a trimmed navigation.
- **One price ladder for both:** Free (100,000 validations/month, 10 req/s per
  key, 7-day logs) · Pro ($199/mo or $166/mo billed annually = $1,992/yr;
  25,000,000/month; 100 req/s; 90-day logs) · Enterprise (contact us).
  "Growth" was renamed **Pro** everywhere.
- **New API host:** `https://api.datacontractgate.com` (direct to the API).
  `https://app.datacontractgate.com` still accepts API calls too.
- **Log retention:** an hourly job (at :17) deletes logs older than 7 days on
  Free and 90 days on Pro.
- **SDK 0.3.0** (PyPI `aischemagate` / `contractgate`, npm
  `@nightmoose/aischemagate` / `@nightmoose/contractgate-sdk`) adds `LLMGate`.
  `contract_id` accepts the contract's **name or UUID**.

## Test plan

Record each as PASS / FAIL / BLOCKED with evidence.

### A. AISchemaGate marketing → sign-up (fresh account #1)
1. Open `https://aischemagate.com`. Check: page loads, favicon shows in the tab,
   logo + "AISchemaGate by DataContractGate", hero copy, Python/TypeScript code tabs switch.
2. Open `https://aischemagate.dev`. Expect the **docs index**, not the landing page.
   Open both docs; links inside them work; `aischemagate.dev/llms.txt` and the
   two `.md` files load as plain text.
3. `aischemagate.com/pricing`: Free 100,000 / Pro **$199** (or $166/mo annually),
   25,000,000 / Enterprise "contact us". No $99, $299, 10,000 or 250,000 anywhere.
4. Click **Get an API key** → lands on `app.datacontractgate.com/auth/signup?product=aischemagate`.
   Page and browser tab say **AISchemaGate**, not DataContractGate.
5. Sign up with **email/password**, confirm the email, sign in.
6. In the app: sidebar says "AISchemaGate / by DataContractGate" and shows only
   Docs, Pricing, Dashboard, Contracts, Audit Log, Playground, Account.
   Catalog, Scorecard, Scaffold, Workbench, Stream Demo, Kafka Connect, Python SDK must **not** appear.
7. Account page: plan shows **AISchemaGate Developer**. Create an API key.
8. Click **Pricing** in the sidebar (no `?product` in the URL). Expect the
   **AISchemaGate** tiers, not DataContractGate's. This was a bug fixed today.

### B. Contract + SDK happy path (account #1)
1. Contracts → **Generate from Sample** must be usable on the free plan (no lock).
   Paste 5 sample invoice outputs, e.g.
   `{"invoice_id":"INV-000123","vendor":"Acme","amount":1200.5,"currency":"USD","due_date":"2026-11-01"}`,
   generate, then tighten: `invoice_id` pattern `^INV-[0-9]{6}$`, `currency` enum
   USD/EUR/GBP, `amount` min 0. Name it `extract_invoice_v1`. Promote to **stable**.
2. Inside the contract: Kafka and Kinesis tabs must **not** appear; Outbound should.
3. Python: `pip install aischemagate` in a clean venv. With
   `CONTRACTGATE_API_KEY` set, validate without an LLM call:
   ```python
   from aischemagate import LLMGate
   gate = LLMGate(api_key=KEY, contract_id="extract_invoice_v1", llm=object(),
                  base_url="https://api.datacontractgate.com")
   ok  = gate.validate_only({"invoice_id":"INV-000123","vendor":"Acme","amount":10,"currency":"USD","due_date":"2026-11-01"})
   bad = gate.validate_only({"invoice_id":"X","vendor":"Acme","amount":-5,"currency":"BTC","due_date":"2026-11-01"})
   ```
   Expect `ok.ok == True`; `bad.ok == False` with violations for invoice_id,
   amount and currency. Repeat once **without** `base_url` (0.3.0 defaults to the
   app host, which proxies), and once with the contract's **UUID** instead of its name.
4. TypeScript: same with `npm i @nightmoose/aischemagate` and `new LLMGate({...})`.
5. If you have an OpenAI or Anthropic key: run `gate.completion(..., auto_repair=True)`
   per `aischemagate.dev/docs/llm-output-validation` §3 and record `result.attempts`.
6. Audit Log in the dashboard shows the validations; the failed one appears in
   Quarantine (payload body is **not** stored on Free; that is expected).
7. Usage on the Account page goes up by the number of validations.

### C. Limits (account #1, free plan)
1. **Rate limit:** send ~30 requests in one second with the same key. Expect some
   **HTTP 429**. At ~5/second, expect none.
2. **Body cap:** POST a ~1.5 MB JSON body to `/egress/{id}`. Expect **413**.
3. Do **not** try to exhaust the 100,000 monthly quota.

### D. Billing — Pro checkout at $199 (account #1)
This also confirms the Stripe price IDs and secret key in Vercel.
1. Pricing → Pro → **Start 30-day trial (monthly)**. Stripe Checkout must open,
   product "Pro — DataContractGate & AISchemaGate", **$199/month after a 30-day trial**.
   Note the exact text. If it shows **$299**, or an error popup like
   "Expired API Key provided", stop and report. That means Vercel is wrong.
2. Toggle **annual** and start again: **$1,992/year**.
3. Complete checkout with a real card (trial, $0 today), return to the app.
   Account shows **AISchemaGate Pro**, status trialing; upsell locks are gone
   (Quarantine replay, Visual Builder).
4. Rate limit on that key is now 100 req/s: ~30 in one second should **not** 429.
   Allow 60 seconds after upgrading for the key cache to refresh.
5. **Cancel** from Account → Billing (Stripe portal) **before the trial ends**.
   After cancelling, the account returns to **AISchemaGate Developer**, not "Free".

### E. DataContractGate regression (fresh account #2)
1. `https://datacontractgate.com`: pricing shows Free 100K / **Pro $199** / 25M; no "Growth".
2. Sign up from **datacontractgate.com** (no `?product`). Branding is
   **DataContractGate** and the full sidebar is present.
3. Account plan shows **Free**. Pricing shows the DataContractGate table with **Pro**.
4. Visit any app page with `?product=aischemagate`, then `?product=datacontractgate`.
   Branding should switch, then switch back.

### F. Misc
1. `https://api.datacontractgate.com/health` → 200, valid certificate.
2. `https://api.datacontractgate.com/openapi.json` → title "DataContractGate Ingest API",
   server `https://api.datacontractgate.com`. No `contractgate.io` anywhere.
3. The Terms page (`/terms`) "Current prices" states $199 / $166 / $1,992 for both products.
4. Anywhere you see the bare name "ContractGate" (not "DataContractGate") in
   product-facing text, report it with the URL.

## Known and expected (don't report these)
- GitHub sign-ups from aischemagate.com land on DataContractGate Free (same limits).
- Free plans don't store event bodies, so quarantined records can't be replayed.
- The "3 contracts" free limit is shown but not enforced yet.
- Old Zapier connector text says "ContractGate"; an update waits on Zapier's approval.
- SDK 0.3.0 defaults to `app.datacontractgate.com`; 0.3.1 will default to the api host.

## Report format
A list per section: ID (e.g. B3), PASS/FAIL/BLOCKED, one-line finding, evidence.
Put FAILs first.
