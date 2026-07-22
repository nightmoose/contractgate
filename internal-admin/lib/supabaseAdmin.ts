import "server-only";
import { createClient } from "@supabase/supabase-js";

// God-mode service-role client — bypasses RLS. SERVER ONLY. Never import this
// into a client component. The key is read from a non-public env var so it can
// never be bundled into browser code.
export function supabaseAdmin() {
  const url = process.env.SUPABASE_URL;
  const key = process.env.SUPABASE_SERVICE_ROLE_KEY;
  if (!url || !key) {
    throw new Error("SUPABASE_URL and SUPABASE_SERVICE_ROLE_KEY must be set");
  }
  return createClient(url, key, {
    auth: { persistSession: false, autoRefreshToken: false },
  });
}
