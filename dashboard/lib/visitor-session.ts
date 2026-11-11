import { headers } from "next/headers";
import { DEMO_MODE } from "@/lib/demo";
import { decodeVisitorPath, VISITOR_AUTH_HEADER, VISITOR_PATH_HEADER } from "@/lib/logged-out-cta";
import { createClient } from "@/lib/supabase/server";
import { supabaseServerUrl } from "@/lib/supabase/env";

export interface VisitorSession {
  /** True when this request has a signed-in user, or when demo mode is on. */
  authed: boolean;
  /** Path plus query the visitor requested. `/` when the proxy did not stamp one. */
  pathname: string;
}

// One session read for the header. proxy.ts already called getUser and
// stamped the result; this trusts that stamp. When the stamp is absent
// (a build-time render, or a matcher miss) it falls back to one getUser,
// and if Auth is not configured it treats the visitor as logged out so
// Sign up stays visible.
export async function resolveVisitor(): Promise<VisitorSession> {
  if (DEMO_MODE) return { authed: true, pathname: "/" };

  const h = await headers();
  const pathname = decodeVisitorPath(h.get(VISITOR_PATH_HEADER));
  const stamp = h.get(VISITOR_AUTH_HEADER);
  if (stamp === "1") return { authed: true, pathname };
  if (stamp === "0") return { authed: false, pathname };

  const url = supabaseServerUrl();
  const key = process.env.NEXT_PUBLIC_SUPABASE_ANON_KEY;
  if (!url || !key) return { authed: false, pathname };

  try {
    const supabase = await createClient();
    const { data: { user } } = await supabase.auth.getUser();
    return { authed: !!user, pathname };
  } catch {
    return { authed: false, pathname };
  }
}
