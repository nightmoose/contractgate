import "server-only";
import { createServerClient } from "@supabase/ssr";
import { cookies } from "next/headers";

// SSR Supabase client for reading the *logged-in operator's* session (anon key,
// not privileged). Used only to identify who is signed in so the allowlist can
// be checked — all privileged data reads go through supabaseAdmin().
export async function supabaseSession() {
  const cookieStore = await cookies();
  return createServerClient(
    process.env.NEXT_PUBLIC_SUPABASE_URL!,
    process.env.NEXT_PUBLIC_SUPABASE_ANON_KEY!,
    {
      cookies: {
        getAll: () => cookieStore.getAll(),
        setAll: (toSet) => {
          try {
            toSet.forEach(({ name, value, options }) =>
              cookieStore.set(name, value, options)
            );
          } catch {
            // Called from a Server Component render — safe to ignore; the
            // middleware refreshes the session cookie.
          }
        },
      },
    }
  );
}
