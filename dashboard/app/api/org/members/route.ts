/**
 * /api/org/members
 *
 * GET    ?org_id=<uuid>                          — list members (any live member).
 * PATCH  { org_id, user_id, role }                — change a member's role.
 * DELETE { org_id, user_id }                       — soft-remove a member.
 *
 * GET: joined with `auth.users.email`.
 *
 * Why: the dashboard's account page wants to render member emails, but
 * `auth.users` is not exposed via RLS (it's in the `auth` schema, not
 * `public`). We resolve the join here using the service-role key, after
 * confirming the caller is a member of the org they're asking about.
 *
 * PATCH/DELETE (RFC-085): owner/admin only, service-role writes (there is no
 * authenticated UPDATE policy on `org_memberships` by design). Shared
 * "resolve caller + assert manager + owner count" logic lives in
 * `@/lib/org/authz` so the last-owner / owner-only-promotes guard can be
 * unit tested without touching the DB.
 *
 * Response: { members: [{ user_id, email, role, joined_at }] } | { ok: true }
 *
 * Sign-off: GET closes the "Member emails not shown in member list" gap from
 * docs/rfcs/001-org-scoped-tenancy-verification.md. PATCH/DELETE close
 * docs/rfcs/085-org-admin-team-management.md.
 */

import { NextResponse } from "next/server";
import { createClient } from "@/lib/supabase/server";
import {
  ROLES,
  type Role,
  isUuid,
  getServiceClient,
  resolveCallerAndAssertManager,
  countLiveOwners,
  assertGuardedChange,
} from "@/lib/org/authz";

interface MembershipRow {
  user_id: string;
  role: Role;
  joined_at: string;
}

export async function GET(request: Request) {
  const { searchParams } = new URL(request.url);
  const orgId = searchParams.get("org_id");

  if (!isUuid(orgId)) {
    return NextResponse.json({ error: "invalid org_id" }, { status: 400 });
  }

  // 1. Resolve caller and verify they belong to the requested org.
  const supabase = await createClient();
  const {
    data: { user },
  } = await supabase.auth.getUser();
  if (!user) {
    return NextResponse.json({ error: "not signed in" }, { status: 401 });
  }

  const svc = getServiceClient();

  const { data: caller } = await svc
    .from("org_memberships")
    .select("role")
    .eq("org_id", orgId)
    .eq("user_id", user.id)
    .is("deleted_at", null)
    .maybeSingle();

  if (!caller) {
    return NextResponse.json({ error: "not a member of this org" }, { status: 403 });
  }

  // 2. Pull memberships (live only — soft-deleted rows are hidden).
  const { data: memberships, error: mErr } = await svc
    .from("org_memberships")
    .select("user_id, role, joined_at")
    .eq("org_id", orgId)
    .is("deleted_at", null)
    .order("joined_at", { ascending: true })
    .returns<MembershipRow[]>();

  if (mErr || !memberships) {
    return NextResponse.json({ error: "failed to load members" }, { status: 500 });
  }

  // 3. Fan out to auth.admin.getUserById for each user (bounded by org size,
  //    typically <50). The Supabase admin API doesn't have a batch endpoint,
  //    but the calls run in parallel so latency is one-RTT.
  const enriched = await Promise.all(
    memberships.map(async (m) => {
      const { data: u } = await svc.auth.admin.getUserById(m.user_id);
      return {
        user_id: m.user_id,
        email: u?.user?.email ?? null,
        role: m.role,
        joined_at: m.joined_at,
      };
    })
  );

  return NextResponse.json({ members: enriched });
}

/**
 * PATCH — change a member's role. Body: { org_id, user_id, role }.
 * Owner/admin only; see @/lib/org/authz for the guardrail logic.
 */
export async function PATCH(request: Request) {
  const body = await request.json().catch(() => null);
  const orgId = body?.org_id;
  const targetUserId = body?.user_id;
  const newRole = body?.role;

  if (!isUuid(orgId) || !isUuid(targetUserId)) {
    return NextResponse.json({ error: "invalid org_id or user_id" }, { status: 400 });
  }
  if (!ROLES.includes(newRole)) {
    return NextResponse.json({ error: "invalid role" }, { status: 400 });
  }

  const manager = await resolveCallerAndAssertManager(orgId);
  if (!manager.ok) {
    return NextResponse.json({ error: manager.error }, { status: manager.status });
  }
  const { svc, callerRole } = manager.ctx;

  const { data: target } = await svc
    .from("org_memberships")
    .select("role")
    .eq("org_id", orgId)
    .eq("user_id", targetUserId)
    .is("deleted_at", null)
    .maybeSingle();
  if (!target) {
    return NextResponse.json({ error: "member not found" }, { status: 404 });
  }

  const liveOwnerCount = await countLiveOwners(svc, orgId);
  const guard = assertGuardedChange({
    callerRole,
    targetCurrentRole: target.role as Role,
    targetNewRole: newRole as Role,
    liveOwnerCount,
  });
  if (!guard.ok) {
    return NextResponse.json({ error: guard.error }, { status: guard.status });
  }

  const { error: updateErr } = await svc
    .from("org_memberships")
    .update({ role: newRole })
    .eq("org_id", orgId)
    .eq("user_id", targetUserId)
    .is("deleted_at", null);
  if (updateErr) {
    return NextResponse.json({ error: "failed to update role" }, { status: 500 });
  }

  return NextResponse.json({ ok: true });
}

/**
 * DELETE — soft-remove a member (sets deleted_at). Body: { org_id, user_id }.
 * Owner/admin only; see @/lib/org/authz for the guardrail logic.
 */
export async function DELETE(request: Request) {
  const body = await request.json().catch(() => null);
  const orgId = body?.org_id;
  const targetUserId = body?.user_id;

  if (!isUuid(orgId) || !isUuid(targetUserId)) {
    return NextResponse.json({ error: "invalid org_id or user_id" }, { status: 400 });
  }

  const manager = await resolveCallerAndAssertManager(orgId);
  if (!manager.ok) {
    return NextResponse.json({ error: manager.error }, { status: manager.status });
  }
  const { svc, callerRole } = manager.ctx;

  const { data: target } = await svc
    .from("org_memberships")
    .select("role")
    .eq("org_id", orgId)
    .eq("user_id", targetUserId)
    .is("deleted_at", null)
    .maybeSingle();
  if (!target) {
    return NextResponse.json({ error: "member not found" }, { status: 404 });
  }

  const liveOwnerCount = await countLiveOwners(svc, orgId);
  const guard = assertGuardedChange({
    callerRole,
    targetCurrentRole: target.role as Role,
    targetNewRole: undefined,
    liveOwnerCount,
  });
  if (!guard.ok) {
    return NextResponse.json({ error: guard.error }, { status: guard.status });
  }

  const { error: delErr } = await svc
    .from("org_memberships")
    .update({ deleted_at: new Date().toISOString() })
    .eq("org_id", orgId)
    .eq("user_id", targetUserId)
    .is("deleted_at", null);
  if (delErr) {
    return NextResponse.json({ error: "failed to remove member" }, { status: 500 });
  }

  return NextResponse.json({ ok: true });
}
