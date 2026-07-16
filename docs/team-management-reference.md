# Team management reference

Org roles, what each can do, and how invite / change-role / remove work.
Covers the **Team Members** section of the Account page
(`dashboard/app/account/page.tsx`) and its backing API
(`dashboard/app/api/org/members/route.ts`). Implements RFC-085.

## Roles

| Role | Can view roster | Can invite | Can change roles | Can remove members |
|---|---|---|---|---|
| `member` | Yes (read-only) | No | No | No |
| `admin` | Yes | Yes (member/admin) | Yes, for `member`/`admin` targets | Yes, for `member`/`admin` targets |
| `owner` | Yes | Yes (member/admin) | Yes, including `owner` targets | Yes, including `owner` targets |

- A `member` sees a read-only roster: no invite form, no per-row actions.
- `admin` and `owner` both see the invite form, revoke controls, and
  per-member row actions (role dropdown + Remove).
- **Only an owner may target an owner** — promote someone to `owner`, change
  an existing owner's role, or remove an owner. If an admin attempts this,
  the request is rejected with `403`.
- No one ever sees management actions on their own row (self-service role
  change / self-removal isn't supported from this UI).

## Inviting a member

Unchanged from the existing flow: `admin`/`owner` use the invite form on the
Account page to send an invite by email with a role of `member` or `admin`
(only an `owner` can invite as `admin`). Invites are still copy-link/RLS based
— no email delivery is sent by ContractGate itself.

## Changing a member's role

`admin`/`owner` use the role dropdown next to a member's row. Selecting a new
role calls `PATCH /api/org/members` with `{ org_id, user_id, role }`.

Server-side rules (in `dashboard/lib/org/authz.ts`, enforced in the API route
— this is the authoritative check; the UI disabling controls is convenience
only):

1. Caller must be signed in and hold a live `owner`/`admin` membership in the
   org, or the request is rejected (`401`/`403`).
2. `role` must be one of `owner`, `admin`, `member` (`400` otherwise).
3. Only an `owner` may set a target to `owner`, or change the role of a
   member who currently *is* an `owner`. An `admin` attempting either gets
   `403`.
4. The change is blocked with `409 { error: "cannot_remove_last_owner" }` if
   it would leave the org with **zero live owners** (see below).
5. A non-existent or already-removed target returns `404`.
6. On success: `{ ok: true }`, and the dashboard reloads the roster.

## Removing a member

`admin`/`owner` use the **Remove** button on a member's row (confirmation
dialog first). This calls `DELETE /api/org/members` with
`{ org_id, user_id }`, which **soft-deletes** the membership
(`org_memberships.deleted_at = now()`) rather than hard-deleting the row.
Removed members immediately lose access; their row drops out of the live
roster (`deleted_at is null` is the "live membership" filter used
everywhere).

The same rules as role changes apply: only an owner can remove an owner, and
removal is blocked if it would remove the org's last live owner.

## The last-owner rule

An org must always have **at least one live owner**. Both the role-change and
removal endpoints compute the current count of live owners before mutating,
and reject the change with `409 cannot_remove_last_owner` if it would drop
that count to zero — whether that's the last owner demoting themselves to
`admin`/`member`, or being removed outright. There is no override; to drop
below one owner, first promote a different member to `owner`.

## Non-goals (deferred, not in this release)

- A dedicated `/team` route (management stays on the Account page for now).
- A transfer-ownership wizard.
- Emailed invites (still copy-link/RLS as today).
- Audit-logging of admin actions (who changed whose role, when).
