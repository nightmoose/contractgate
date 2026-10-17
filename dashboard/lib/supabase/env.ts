/** Browser-reachable Supabase URL. Inlined at build from NEXT_PUBLIC_SUPABASE_URL. */
export function supabasePublicUrl(): string {
  return process.env.NEXT_PUBLIC_SUPABASE_URL ?? "";
}

/**
 * URL the Next.js server uses to reach Auth and PostgREST.
 * Self-host compose sets SUPABASE_SERVER_URL to the in-network API when
 * NEXT_PUBLIC_SUPABASE_URL is the host loopback the browser uses.
 * Unset on hosted deploys, which keep using NEXT_PUBLIC_SUPABASE_URL.
 */
export function supabaseServerUrl(): string {
  const internal = process.env.SUPABASE_SERVER_URL?.trim();
  if (internal) return internal;
  return process.env.NEXT_PUBLIC_SUPABASE_URL ?? "";
}
