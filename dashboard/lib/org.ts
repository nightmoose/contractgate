"use client";

/**
 * useOrg — resolves the current Supabase user's primary org.
 *
 * ContractGate uses org-scoped tenancy (RFC-001): every user is
 * auto-provisioned an org on first sign-up, and all resources (contracts,
 * api_keys, audit_log) carry an org_id FK.
 *
 * This hook fetches the user's first org membership (by created_at) so
 * client components can include org_id in INSERT statements and display
 * org context in the UI.
 *
 * RLS policies on the Supabase tables handle SELECT scoping automatically —
 * callers only need org_id when writing rows.
 */

import { useState, useEffect, useRef } from "react";
import { createClient } from "@/lib/supabase/client";
import { DEMO_MODE, DEMO_ORG_UUID, DEMO_ORG_NAME } from "@/lib/demo";

export interface OrgInfo {
  org_id: string;
  org_name: string;
  slug: string;
  role: "owner" | "admin" | "member";
}

/**
 * Shape of the joined `orgs(name, slug)` cell.  Supabase's generated
 * types render foreign-key joins as either `T | null` (one-to-one) or
 * `T[]` (one-to-many) depending on schema introspection — and since we
 * read it as `data.orgs` with `.single()` either flavour can land at
 * runtime.  Local type so the unwrap below stays narrow and removes the
 * `as unknown` double-cast that was here previously.
 */
type OrgJoinCell = { name: string; slug: string } | { name: string; slug: string }[] | null;

interface UseOrgResult {
  org: OrgInfo | null;
  /** True while the membership query is in-flight. */
  loading: boolean;
  /** Non-null if the lookup failed (network error, RLS rejection, etc.). */
  error: string | null;
}

/**
 * Returns the current user's primary org.
 *
 * "Primary" = first membership row by created_at; for single-org users
 * (the typical case right now) this is always their personal org.
 * Multi-org support (org switcher) is a future concern — this hook is the
 * single place to upgrade when that lands.
 */
export function useOrg(): UseOrgResult {
  // Demo mode: return fixed org synchronously — no Supabase session needed.
  if (DEMO_MODE) {
    return {
      org: {
        org_id: DEMO_ORG_UUID,
        org_name: DEMO_ORG_NAME,
        slug: "demo",
        role: "owner",
      },
      loading: false,
      error: null,
    };
  }

  const [org, setOrg] = useState<OrgInfo | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  // Stable client ref — createBrowserClient is idempotent but we avoid
  // re-creating it on every render to keep the effect deps clean.
  const supabaseRef = useRef(createClient());

  useEffect(() => {
    let cancelled = false;
    const supabase = supabaseRef.current;

    (async () => {
      try {
        const {
          data: { user },
        } = await supabase.auth.getUser();

        if (!user) {
          if (!cancelled) setLoading(false);
          return;
        }

        const { data, error: dbErr } = await supabase
          .from("org_memberships")
          .select("org_id, role, orgs(name, slug)")
          .eq("user_id", user.id)
          .order("joined_at", { ascending: true })
          .limit(1)
          .single();

        if (cancelled) return;

        if (dbErr || !data) {
          setError(dbErr?.message ?? "No org membership found");
          setLoading(false);
          return;
        }

        // Normalize the `orgs` join cell — see OrgJoinCell above for why
        // it can be either an object or an array.
        const rawOrgs = data.orgs as OrgJoinCell;
        const orgsRow = Array.isArray(rawOrgs) ? (rawOrgs[0] ?? null) : rawOrgs;
        setOrg({
          org_id: data.org_id as string,
          org_name: orgsRow?.name ?? "My Org",
          slug: orgsRow?.slug ?? "",
          role: (data.role as OrgInfo["role"]) ?? "member",
        });
      } catch (e: unknown) {
        if (!cancelled) {
          setError(e instanceof Error ? e.message : "Failed to load org");
        }
      } finally {
        if (!cancelled) setLoading(false);
      }
    })();

    return () => {
      cancelled = true;
    };
  }, []);

  return { org, loading, error };
}
