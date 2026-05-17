"use client";

/**
 * OrgProvider — resolves the current user's org and session JWT, registering
 * both with the API client so every Rust API call carries the correct auth.
 *
 * RFC-039: also calls setApiSession() so apiFetch sends
 * `Authorization: Bearer <token>` instead of the static x-api-key.
 * Token refreshes propagate via onAuthStateChange so long-lived sessions
 * don't get stale tokens after Supabase auto-refreshes.
 *
 * Rendered once in RootLayout (below the Sidebar).  Has no visible output —
 * it's purely a side-effect component.
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

  // RFC-039: register the initial session JWT and keep it current.
  useEffect(() => {
    const supabase = supabaseRef.current;

    // Seed from the current session immediately (no round-trip wait).
    supabase.auth.getSession().then(({ data: { session } }) => {
      setApiSession(session?.access_token ?? null);
    });

    // Keep the token fresh across auto-refreshes and sign-out.
    const { data: { subscription } } = supabase.auth.onAuthStateChange(
      (_event, session) => {
        setApiSession(session?.access_token ?? null);
      }
    );

    return () => subscription.unsubscribe();
  }, []);

  return null;
}
