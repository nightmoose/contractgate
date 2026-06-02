/**
 * /api/github/config — Read and write the GitHub integration config for the
 * current user's org.
 *
 * GET  — Returns the config (repo, path_prefix, branch, has_token).
 *         NOTE: The github_token is intentionally omitted from the response —
 *         it is stored server-side only and never sent to the browser.
 *
 * PUT  — Upserts the config. Accepts { repo, path_prefix, branch, github_token }.
 *         Pass github_token: "" to clear the token without removing the config.
 *
 * Both endpoints require an authenticated Supabase session (cookie-based).
 * The org_id is resolved from the user's first org membership, matching the
 * pattern used by the rest of the dashboard (see lib/org.ts).
 */

import { createClient } from "@/lib/supabase/server";
import { createClient as createServiceClient } from "@supabase/supabase-js";
import { NextResponse } from "next/server";

// Service-role client — can read the github_token column that RLS hides from
// the anon/user role. Only used server-side, never exposed to the browser.
function getServiceClient() {
  const url = process.env.NEXT_PUBLIC_SUPABASE_URL!;
  const key = process.env.SUPABASE_SERVICE_ROLE_KEY!;
  return createServiceClient(url, key);
}

/** Resolve the caller's primary org_id from their Supabase session. */
async function resolveOrgId(userId: string): Promise<string | null> {
  const svc = getServiceClient();
  const { data } = await svc
    .from("org_memberships")
    .select("org_id")
    .eq("user_id", userId)
    .order("joined_at", { ascending: true })
    .limit(1)
    .single();
  return data?.org_id ?? null;
}

// ── GET /api/github/config ────────────────────────────────────────────────────

export async function GET() {
  const supabase = await createClient();
  const { data: { user }, error: authErr } = await supabase.auth.getUser();
  if (authErr || !user) {
    return NextResponse.json({ error: "Unauthorized" }, { status: 401 });
  }

  const orgId = await resolveOrgId(user.id);
  if (!orgId) {
    return NextResponse.json({ error: "No org found for user" }, { status: 404 });
  }

  const svc = getServiceClient();
  const { data, error } = await svc
    .from("github_integrations")
    .select("id, repo, path_prefix, branch, github_token, created_at, updated_at")
    .eq("org_id", orgId)
    .maybeSingle();

  if (error) {
    console.error("[github/config GET]", error);
    return NextResponse.json({ error: error.message }, { status: 500 });
  }

  if (!data) {
    return NextResponse.json(null, { status: 200 });
  }

  // Strip the token — return only a boolean indicating whether one is set.
  return NextResponse.json({
    id: data.id,
    repo: data.repo,
    path_prefix: data.path_prefix,
    branch: data.branch,
    has_token: Boolean(data.github_token),
    created_at: data.created_at,
    updated_at: data.updated_at,
  });
}

// ── PUT /api/github/config ────────────────────────────────────────────────────

export async function PUT(request: Request) {
  const supabase = await createClient();
  const { data: { user }, error: authErr } = await supabase.auth.getUser();
  if (authErr || !user) {
    return NextResponse.json({ error: "Unauthorized" }, { status: 401 });
  }

  const orgId = await resolveOrgId(user.id);
  if (!orgId) {
    return NextResponse.json({ error: "No org found for user" }, { status: 404 });
  }

  let body: { repo?: string; path_prefix?: string; branch?: string; github_token?: string };
  try {
    body = await request.json();
  } catch {
    return NextResponse.json({ error: "Invalid JSON body" }, { status: 400 });
  }

  const { repo, path_prefix, branch, github_token } = body;

  if (!repo || typeof repo !== "string" || !repo.trim()) {
    return NextResponse.json({ error: "repo is required (owner/repo format)" }, { status: 400 });
  }
  if (!/^[a-zA-Z0-9_.-]+\/[a-zA-Z0-9_.-]+$/.test(repo.trim())) {
    return NextResponse.json(
      { error: "repo must be in owner/repo format (e.g. acme/data-contracts)" },
      { status: 400 }
    );
  }

  // Ensure path_prefix ends with "/" if non-empty.
  let normalizedPrefix = (path_prefix ?? "contracts/").trim();
  if (normalizedPrefix && !normalizedPrefix.endsWith("/")) {
    normalizedPrefix += "/";
  }

  const svc = getServiceClient();

  // Check whether a row already exists for this org.
  const { data: existing } = await svc
    .from("github_integrations")
    .select("id, github_token")
    .eq("org_id", orgId)
    .maybeSingle();

  // If caller didn't send a token (or sent empty string) and we're updating,
  // preserve the existing token rather than wiping it.
  const resolvedToken =
    typeof github_token === "string" && github_token.trim()
      ? github_token.trim()
      : existing?.github_token ?? null;

  const payload = {
    org_id: orgId,
    repo: repo.trim(),
    path_prefix: normalizedPrefix,
    branch: (branch ?? "main").trim() || "main",
    github_token: resolvedToken,
  };

  let result;
  if (existing) {
    result = await svc
      .from("github_integrations")
      .update(payload)
      .eq("org_id", orgId)
      .select("id, repo, path_prefix, branch, created_at, updated_at")
      .single();
  } else {
    result = await svc
      .from("github_integrations")
      .insert(payload)
      .select("id, repo, path_prefix, branch, created_at, updated_at")
      .single();
  }

  if (result.error) {
    console.error("[github/config PUT]", result.error);
    return NextResponse.json({ error: result.error.message }, { status: 500 });
  }

  return NextResponse.json({
    ...result.data,
    has_token: Boolean(resolvedToken),
  });
}

// ── DELETE /api/github/config ─────────────────────────────────────────────────

export async function DELETE() {
  const supabase = await createClient();
  const { data: { user }, error: authErr } = await supabase.auth.getUser();
  if (authErr || !user) {
    return NextResponse.json({ error: "Unauthorized" }, { status: 401 });
  }

  const orgId = await resolveOrgId(user.id);
  if (!orgId) {
    return NextResponse.json({ error: "No org found for user" }, { status: 404 });
  }

  const svc = getServiceClient();
  const { error } = await svc
    .from("github_integrations")
    .delete()
    .eq("org_id", orgId);

  if (error) {
    console.error("[github/config DELETE]", error);
    return NextResponse.json({ error: error.message }, { status: 500 });
  }

  return new NextResponse(null, { status: 204 });
}
