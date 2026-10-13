import { createClient } from "@/lib/supabase/server";
import { NextResponse } from "next/server";
import { BRAND_COOKIE } from "@/lib/brand";

/**
 * Handles the email confirmation redirect from Supabase Auth.
 * Supabase redirects here with a `code` query param after email verification.
 */
export async function GET(request: Request) {
  const { searchParams, origin } = new URL(request.url);
  const code = searchParams.get("code");
  const next = searchParams.get("next") ?? "/";
  // Signup pages that came from aischemagate.com embed product=aischemagate in
  // the Supabase email `redirectTo` so cross-device confirmation lands AISG-
  // branded even before useOrg() resolves. Same cookie options as proxy.ts.
  const isAisg = searchParams.get("product") === "aischemagate";

  if (code) {
    const supabase = await createClient();
    const { error } = await supabase.auth.exchangeCodeForSession(code);
    if (!error) {
      const response = NextResponse.redirect(`${origin}${next}`);
      if (isAisg) {
        response.cookies.set(BRAND_COOKIE, "aisg", {
          path: "/",
          maxAge: 60 * 60 * 24 * 30,
          sameSite: "lax",
          secure: true,
        });
      }
      return response;
    }
  }

  // Redirect to error page or login on failure
  return NextResponse.redirect(`${origin}/auth/login?error=confirmation_failed`);
}
