"use client";

/**
 * OrgProvider — resolves the current user's session JWT and registers it with
 * the API client so every Rust API call carries the correct Bearer token.
 *
 * RFC-039: calls setApiSession() so apiFetch sends
 * `Authorization: Bearer <token>` instead of the static x-api-key.
 * Token refreshes propagate via onAuthStateChange so long-lived sessions
 * don't get stale tokens after Supabase auto-refreshes.
 *
 * RFC-048: x-org-id header removed — org context is carried by the JWT itself.
 * setApiOrgId / _apiOrgId have been deleted from the API client.
 *
 * Rendered once in RootLayout.  Has no visible output — purely side-effects.
 */

import { useEffect, useRef } from "react";
import { setApiSession } from "@/lib/api";
import { createClient } from "@/lib/supabase/client";

export default function OrgProvider() {
  const supabaseRef = useRef(createClient());

  // RFC-039: seed Bearer token from current session; keep it fresh.
  useEffect(() => {
    const supabase = supabaseRef.current;

    supabase.auth.getSession().then(({ data: { session } }) => {
      setApiSession(session?.access_token ?? null);
    });

    const {
      data: { subscription },
    } = supabase.auth.onAuthStateChange((_event, session) => {
      setApiSession(session?.access_token ?? null);
    });

    return () => subscription.unsubscribe();
  }, []);

  return null;
}
