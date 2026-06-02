# RFC-039 — Supabase-JWT Auth Path for Dashboard Traffic

**Status:** Accepted  
**Date:** 2026-05-16  
**Addresses:** REVIEW-2026-05-16-saas-readiness P0-1

---

## Problem

The dashboard sends `x-api-key: ${NEXT_PUBLIC_API_KEY}` on every Rust backend
call. `NEXT_PUBLIC_API_KEY` is a build-time env var inlined by `next build`.

Two failure modes:

1. **Current bug (Alex's screenshot):** `NEXT_PUBLIC_API_KEY` is empty → header
   omitted → backend returns 401 → SWR fails silently → contracts list and
   audit log appear empty despite a successful Supabase login.

2. **Multi-tenancy hole (future):** If the key _is_ set, every logged-in user
   authenticates as the same shared key. Writes are scoped to that key's
   `org_id` regardless of who is actually logged in.

API keys must remain for server-to-server traffic (Kafka connectors, CLI, SDKs).
Browser sessions must use the logged-in user's identity.

---

## Decision

**Option A** (recommended by the review doc): add a Supabase-JWT verification
path to the Rust backend.

- `require_api_key` renamed to `require_auth`.
- New branch: if `Authorization: Bearer <token>` is present, verify the JWT
  using `SUPABASE_JWT_SECRET` (HS256), extract `sub` (Supabase user UUID),
  look up the user's primary org membership, and inject a `ValidatedKey` into
  request extensions — identical struct the rest of the codebase already reads.
- Existing `x-api-key` path unchanged (DB-backed keys + legacy env-var key).
- Dashboard `apiFetch` gains a `setApiSession(token)` API (mirrors existing
  `setApiOrgId`). `OrgProvider` calls it after `supabase.auth.getSession()`.
  When a session token is present the dashboard sends
  `Authorization: Bearer <session.access_token>` instead of `x-api-key`.

---

## Implementation

### Rust backend

**New env var:** `SUPABASE_JWT_SECRET` — the JWT secret from
`Supabase → Settings → API → JWT Settings`. Required in production; if unset
the JWT branch is skipped (dev mode falls through to existing logic).

**New crate:** `jsonwebtoken` (already in Rust ecosystem; HS256 verify).

**New file:** `src/jwt_auth.rs`

```
verify_supabase_jwt(token: &str, secret: &str, db: &PgPool)
  -> Result<ValidatedKey, JwtAuthError>
```

Steps:
1. Decode + verify signature (HS256, `SUPABASE_JWT_SECRET`).
2. Check `exp` claim — reject expired tokens.
3. Extract `sub` as a UUID (Supabase user id).
4. Query `org_memberships` for the user's primary org (oldest live membership):
   ```sql
   SELECT org_id FROM org_memberships
   WHERE user_id = $1 AND deleted_at IS NULL
   ORDER BY created_at ASC LIMIT 1
   ```
5. Return `ValidatedKey { api_key_id: Uuid::nil(), user_id, org_id, .. }`.
   `api_key_id = Uuid::nil()` signals "JWT-authed session" to audit handlers.

**Modified:** `src/main.rs` — `require_api_key` → `require_auth`:

```
1. Check Authorization: Bearer <token>
   → if SUPABASE_JWT_SECRET set: call verify_supabase_jwt
   → Ok(vk) → inject + continue
   → Err → 401 (don't fall through to x-api-key; avoids confused-deputy)
2. Check x-api-key (DB-backed, then legacy env-var) — unchanged
3. Dev-mode passthrough (state.api_key.is_empty() + no JWT secret) — unchanged
```

**AppState** gains `supabase_jwt_secret: Option<String>`.

### Dashboard

**`dashboard/lib/api.ts`**

- Add module-level `let _apiSession: string | null = null`.
- Export `setApiSession(token: string): void`.
- In `apiFetch`: when `_apiSession` is set (and no `API_KEY`), add header
  `Authorization: Bearer ${_apiSession}` instead of `x-api-key`.
- `exportOdcs` gets the same treatment (it builds headers manually).

**`dashboard/lib/org.ts` / `OrgProvider`**

After `supabase.auth.getSession()` resolves:
```ts
if (session) {
  setApiSession(session.access_token);
  setApiOrgId(org.org_id);
}
```

Wire `supabase.auth.onAuthStateChange` to call `setApiSession(session?.access_token ?? "")` on every state change so token refreshes propagate automatically.

### Env changes

**`.env.example`** — add:
```
# Supabase JWT secret (Settings → API → JWT Settings → JWT Secret).
# Required for dashboard login to authenticate against the Rust backend.
SUPABASE_JWT_SECRET=your-supabase-jwt-secret-here
```

**`dashboard/.env.local`** — add comment:
```
# NEXT_PUBLIC_API_KEY is now only needed for server-to-server (CLI/SDK) traffic.
# Browser sessions authenticate via the Supabase JWT (RFC-039).
```

---

## What this does NOT change

- API key validation path (`x-api-key`) is untouched — zero breakage for
  Kafka connectors, CLI, SDKs.
- `ValidatedKey` struct shape is unchanged — all downstream handlers work as-is.
- RLS on the Supabase side is not affected (the Rust backend uses the service
  role; JWT verification is handled in Rust, not via Supabase RLS).

---

## Out of scope

- Token refresh on the Rust side (Supabase issues short-lived JWTs; the
  dashboard Supabase client refreshes automatically and `onAuthStateChange`
  updates `_apiSession`).
- P0-2 (RLS gaps on `contract_versions` etc.) — separate RFC/migration.
- P0-4 (server-side key creation route) — separate RFC.

---

## Migration / rollout

No DB migration required. Deploy order:

1. Set `SUPABASE_JWT_SECRET` in Fly secrets before deploying the new binary.
2. Deploy Rust backend.
3. Deploy dashboard with updated `apiFetch`.
4. Verify: log in → contracts list populates → audit log populates.
5. After verifying, `NEXT_PUBLIC_API_KEY` can be removed from Vercel env vars
   (browser traffic no longer needs it). Keep the DB-backed key for CLI.
