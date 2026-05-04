import { createBrowserClient } from "@supabase/ssr";
import type { SupabaseClient } from "@supabase/supabase-js";

/**
 * Browser-side Supabase client.
 * Use this in Client Components ("use client").
 *
 * Returns a single module-level instance per page load — Supabase advises
 * against constructing multiple browser clients (each carries its own
 * realtime/auth listeners), and call sites previously instantiated a fresh
 * one per consumer. The function shape is preserved so existing imports keep
 * working unchanged.
 */
let _client: SupabaseClient | null = null;

/**
 * Returns a real Supabase client when URL + key are present, or a no-op stub
 * in demo mode (NEXT_PUBLIC_DEMO_MODE=1).  The stub satisfies the TypeScript
 * interface but every method is a safe no-op — in demo mode the three
 * chokepoints (middleware, AuthGate, useOrg) short-circuit before any real
 * Supabase call is needed.  Prevents SSR prerender crashes when no Supabase
 * env vars are configured.
 */
export function createClient(): SupabaseClient {
  if (_client) return _client;

  const url = process.env.NEXT_PUBLIC_SUPABASE_URL ?? "";
  const key = process.env.NEXT_PUBLIC_SUPABASE_ANON_KEY ?? "";

  if (!url || !key) {
    // Demo / missing-env mode: return a minimal stub that never throws.
    // All real auth paths are short-circuited before reaching this client.
    const noSub = { unsubscribe: () => {} };
    const chain = (): Record<string, unknown> =>
      new Proxy({} as Record<string, unknown>, {
        get(_, prop) {
          if (prop === "then") return undefined; // not a Promise
          const s = String(prop);
          const chainable = ["select","insert","update","delete","upsert","eq","neq","is","gt","lt","gte","lte","order","limit","single","maybeSingle","not","in"];
          if (chainable.includes(s)) return () => chain();
          return async () => ({ data: null, error: null });
        },
      });
    _client = {
      auth: {
        getUser: async () => ({ data: { user: null }, error: null }),
        signOut: async () => ({ error: null }),
        signInWithPassword: async () => ({ data: { user: null, session: null }, error: null }),
        signUp: async () => ({ data: { user: null, session: null }, error: null }),
        onAuthStateChange: () => ({ data: { subscription: noSub } }),
        verifyOtp: async () => ({ data: { user: null, session: null }, error: null }),
      },
      from: () => chain(),
    } as unknown as SupabaseClient;
    return _client;
  }

  _client = createBrowserClient(url, key);
  return _client;
}
