import "server-only";
import { redirect } from "next/navigation";
import { supabaseSession } from "./supabaseServer";

/** Parsed, lower-cased superadmin allowlist from ADMIN_EMAILS. */
export function adminEmails(): string[] {
  return (process.env.ADMIN_EMAILS ?? "")
    .split(",")
    .map((e) => e.trim().toLowerCase())
    .filter(Boolean);
}

export function isAllowlisted(email: string | null | undefined): boolean {
  if (!email) return false;
  return adminEmails().includes(email.toLowerCase());
}

/**
 * Gate for every admin surface: require a signed-in Supabase user whose email
 * is on the allowlist. Redirects to /login otherwise. Returns the operator's
 * email. Server components / route handlers call this before any privileged
 * read — defense in depth on top of middleware.ts.
 */
export async function requireAdmin(): Promise<string> {
  const supabase = await supabaseSession();
  const {
    data: { user },
  } = await supabase.auth.getUser();

  if (!user) redirect("/login");
  if (!isAllowlisted(user.email)) redirect("/login?denied=1");
  return user.email!;
}
