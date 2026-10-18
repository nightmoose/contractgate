/**
 * Gate for the features `plan::gate_paid` covers: passed-event export,
 * Connect Fivetran, and payload storage.
 *
 * `paidFeatures` is `GET /v1/me` `paid_features`. A boolean wins, including
 * `true` on a self-hosted free org. A missing field falls back to whether
 * the stored plan is paid (`isPaidPlan`).
 */
export function paidFeatureAccess(
  paidFeatures: boolean | null | undefined,
  planPaid: boolean,
): boolean {
  if (typeof paidFeatures === "boolean") return paidFeatures;
  return planPaid;
}
