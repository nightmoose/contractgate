/**
 * POST /api/auth/verify-turnstile
 *
 * Verifies a Cloudflare Turnstile token server-side before the client is
 * allowed to call supabase.auth.signUp(). Turnstile tokens are single-use and
 * must be checked against Cloudflare's siteverify endpoint with the secret
 * key — the secret must never reach the browser.
 */

import { NextRequest, NextResponse } from "next/server";

const SITEVERIFY_URL = "https://challenges.cloudflare.com/turnstile/v0/siteverify";

export async function POST(req: NextRequest) {
  const secret = process.env.TURNSTILE_SECRET_KEY;
  if (!secret) {
    // No secret configured (e.g. local dev without keys) — fail closed in
    // production, but allow through in dev so the signup flow isn't blocked.
    if (process.env.NODE_ENV === "production") {
      return NextResponse.json({ success: false, error: "captcha_not_configured" }, { status: 500 });
    }
    return NextResponse.json({ success: true });
  }

  let token: string | undefined;
  try {
    const body = await req.json();
    token = body?.token;
  } catch {
    return NextResponse.json({ success: false, error: "invalid_request" }, { status: 400 });
  }

  if (!token) {
    return NextResponse.json({ success: false, error: "missing_token" }, { status: 400 });
  }

  const remoteip = req.headers.get("x-forwarded-for")?.split(",")[0]?.trim();

  const verifyRes = await fetch(SITEVERIFY_URL, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ secret, response: token, ...(remoteip ? { remoteip } : {}) }),
  });

  const result = await verifyRes.json();

  if (!result.success) {
    return NextResponse.json({ success: false, error: "captcha_failed" }, { status: 400 });
  }

  return NextResponse.json({ success: true });
}
