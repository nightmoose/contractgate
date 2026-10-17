/**
 * `paidFeatures` is `GET /v1/me` `paid_features`. A boolean wins, including
 * `true` on a self-hosted free org. A missing field falls back to `planPaid`.
 */
export function paidFeatureAccess(
  paidFeatures: boolean | null | undefined,
  planPaid: boolean,
): boolean {
  if (typeof paidFeatures === "boolean") return paidFeatures;
  return planPaid;
}

/**
 * Pro-tier gates follow `paid_features`. Enterprise gates follow the stored
 * plan unless `selfHosted` is true, which unlocks every tier.
 */
export function tierOpen(
  selfHosted: boolean,
  paidFeatures: boolean | null | undefined,
  storedPlanOpen: boolean,
  enterpriseGate: boolean,
): boolean {
  if (selfHosted) return true;
  if (enterpriseGate) return storedPlanOpen;
  return paidFeatureAccess(paidFeatures, storedPlanOpen);
}
