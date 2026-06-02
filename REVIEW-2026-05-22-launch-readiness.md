# ContractGate Public-Launch Readiness Review — 2026-05-22

Deep audit of the backend (Rust/Axum), dashboard (Next.js 15), Supabase
migrations, CI, and docs. Question asked: **is ContractGate ready for a full
public launch?**

## Verdict

**Not yet. Do not launch publicly until the four blockers below are fixed.**

The validation engine, contract model, CI, and feature surface are in good
shape. The launch risk is concentrated in **multi-tenant isolation**: RFC-040
correctly closed the Supabase REST path with org-scoped RLS, but the **Rust
backend is the primary data path and it connects as the service role, which
bypasses RLS entirely**. The by-ID contract and version routes do not
re-implement org scoping in application code — so the RLS fix gives a false
sense of safety. A public launch invites untrusted tenants onto exactly the
surface that is unprotected.

Each finding below has a matching RFC (047–058) with the file, the defect, and
the fix.

---

## P0 — Launch blockers

### B1. Cross-org IDOR on every by-ID contract and version route — RFC-047

**Files:** `src/main.rs:327-464`, `src/storage.rs:240-742`

The backend connects as the Supabase service role (`DATABASE_URL`), which
bypasses RLS unconditionally. Yet `get_contract_identity`, `patch_contract_identity`,
`delete_contract`, `create_version`, `patch_version_yaml`, `promote_version`,
`deprecate_version`, `delete_version`, and `get_version` take only a UUID — no
`org_id` filter. Any holder of *any* valid JWT or API key can read, modify,
promote, or soft-delete *any other tenant's* contracts and versions by
enumerating UUIDs. `list_contracts` / `audit` / `stats` are org-scoped; the
single-resource routes are not. This is a Broken-Object-Level-Authorization
hole across the whole `/contracts/{id}/...` surface.

### B2. `x-org-id` header is trusted — tenant impersonation — RFC-048

**File:** `src/main.rs:287-297`

`org_id_from_req` falls back to the client-supplied `x-org-id` header whenever
no `ValidatedKey` is present. The legacy env-var `API_KEY` path produces no
`ValidatedKey`. So any client holding the shared env-var key can set
`x-org-id` to an arbitrary org and read/write that org's data on every
org-scoped route (`list_contracts`, `audit`, `stats`, `create_contract`,
`deploy`). Combined with B1, tenant isolation is effectively absent.

### B3. SSRF via redirect in `/contracts/infer/url` — RFC-049

**File:** `src/infer_url.rs:115-119`

`check_ssrf` resolves and pins the *initial* host's IP, but the reqwest client
is built with no redirect policy. reqwest follows up to 10 redirects by
default, and a redirect to a new host (`http://169.254.169.254/...`) does a
fresh, unchecked connection. A public attacker-controlled URL can 302 to the
cloud metadata endpoint and exfiltrate instance credentials.

### B4. CORS allows any origin, method, and header — RFC-050

**File:** `src/main.rs:847-850`

`CorsLayer::new().allow_origin(Any).allow_methods(Any).allow_headers(Any)`.
For a public multi-tenant API with browser-delivered bearer tokens, any
website can drive the API with a victim's credentials. Needs an explicit
dashboard-origin allowlist, with a separate permissive layer for the genuinely
public endpoints.

---

## P1 — High (fix before launch)

### H1. API-key cache stores DB errors as hard auth failures — RFC-051

**File:** `src/api_key_auth.rs:118-138, 167-194`

`verify_against_db` collapses a DB error into `Err(())`, and `validate` caches
that `Err` for 60 s. A transient database blip locks a valid key out for a
full minute. Errors must never be cached; only definitive "key not found"
should be. Same RFC: cache is keyed by the raw plaintext key (secrets sit in
the heap, leak in core dumps) and has no background eviction (unbounded
growth); `last_used_at` update errors are silently discarded.

### H2. Supabase JWKS fetched once at startup, never refreshed — RFC-052

**File:** `src/main.rs:1192-1214`, `src/jwt_auth.rs`

When Supabase rotates its JWT signing keys, every dashboard session breaks
until the backend is manually restarted. Needs a periodic refresh and a
refresh-on-unknown-`kid` fallback.

### H3. `/health` returns hardcoded `"ok"` — RFC-053

**File:** `src/main.rs:681-687`

The health endpoint never touches the database. A backend with a dead DB pool
still reports healthy, so the Fly/load-balancer health check cannot drain a
broken instance. Needs a `SELECT 1` probe.

### H4. Lock-poison `.expect()` panics take down the whole process — RFC-054

**Files:** `src/main.rs:146-156`, `src/api_key_auth.rs:97-101`,
`src/rate_limit.rs:141`

The contract cache, API-key cache, and rate-limit buckets all `.expect()` on
mutex/RwLock poison. For a single-tenant tool that is fine; for a multi-tenant
gateway, one panicked thread poisons the lock and crashes the process for
every customer. Recover the inner value instead.

### H5. CI `sqlx-cli` pinned to 0.7.4 while the crate is `sqlx` 0.8 — RFC-055

**File:** `.github/workflows/ci.yml` (`migrations-check`, `Install sqlx-cli`)

`Cargo.toml` is on `sqlx = "0.8"`; CI installs `sqlx-cli --version 0.7.4`. The
`.sqlx/` metadata format changed between 0.7 and 0.8, so `cargo sqlx prepare
--check` is either failing or silently meaningless. Same job: the migration
sentinel still checks only migration 009 ("Expected all 9 migrations") though
26 migrations now exist.

---

## P2 — Medium

- **M1 — API keys inserted directly from the browser (RFC-056).** No
  server-side issuance route; the client is trusted to hash. Route key
  creation through a Next.js handler that hashes server-side.
- **M2 — Docs gaps (RFC-057).** No `docs/*-reference.md` for kinesis-ingress,
  CSV/URL inference, public catalog, JWT auth, the `date` type (RFC-044), or
  plan gating (RFC-045) — a CLAUDE.md rule violation for user-facing surfaces.
  No consolidated RFC status index (46 RFCs, shipped-vs-draft unclear).
  CLAUDE.md still documents a `code-review-graph` MCP that is not connected.
  Legacy columns `contracts.version/active/yaml_content` should be dropped or
  annotated.

## P3 — Polish

- README headline claims `<10 µs p99` latency; CLAUDE.md targets `<15 ms p99`.
  Reconcile the public claim with a measured, reproducible number before
  launch — an unverifiable performance claim is a credibility risk.
- `src/stream_demo.rs:249-251` panics at startup if a bundled demo scenario
  has bad YAML.
- Rate-limit `DashMap` has no eviction — minor unbounded growth.

---

## What is in good shape

- Validation engine architecture: compiled-contract cache keyed by
  `(contract_id, version)`, served as `Arc` clones off a read lock — no
  allocation on the hot path. The `<15 ms p99` budget is sound by design.
- CI is comprehensive: fmt/clippy/test, dashboard build, Docker, migrations
  apply, `cargo audit` + `cargo deny` + Trivy, compose smoke tests, gated
  Fly deploy.
- RLS migrations are correct *for the REST path*; `get_my_org_ids()` avoids
  the PG-42P17 recursion.
- Body-size limits, per-key rate limiting, and playground auth (RFC-042/043)
  are correctly wired.

---

## Recommended order of operations

1. **B1 + B2 together** — they are the same isolation hole from two angles;
   ship as one bundle so org scoping lands consistently.
2. **B3, B4** — small, independent, ship standalone.
3. **H1–H5** — the abuse/availability hardening bundle; 2–3 nightly runs.
4. **M1, M2** — before the launch announcement, not before first pilot.
5. **RFC-058 (roadmap)** — informs sequencing of everything after launch.

RFCs 047–058 in `docs/rfcs/` carry the detail. Implement one per branch per
the project's RFC-first rule; do not bundle unrelated RFCs into one PR.
