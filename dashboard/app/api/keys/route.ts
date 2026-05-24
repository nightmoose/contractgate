/**
 * POST /api/keys  — Issue a new API key (server-side, service role).
 * DELETE /api/keys — Revoke an existing key by id.
 *
 * RFC-056: moves API key issuance server-side so the browser never
 * generates or hashes the raw key.
 *
 * Key format (must match api_key_auth.rs exactly):
 *   raw  = "cg_live_" + 48 lowercase hex chars  (56 chars total)
 *   prefix = raw[0..12]
 *   hash   = base64( sha256(raw_as_utf8_bytes) )  — standard base64, no padding difference
 *
 * Both endpoints:
 *   • Require a valid Supabase session (cookie auth).
 *   • Resolve org_id from org_memberships (never trust client-supplied org).
 *   • Perform a same-origin CSRF check (Origin or Referer must match Host).
 */

import crypto from "crypto";
import { createClient } from "@/lib/supabase/server";
import { createClient as createServiceClient } from "@supabase/supabase-js";
import { NextResponse } from "next/server";

// ── Helpers ───────────────────────────────────────────────────────────────────

function getServiceClient() {
  return createServiceClient(
    process.env.NEXT_PUBLIC_SUPABASE_URL!,
    process.env.SUPABASE_SERVICE_ROLE_KEY!
  );
}

/** Resolve the session user's primary org via org_memberships. */
async function resolveOrgId(userId: string): Promise<string | null> {
  const svc = getServiceClient();
  const { data } = await svc
    .from("org_memberships")
    .select("org_id")
    .eq("user_id", userId)
    .is("deleted_at", null)
    .order("joined_at", { ascending: true })
    .limit(1)
    .single();
  return data?.org_id ?? null;
}

/**
 * Same-origin CSRF check: Origin (or Referer) host must match the request
 * Host header.  Browser same-origin requests always send one of these; a
 * cross-origin request from an attacker-controlled page will either send a
 * different origin or (on older browsers) no origin at all.
 *
 * Rejects if neither header is present — a missing origin from a
 * same-origin browser fetch should not happen in practice.
 */
function isSameOrigin(request: Request): boolean {
  const host = request.headers.get("host");
  if (!host) return false;

  const origin = request.headers.get("origin");
  const referer = request.headers.get("referer");
  const source = origin ?? referer;
  if (!source) return false;

  try {
    return new URL(source).host === host;
  } catch {
    return false;
  }
}

// ── POST /api/keys ────────────────────────────────────────────────────────────

export async function POST(request: Request) {
  // 1. CSRF guard — must come before session resolution to fail fast.
  if (!isSameOrigin(request)) {
    return NextResponse.json({ error: "forbidden: cross-origin request" }, { status: 403 });
  }

  // 2. Resolve session.
  const supabase = await createClient();
  const {
    data: { user },
  } = await supabase.auth.getUser();
  if (!user) {
    return NextResponse.json({ error: "not signed in" }, { status: 401 });
  }

  // 3. Parse body.
  let body: { name?: unknown };
  try {
    body = await request.json();
  } catch {
    return NextResponse.json({ error: "invalid JSON body" }, { status: 400 });
  }

  const name = typeof body.name === "string" ? body.name.trim() : "";
  if (!name) {
    return NextResponse.json({ error: "name is required" }, { status: 400 });
  }
  if (name.length > 80) {
    return NextResponse.json({ error: "name must be ≤ 80 characters" }, { status: 400 });
  }

  // 4. Resolve org — never trust a client-supplied org_id.
  const orgId = await resolveOrgId(user.id);
  if (!orgId) {
    return NextResponse.json({ error: "no org found for user" }, { status: 404 });
  }

  // 5. Generate raw key server-side with CSPRNG.
  //    Format: cg_live_ + 48 lowercase hex chars = 56 chars total.
  //    24 random bytes → 48 hex chars (192 bits of entropy).
  const rawKey = `cg_live_${crypto.randomBytes(24).toString("hex")}`;

  // 6. Derive prefix and hash — must match api_key_auth.rs exactly.
  //    key_prefix = first 12 chars of rawKey ("cg_live_XXXX")
  //    key_hash   = base64( sha256(rawKey as UTF-8 bytes) )
  const keyPrefix = rawKey.slice(0, 12);
  const keyHash = crypto.createHash("sha256").update(rawKey, "utf8").digest("base64");

  // 7. Insert via service role (bypasses RLS insert block after migration 027).
  //    Raw key is NEVER logged — only hash and prefix reach the DB.
  const svc = getServiceClient();
  const { data: inserted, error: insertErr } = await svc
    .from("api_keys")
    .insert({
      user_id: user.id,
      org_id: orgId,
      name,
      key_prefix: keyPrefix,
      key_hash: keyHash,
    })
    .select("id, name, key_prefix, created_at")
    .single();

  if (insertErr || !inserted) {
    console.error("[POST /api/keys] insert error:", insertErr?.message);
    return NextResponse.json(
      { error: "failed to create key", detail: insertErr?.message },
      { status: 500 }
    );
  }

  // 8. Return the raw key exactly once.  It is never persisted — the caller
  //    must display it to the user immediately and cannot retrieve it again.
  return NextResponse.json({
    id: inserted.id,
    name: inserted.name,
    key_prefix: inserted.key_prefix,
    created_at: inserted.created_at,
    raw_key: rawKey,
  });
}

// ── DELETE /api/keys ──────────────────────────────────────────────────────────

export async function DELETE(request: Request) {
  // 1. CSRF guard.
  if (!isSameOrigin(request)) {
    return NextResponse.json({ error: "forbidden: cross-origin request" }, { status: 403 });
  }

  // 2. Resolve session.
  const supabase = await createClient();
  const {
    data: { user },
  } = await supabase.auth.getUser();
  if (!user) {
    return NextResponse.json({ error: "not signed in" }, { status: 401 });
  }

  // 3. Parse body.
  let body: { id?: unknown };
  try {
    body = await request.json();
  } catch {
    return NextResponse.json({ error: "invalid JSON body" }, { status: 400 });
  }

  const keyId = typeof body.id === "string" ? body.id : null;
  if (!keyId || !/^[0-9a-f-]{36}$/i.test(keyId)) {
    return NextResponse.json({ error: "id is required (UUID)" }, { status: 400 });
  }

  // 4. Resolve org — scopes the revocation to the session user's org.
  const orgId = await resolveOrgId(user.id);
  if (!orgId) {
    return NextResponse.json({ error: "no org found for user" }, { status: 404 });
  }

  // 5. Verify the key belongs to the session user's org before revoking.
  //    Returns 404 (not found or wrong org) so callers cannot enumerate other orgs' key IDs.
  const svc = getServiceClient();
  const { data: existing } = await svc
    .from("api_keys")
    .select("id, revoked_at")
    .eq("id", keyId)
    .eq("org_id", orgId)
    .is("deleted_at", null)
    .maybeSingle();

  if (!existing) {
    return NextResponse.json({ error: "key not found" }, { status: 404 });
  }
  if (existing.revoked_at) {
    return NextResponse.json({ error: "key already revoked" }, { status: 409 });
  }

  // 6. Stamp revoked_at.  Revocation propagates within the 60 s cache TTL
  //    in api_key_auth.rs — callers should expect up to 60 s of latency.
  const { error: revokeErr } = await svc
    .from("api_keys")
    .update({ revoked_at: new Date().toISOString() })
    .eq("id", keyId)
    .eq("org_id", orgId)
    .is("revoked_at", null); // conditional guard against races

  if (revokeErr) {
    console.error("[DELETE /api/keys] revoke error:", revokeErr.message);
    return NextResponse.json(
      { error: "failed to revoke key", detail: revokeErr.message },
      { status: 500 }
    );
  }

  return new NextResponse(null, { status: 204 });
}
