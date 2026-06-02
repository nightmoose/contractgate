/**
 * POST /api/invites/accept
 *
 * Redeems an org invite token (RFC-001).
 *
 * Body:
 *   { token: string }   — uuid generated when the invite was created
 *
 * Behaviour:
 *   1. Resolve the caller's session (Supabase server client). 401 if absent.
 *   2. Look up the invite by token via service-role (RLS would otherwise
 *      hide it — invite rows are owner/admin-readable per RFC-001).
 *   3. Validate the invite is live: not revoked, not accepted, not expired.
 *   4. Verify the signed-in user's email matches `invite.email` exactly.
 *      Sign-off #8 was "require explicit accept"; refusing on mismatch
 *      prevents an attacker who guesses a token from joining a different org.
 *   5. Insert org_memberships row (or no-op if already a member of that org).
 *   6. Stamp `accepted_at` on the invite row.
 *
 * Success: 200 { org_id, role }
 * Error:   401 / 404 / 410 / 409 with { error }
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

interface InviteRow {
  id: string;
  org_id: string;
  email: string;
  role: "owner" | "admin" | "member";
  expires_at: string;
  accepted_at: string | null;
  revoked_at: string | null;
}

export async function POST(request: Request) {
  let body: { token?: unknown };
  try {
    body = await request.json();
  } catch {
    return NextResponse.json({ error: "invalid json body" }, { status: 400 });
  }

  const token = typeof body.token === "string" ? body.token : null;
  if (!token || !/^[0-9a-f-]{36}$/i.test(token)) {
    return NextResponse.json({ error: "invalid token" }, { status: 400 });
  }

  // 1. Resolve caller.
  const supabase = await createClient();
  const {
    data: { user },
  } = await supabase.auth.getUser();
  if (!user || !user.email) {
    return NextResponse.json({ error: "not signed in" }, { status: 401 });
  }

  const svc = getServiceClient();

  // 2. Look up invite (service role bypasses the owner-only RLS read policy).
  const { data: invite, error: inviteErr } = await svc
    .from("org_invites")
    .select("id, org_id, email, role, expires_at, accepted_at, revoked_at")
    .eq("token", token)
    .maybeSingle<InviteRow>();

  if (inviteErr) {
    return NextResponse.json({ error: "lookup failed" }, { status: 500 });
  }
  if (!invite) {
    return NextResponse.json({ error: "invite not found" }, { status: 404 });
  }

  // 3. Validate live state.
  if (invite.revoked_at) {
    return NextResponse.json({ error: "invite revoked" }, { status: 410 });
  }
  if (invite.accepted_at) {
    return NextResponse.json({ error: "invite already accepted" }, { status: 410 });
  }
  if (new Date(invite.expires_at) < new Date()) {
    return NextResponse.json({ error: "invite expired" }, { status: 410 });
  }

  // 4. Email-match guard. Case-insensitive compare; emails are stored lowercased
  //    on insert from the dashboard, but anything writing directly to the table
  //    might not, so normalise on read too.
  if (invite.email.toLowerCase() !== user.email.toLowerCase()) {
    return NextResponse.json(
      {
        error: "invite email does not match signed-in user",
        invite_email: invite.email,
      },
      { status: 409 }
    );
  }

  // 5. Insert membership. Idempotent: if the user is already in the org
  //    (could happen if they double-click the link), surface success rather
  //    than a unique-constraint failure.
  const { error: memberErr } = await svc.from("org_memberships").upsert(
    {
      org_id: invite.org_id,
      user_id: user.id,
      role: invite.role,
      invited_by: null, // we don't carry it through; invite row has the answer
    },
    { onConflict: "org_id,user_id" }
  );
  if (memberErr) {
    return NextResponse.json(
      { error: "failed to add membership", detail: memberErr.message },
      { status: 500 }
    );
  }

  // 6. Stamp accepted_at. Use a conditional WHERE so we can't double-stamp
  //    if two requests race — the second update will affect 0 rows but the
  //    first response already won.
  await svc
    .from("org_invites")
    .update({ accepted_at: new Date().toISOString() })
    .eq("id", invite.id)
    .is("accepted_at", null);

  return NextResponse.json({ org_id: invite.org_id, role: invite.role });
}
