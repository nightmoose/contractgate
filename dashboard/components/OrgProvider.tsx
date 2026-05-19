"use client";

/**
 * OrgProvider — resolves the current user's org and session JWT, registering
 * both with the API client so every Rust API call carries the correct auth.
 *
 * RFC-039: calls setApiSession() so apiFetch sends
 * `Authorization: Bearer <token>` instead of the static x-api-key.
 * Token refreshes propagate via onAuthStateChange so long-lived sessions
 * don't get stale tokens after Supabase auto-refreshes.
 *
 * Rendered once in RootLayout.  Has no visible output — purely side-effects.
 */

import { useEffect, useRef } from "react";
import { useOrg } from "@/lib/org";
import { setApiOrgId, setApiSession } from "@/lib/api";
import { createClient } from "@/lib/supabase/client";

export default function OrgProvider() {
  const { org } = useOrg();
  const supabaseRef = useRef(createClient());

  // Register org_id with the API client.
  useEffect(() => {
    if (org?.org_id) {
      setApiOrgId(org.org_id);
    }
  }, [org?.org_id]);

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
