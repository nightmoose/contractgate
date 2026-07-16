/**
 * Pure org-role guard logic (RFC-085 — org admin / team management).
 *
 * No imports, no I/O — deliberately separated from `lib/org/authz.ts` (which
 * needs the Supabase server/service clients) so this file can be compiled
 * and unit tested standalone with plain `tsc` + `node`, no path-alias or
 * test-runner setup required. See `roleGuard.test.ts`.
 */

export type Role = "owner" | "admin" | "member";
export const ROLES: Role[] = ["owner", "admin", "member"];

export interface GuardInput {
  callerRole: Role;
  targetCurrentRole: Role;
  /** Undefined for a removal (DELETE); the requested role for a PATCH. */
  targetNewRole?: Role;
  liveOwnerCount: number;
}

export type GuardResult =
  | { ok: true }
  | { ok: false; status: 403 | 409; error: string };

/**
 * Decide whether a role-change or removal is allowed, per RFC-085's owner
 * guardrails:
 *   - Only an owner may target a current owner, or promote someone to owner.
 *   - The mutation must never drop the org to zero live owners.
 */
export function assertGuardedChange(input: GuardInput): GuardResult {
  const { callerRole, targetCurrentRole, targetNewRole, liveOwnerCount } = input;

  const targetsOwner = targetCurrentRole === "owner";
  const promotesToOwner = targetNewRole === "owner";
  if ((targetsOwner || promotesToOwner) && callerRole !== "owner") {
    return { ok: false, status: 403, error: "only an owner can manage owners" };
  }

  // Does this change remove a live owner slot (demotion or removal)?
  const removesOwnerSlot = targetsOwner && targetNewRole !== "owner";
  if (removesOwnerSlot && liveOwnerCount <= 1) {
    return { ok: false, status: 409, error: "cannot_remove_last_owner" };
  }

  return { ok: true };
}
