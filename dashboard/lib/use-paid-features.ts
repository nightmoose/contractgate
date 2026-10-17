"use client";

import useSWR from "swr";
import { getMe } from "@/lib/api";
import { isPaidPlan, useOrg } from "@/lib/org";
import { paidFeatureAccess } from "@/lib/paid-features";

/**
 * Whether passed-event export, Connect Fivetran, and payload storage are open.
 * Reads `paid_features` from `GET /v1/me`. `null` until that response settles.
 * Falls back to `isPaidPlan(org.plan)` when the field is missing or the
 * request fails.
 */
export function usePaidFeatures(): boolean | null {
  const { org } = useOrg();
  const { data, error, isLoading } = useSWR(org ? "/v1/me" : null, () => getMe());
  if (!org) return null;
  if (typeof data?.paid_features === "boolean") return data.paid_features;
  if (data || error) return paidFeatureAccess(data?.paid_features, isPaidPlan(org.plan));
  if (isLoading) return null;
  return isPaidPlan(org.plan);
}
