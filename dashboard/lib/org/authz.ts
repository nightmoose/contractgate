/**
 * Shared authorization helpers for org member management routes
 * (RFC-085 — org admin / team management).
 *
 * Centralizes the "resolve caller + assert manager + load owner set"
 * preamble used by the PATCH/DELETE handlers in
 * `app/api/org/members/route.ts`. The last-owner / owner-only-promotes
 * guard itself lives in `./roleGuard` (pure, no imports) so it can be unit
 * tested standalone — re-exported here for convenience.
 */

import { createClient } from "@/lib/supabase/server";
import {
  createClient as createServiceClient,
  type SupabaseClient,
} from "@supabase/supabase-js";
import { assertGuardedChange, ROLES, type Role } from "./roleGuard";

export { assertGuardedChange, ROLES, type Role };

export function isUuid(v: unknown): v is string {
  return typeof v === "string" && /^[0-9a-f-]{36}$/i.test(v);
}

export function getServiceClient(): SupabaseClient {
  return createServiceClient(
    process.env.NEXT_PUBLIC_SUPABASE_URL!,
    process.env.SUPABASE_SERVICE_ROLE_KEY!
  );
}

export interface ManagerContext {
  callerId: string;
  callerRole: Role;
  svc: SupabaseClient;
}

export type ManagerResult =
  | { ok: true; ctx: ManagerContext }
  | { ok: false; status: 401 | 403; error: string };

/**
 * Resolve the signed-in caller and verify they hold a live owner/admin
 * membership in `orgId`. Returns a discriminated result so route handlers
 * can short-circuit on `.ok === false` with the right status code.
 */
export async function resolveCallerAndAssertManager(
  orgId: string
): Promise<ManagerResult> {
  const supabase = await createClient();
  const {
    data: { user },
  } = await supabase.auth.getUser();
  if (!user) return { ok: false, status: 401, error: "not signed in" };

  const svc = getServiceClient();
  const { data: caller } = await svc
    .from("org_memberships")
    .select("role")
    .eq("org_id", orgId)
    .eq("user_id", user.id)
    .is("deleted_at", null)
    .maybeSingle();

  if (!caller) {
    return { ok: false, status: 403, error: "not a member of this org" };
  }
  if (caller.role !== "owner" && caller.role !== "admin") {
    return { ok: false, status: 403, error: "insufficient permissions" };
  }

  return {
    ok: true,
    ctx: { callerId: user.id, callerRole: caller.role as Role, svc },
  };
}

/** Live (non-deleted) owner count for an org — used for the last-owner guard. */
export async function countLiveOwners(
  svc: SupabaseClient,
  orgId: string
): Promise<number> {
  const { count } = await svc
    .from("org_memberships")
    .select("user_id", { count: "exact", head: true })
    .eq("org_id", orgId)
    .eq("role", "owner")
    .is("deleted_at", null);
  return count ?? 0;
}
