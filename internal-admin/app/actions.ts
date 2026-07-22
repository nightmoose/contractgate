"use server";

import { redirect } from "next/navigation";
import { supabaseSession } from "@/lib/supabaseServer";
import { isAllowlisted } from "@/lib/auth";

export async function loginAction(formData: FormData) {
  const email = String(formData.get("email") ?? "").trim();
  const password = String(formData.get("password") ?? "");

  const supabase = await supabaseSession();
  const { error } = await supabase.auth.signInWithPassword({ email, password });
  if (error) redirect(`/login?error=${encodeURIComponent(error.message)}`);

  // Allowlist is the real gate — a valid Supabase login is not enough.
  if (!isAllowlisted(email)) {
    await supabase.auth.signOut();
    redirect("/login?denied=1");
  }
  redirect("/users");
}

export async function logoutAction() {
  const supabase = await supabaseSession();
  await supabase.auth.signOut();
  redirect("/login");
}
