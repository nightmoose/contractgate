/**
 * GET /api/org/members?org_id=<uuid>
 *
 * Returns the membership list for an org, joined with `auth.users.email`.
 *
 * Why: the dashboard's account page wants to render member emails, but
 * `auth.users` is not exposed via RLS (it's in the `auth` schema, not
 * `public`). We resolve the join here using the service-role key, after
 * confirming the caller is a member of the org they're asking about.
 *
 * Response: { members: [{ user_id, email, role, joined_at }] }
 *
 * Sign-off: closes the "Member emails not shown in member list" gap from
 * docs/rfcs/001-org-scoped-tenancy-verification.md.
 */

import { createClient } from "@/lib/supabase/server";
import { createClient as createServiceClient } from "@supabase/supabase-js";
import { NextResponse } from "next/server";

function getServiceClient() {
  return createServiceClient(
    process.env.NEXT_PUBLIC_SUPABASE_URL!,
    process.env.SUPABASE_SERVICE_ROLE_KEY!
  );
}

interface MembershipRow {
  user_id: string;
  role: "owner" | "admin" | "member";
  joined_at: string;
}

export async function GET(request: Request) {
  const { searchParams } = new URL(request.url);
  const orgId = searchParams.get("org_id");

  if (!orgId || !/^[0-9a-f-]{36}$/i.test(orgId)) {
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
