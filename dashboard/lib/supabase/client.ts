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

export function createClient(): SupabaseClient {
  if (_client) return _client;
  _client = createBrowserClient(
    process.env.NEXT_PUBLIC_SUPABASE_URL!,
    process.env.NEXT_PUBLIC_SUPABASE_ANON_KEY!
  );
  return _client;
}
