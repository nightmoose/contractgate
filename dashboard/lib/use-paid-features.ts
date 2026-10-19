"use client";

import useSWR from "swr";
import { getMe } from "@/lib/api";
import { isPaidPlan, useOrg } from "@/lib/org";
import { paidFeatureAccess } from "@/lib/paid-features";

/**
 * `paid_features` from `GET /v1/me`.
 *
 * `null` until the response settles. Callers use it for Pro-tier gates.
 * A missing field or a failed request falls back to `isPaidPlan(org.plan)`.
 * Enterprise-only gates use `useSelfHosted` plus the stored plan.
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

/**
 * `self_hosted` from `GET /v1/me`. `null` until the response settles.
 * A missing field or a failed request is false, which keeps hosted gates
 * on the stored plan.
 */
export function useSelfHosted(): boolean | null {
  const { org } = useOrg();
  const { data, error, isLoading } = useSWR(org ? "/v1/me" : null, () => getMe());
  if (!org) return null;
  if (typeof data?.self_hosted === "boolean") return data.self_hosted;
  if (data || error || !isLoading) return false;
  return null;
}
