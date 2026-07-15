# Incident: Prod JWT CryptoProvider panic (2026-07-14)

**Status:** Resolved (Fly **v126**, manual deploy)  
**Severity:** P0 — dashboard unusable for signed-in users (contracts list dead)  
**Code fix:** PR #141 (`jsonwebtoken` crypto backend + deny note for RUSTSEC-2023-0071)

---

## User-visible symptoms

On `https://app.datacontractgate.com`:

- `Failed to load contracts: Failed to fetch`
- Chrome: CORS blocked + **502 Bad Gateway** on `contractgate-api.fly.dev/contracts`

**Misleading signal:** CORS.  
**Actual signal:** Fly **502** while machines crash-loop.

---

## Root cause

`jsonwebtoken` **9 → 10** with only `features = ["use_pem"]`. v10 requires an
explicit crypto backend (`rust_crypto` or `aws_lc_rs`). Without it, the first
Bearer JWT verify panics:

```text
Could not automatically determine the process-level CryptoProvider
```

`panic = "abort"` → SIGABRT → Fly restart ×10 → machines **stopped** → edge 502
**without** CORS headers → browser reports CORS. `/health` stayed 200 when a
machine was briefly up.

---

## Fix

```toml
jsonwebtoken = { version = "10", features = ["use_pem", "rust_crypto"] }
```

`deny.toml` ignores **RUSTSEC-2023-0071** (rsa / Marvin) with a documented
rationale: verify-only public JWKS path, no private key / decrypt. Do **not**
switch to `aws_lc_rs` without an explicit dual-provider plan — that path
reintroduced CryptoProvider panics in a follow-up attempt.

Prod restored on Fly **v126** (manual deploy after merge).

---

## Diagnose next time

```bash
fly logs -a contractgate-api   # panic / SIGABRT first
fly releases -a contractgate-api | head -5
curl -sS https://contractgate-api.fly.dev/health
curl -sS https://contractgate-api.fly.dev/ready
```

Do not start by editing CORS on 502 + crash-loop.

---

## Prevention

- One `fly deploy` at a time; confirm **new release number** before declaring fixed.
- Merged to `main` ≠ prod fixed.
- Keep a crypto feature on jsonwebtoken 10 forever.
