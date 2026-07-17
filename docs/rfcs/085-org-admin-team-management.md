# RFC-085 — Org admin / team management

**Status:** Implemented
**Date:** 2026-07-16
**Author:** Claude (proposes) → Sonnet (executes)
**Branch:** `nightly-maintenance-2026-07-16-rfc085-team-admin` (Alex creates/commits — no agent git)
**Migration:** none (reuses existing `org_memberships` / `org_invites` schema)

---

## Problem

The dashboard already lists org members and sends/revokes invites (RFC-001, on the
Account page), but there is **no way to manage existing members**: you can't change
a member's role or remove them. There is also **no role-gating in the UI** — the
member/invite controls render for everyone; today only `org_invites` RLS actually
blocks a non-admin from inserting (they get an opaque RLS error, not a clean "you
don't have permission").

This is the last obvious gap in self-serve org administration, and it's a common
first question on a paid multi-seat pilot ("how do I add/remove my teammates?").

## Goal (this RFC)

Minimal, correct team administration on the **existing Account page surface**:

1. **Role-gate** the member + invite section: management controls (invite, revoke,
   change-role, remove) render only for `owner`/`admin`. `member` sees a read-only
   roster.
2. **Change a member's role** (`owner` ⇄ `admin` ⇄ `member`).
3. **Remove a member** (soft-delete: set `org_memberships.deleted_at`).
4. **Guardrails:** never remove or demote the **last remaining `owner`**; only an
   `owner` may promote someone to `owner` or change another `owner`'s role.

**Non-goals (deferred):** a dedicated `/team` route (keep it on Account for now);
transfer-ownership wizard; email delivery of invites (still copy-link/RLS as today);
audit-logging of admin actions. Note these as follow-ups.

## Existing pieces to reuse (do not rebuild)

- `dashboard/app/account/page.tsx` — `OrgMember` / `OrgInvite` types, `members` /
  `invites` state, `loadOrgData()`, `handleSendInvite`, `handleRevokeInvite`, and
  the members/invites render block. `org.role` is already in scope for gating.
- `dashboard/app/api/org/members/route.ts` — the **canonical pattern**: service-role
  client, resolve caller via `supabase.auth.getUser()`, verify caller membership in
  the target org, then act. Copy this shape for the mutation route.
- `org_memberships` schema (migration 007 + 012): `(org_id, user_id, role
  owner|admin|member, joined_at, deleted_at)`, live index `where deleted_at is null`.
- Service-role RLS: both tables have `service role full access` — mutations go
  through a service-role API route (there is **no** authenticated UPDATE policy on
  `org_memberships`, so client-side updates would fail; the route is required).

## Design

### API — `dashboard/app/api/org/members/route.ts` (extend the existing file)

Add two handlers alongside the current `GET`:

- **`PATCH`** — change a member's role.
  Body: `{ org_id, user_id, role: "owner"|"admin"|"member" }`.
- **`DELETE`** — remove a member (soft delete).
  Body: `{ org_id, user_id }`.

Both handlers, before mutating (server-side, using the service client):

1. `getUser()` → 401 if not signed in.
2. Load caller's live membership in `org_id`; require `role in (owner, admin)` → 403.
3. Validate `org_id` / `user_id` as UUIDs; validate `role` against the allowlist.
4. **Owner guardrails:**
   - Only an `owner` may set a target to `owner`, or change a member who is currently
     `owner` (admins can manage members/admins but not owners) → 403 otherwise.
   - Compute live `owner` count for the org. Block the mutation if it would drop the
     org to **zero owners** (removing the last owner, or demoting the last owner) →
     409 `{ error: "cannot_remove_last_owner" }`.
5. Perform the change with the service client:
   - PATCH: `update org_memberships set role = $role where org_id and user_id and deleted_at is null`.
   - DELETE: `update org_memberships set deleted_at = now() where org_id and user_id and deleted_at is null`.
6. Return `{ ok: true }` (or the updated row). Non-existent/already-removed target → 404.

Keep the file focused; if `route.ts` pushes past ~150 lines, factor the shared
"resolve caller + assert manager + load owner set" preamble into a small helper
(e.g. `dashboard/lib/org/authz.ts`) and note it in the PR.

### UI — `dashboard/app/account/page.tsx`

- Gate the whole management affordance on `const canManage = org?.role === "owner" || org?.role === "admin"`.
  - `member`: read-only roster (name/email/role), no invite form, no row actions.
  - `admin`/`owner`: existing invite form + revoke, plus per-member row actions.
- Per-member row (for `canManage`, excluding the current user's own row):
  - **Role** dropdown (`member`/`admin`, plus `owner` only when the *actor* is an
    owner) → calls `PATCH /api/org/members`.
  - **Remove** button (confirm dialog) → calls `DELETE /api/org/members`.
  - Disable owner-targeting actions when the actor is only an `admin`.
  - After success, call `loadOrgData(org.org_id)` to refresh (existing pattern).
- Surface the 409 last-owner and 403 permission errors as inline messages (reuse the
  `inviteError`-style pattern), not raw JSON.
- Do **not** let a user remove or demote themselves out of the last-owner slot (belt
  and suspenders with the server guard).

## Security notes

- All mutations are **server-side, service-role, owner/admin-gated** — the client
  never updates `org_memberships` directly (no authenticated UPDATE policy exists,
  by design). This matches the `/api/org/members` GET precedent.
- The **last-owner guard lives on the server** (authoritative); the UI guard is only
  UX. Both must exist.
- No new RLS policy and no schema change → no advisor delta, no
  `EXPECTED_MIGRATION_COUNT` bump, no new CI sentinel.

## Testing

- Manual matrix (dev-no-auth or two seeded orgs): owner promotes/demotes/removes;
  admin can manage members but not owners; member sees read-only; last-owner
  removal/demotion is blocked (409); cross-org `org_id` is rejected (403).
- `cd dashboard && npm run build` (tsc + build) must pass — CI gates on this.
- If a shared authz helper is added, a small unit test for the last-owner /
  owner-only-promotes logic is welcome (pure function).

## Docs

- New: `docs/team-management-reference.md` — roles (owner/admin/member) and what each
  can do, how to invite/change-role/remove, the last-owner rule. (Per CLAUDE.md:
  user-facing surface with no existing doc → add one.)
- Link it from the account/team section if there's an obvious spot.

## Rollout

1. No migration to apply. Ship with the normal dashboard deploy (Vercel).
2. Verify on a two-member org in prod after deploy: promote/demote/remove + last-owner
   block.

## Out of scope / follow-ups

- Dedicated `/team` route (lift the section out of Account).
- Transfer-ownership flow; admin-action audit logging; emailed invites.
