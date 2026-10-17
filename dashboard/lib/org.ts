"use client";

/**
 * useOrg — resolves the current Supabase user's primary org.
 *
 * DataContractGate uses org-scoped tenancy (RFC-001): every user is
 * auto-provisioned an org on first sign-up, and all resources (contracts,
 * api_keys, audit_log) carry an org_id FK.
 *
 * Resolves the signed-in user's earliest org membership (`joined_at`
 * ascending). Callers use it for plan checks, labels, and org-scoped reads.
 */

import { useState, useEffect, useRef } from "react";
import { createClient } from "@/lib/supabase/client";
import { DEMO_MODE, DEMO_ORG_UUID, DEMO_ORG_NAME } from "@/lib/demo";

/**
 * RFC-045: billing plan tier exposed to UI for feature gating.
 * `aisg_*` are AISchemaGate's plans (migration 036): same tenant and backend,
 * its own tiers. For feature gating they rank alongside free and growth.
 */
export type PlanTier = "free" | "growth" | "enterprise" | "aisg_free" | "aisg_pro";

/** Ordered list used for tier comparisons — index = capability rank. */
export const PLAN_ORDER: PlanTier[] = ["free", "growth", "enterprise"];

const PLAN_RANK: Record<PlanTier, number> = {
  free: 0,
  aisg_free: 0,
  growth: 1,
  aisg_pro: 1,
  enterprise: 2,
};

const KNOWN_PLANS = Object.keys(PLAN_RANK) as PlanTier[];

/** Returns true when `actual` meets or exceeds `required`. */
export function planAtLeast(actual: PlanTier, required: PlanTier): boolean {
  return PLAN_RANK[actual] >= PLAN_RANK[required];
}

/** True for AISchemaGate plans. */
export function isAisgPlan(plan: PlanTier): boolean {
  return plan === "aisg_free" || plan === "aisg_pro";
}

/** True for the two no-cost plans. */
export function isFreePlan(plan: PlanTier): boolean {
  return plan === "free" || plan === "aisg_free";
}

/**
 * Plans for which `isPaidPlan` is true. `paidPlansPhrase` is built from this
 * same list (DataContractGate Pro, AISchemaGate Pro, Enterprise).
 */
const PAID_PLANS: readonly PlanTier[] = ["growth", "aisg_pro", "enterprise"];

/** True for plans billed through Stripe or managed (enterprise). */
export function isPaidPlan(plan: PlanTier): boolean {
  return PAID_PLANS.includes(plan);
}

/** Name shown to the customer. */
export function planDisplayName(plan: PlanTier): string {
  switch (plan) {
    case "free": return "Free";
    case "growth": return "Pro";
    case "enterprise": return "Enterprise";
    case "aisg_free": return "Free";
    case "aisg_pro": return "AISchemaGate Pro";
  }
}

/** Name of the self-serve paid plan a free org upgrades to (same for both products). */
export function upgradePlanName(_plan: PlanTier): string {
  return "Pro";
}

/** Display names of every plan `isPaidPlan` accepts, e.g. "Pro, AISchemaGate Pro, or Enterprise". */
export function paidPlansPhrase(): string {
  const names = PAID_PLANS.map(planDisplayName);
  if (names.length === 0) return "";
  if (names.length === 1) return names[0];
  if (names.length === 2) return `${names[0]} or ${names[1]}`;
  return `${names.slice(0, -1).join(", ")}, or ${names[names.length - 1]}`;
}

export interface OrgInfo {
  org_id: string;
  org_name: string;
  slug: string;
  role: "owner" | "admin" | "member";
  /** RFC-045: billing plan tier. Defaults to "free" for legacy rows. */
  plan: PlanTier;
  /** Stripe subscription status (trialing, active, past_due, canceled, etc.) */
  plan_status?: string | null;
  stripe_customer_id?: string | null;
  stripe_subscription_id?: string | null;
}

/**
 * Shape of the joined `orgs(name, slug)` cell.  Supabase's generated
 * types render foreign-key joins as either `T | null` (one-to-one) or
 * `T[]` (one-to-many) depending on schema introspection — and since we
 * read it as `data.orgs` with `.single()` either flavour can land at
 * runtime.  Local type so the unwrap below stays narrow and removes the
 * `as unknown` double-cast that was here previously.
 */
type OrgJoinCell = {
  name: string;
  slug: string;
  plan: string;
  plan_status?: string | null;
  stripe_customer_id?: string | null;
  stripe_subscription_id?: string | null;
} | {
  name: string;
  slug: string;
  plan: string;
  plan_status?: string | null;
  stripe_customer_id?: string | null;
  stripe_subscription_id?: string | null;
}[] | null;

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
 * Earliest membership by `joined_at`. A single-org user has one row.
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
        plan: "growth" as PlanTier, // demo shows Growth features
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
          .select("org_id, role, orgs(name, slug, plan, plan_status, stripe_customer_id, stripe_subscription_id)")
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
        const rawPlan = orgsRow?.plan ?? "free";
        const plan: PlanTier = (KNOWN_PLANS as string[]).includes(rawPlan)
          ? (rawPlan as PlanTier)
          : "free";
        setOrg({
          org_id: data.org_id as string,
          org_name: orgsRow?.name ?? "My Org",
          slug: orgsRow?.slug ?? "",
          role: (data.role as OrgInfo["role"]) ?? "member",
          plan,
          plan_status: orgsRow?.plan_status ?? null,
          stripe_customer_id: orgsRow?.stripe_customer_id ?? null,
          stripe_subscription_id: orgsRow?.stripe_subscription_id ?? null,
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
