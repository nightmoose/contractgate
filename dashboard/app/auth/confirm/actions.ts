"use server";

import { cookies } from "next/headers";
import { redirect } from "next/navigation";
import { BRAND_COOKIE } from "@/lib/brand";
import { createClient } from "@/lib/supabase/server";
import {
  defaultNext,
  failurePath,
  isEmailOtpType,
  safeNextPath,
} from "@/lib/auth-email-link";

// Verifies a token_hash from a Supabase email template. Called from the
// confirm page's form, not from the page load: mail scanners GET the link
// and would otherwise burn the one-time token before the person clicks.
export async function confirmEmailLink(formData: FormData) {
  const tokenHash = String(formData.get("token_hash") ?? "");
  const type = String(formData.get("type") ?? "");
  const next = String(formData.get("next") ?? "");
  const product = String(formData.get("product") ?? "");

  if (!tokenHash || !isEmailOtpType(type)) {
    redirect("/auth/login?error=confirmation_failed");
  }

  const supabase = await createClient();
  const { data, error } = await supabase.auth.verifyOtp({
    type,
    token_hash: tokenHash,
  });
  if (error) {
    redirect(failurePath(type));
  }

  // Signup from aischemagate.com stores product on the user before the email
  // is confirmed. The cookie is what the first dashboard render brands on,
  // before useOrg() resolves. Same options as proxy.ts.
  const metaProduct = data.user?.user_metadata?.product;
  if (product === "aischemagate" || metaProduct === "aischemagate") {
    const cookieStore = await cookies();
    cookieStore.set(BRAND_COOKIE, "aisg", {
      path: "/",
      maxAge: 60 * 60 * 24 * 30,
      sameSite: "lax",
      secure: true,
    });
  }

  redirect(safeNextPath(next, defaultNext(type)));
}
