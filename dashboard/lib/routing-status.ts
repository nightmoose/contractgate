import type { MultiStableResolution } from "./api";

/**
 * What unpinned traffic resolves to, derived from the contract alone
 * (not the version list — `latest_stable_version` is already the
 * server's answer). A stale `contract` after promote/deprecate is exactly
 * what makes this wrong, so keeping the derivation in one place makes
 * that failure mode testable without a live contract object.
 */
export interface RoutingStatus {
  hasStable: boolean;
  version: string | null;
  resolution: MultiStableResolution;
}

export function routingStatus(
  contract: {
    latest_stable_version?: string | null;
    multi_stable_resolution?: MultiStableResolution | null;
  } | null,
): RoutingStatus {
  const version = contract?.latest_stable_version ?? null;
  return {
    hasStable: version !== null,
    version,
    resolution: contract?.multi_stable_resolution === "fallback" ? "fallback" : "strict",
  };
}
