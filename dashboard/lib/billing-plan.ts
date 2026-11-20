// Pure plan/price mapping for Stripe checkout and webhooks. No env access, so
// it is unit-tested directly; callers pass the configured price ids from
// lib/stripe-prices.ts.

export type PaidStripePlan = "growth" | "aisg_pro" | "impact";
export type FreeStripePlan = "free" | "aisg_free";

/** Self-serve plans a checkout can buy. Pro covers both products. */
export type CheckoutPlan = "pro" | "impact";

export interface ConfiguredPrices {
  /** Pro monthly and annual price ids (either may be unset). */
  pro: Array<string | undefined>;
  /** Pro + Impact monthly and annual price ids (either may be unset). */
  impact: Array<string | undefined>;
}

export function isAisgPlan(plan: string | null | undefined): boolean {
  return plan === "aisg_free" || plan === "aisg_pro";
}

/** Plans that already hold a paid subscription or a managed contract. */
export function isPaidStoredPlan(plan: string | null | undefined): boolean {
  return plan === "growth" || plan === "aisg_pro" || plan === "impact" || plan === "enterprise";
}

/** Free plan on the org's own product line. */
export function freePlanFor(plan: string | null | undefined): FreeStripePlan {
  return isAisgPlan(plan) ? "aisg_free" : "free";
}

function configured(ids: Array<string | undefined>): string[] {
  return ids.filter((p): p is string => !!p);
}

/** True when `priceId` is one of the configured Pro + Impact prices. */
export function isImpactPrice(priceId: string | null | undefined, prices: ConfiguredPrices): boolean {
  return !!priceId && configured(prices.impact).includes(priceId);
}

/** True when `priceId` is one of the configured Pro prices. */
export function isProPrice(priceId: string | null | undefined, prices: ConfiguredPrices): boolean {
  return !!priceId && configured(prices.pro).includes(priceId);
}

/**
 * Plan a completed checkout grants, or null for an unexpected price.
 *
 * The price decides the plan: a configured Pro + Impact price always grants
 * `impact`. Otherwise the existing Pro rule applies (unchanged): a configured
 * Pro price, a missing price id, or a deploy with a Pro price env var unset
 * grants Pro on the buyer's product line (`aisg_pro` or `growth`).
 */
export function planForCheckout(
  priceId: string | null | undefined,
  prices: ConfiguredPrices,
  opts: { aisg: boolean },
): PaidStripePlan | null {
  if (isImpactPrice(priceId, prices)) return "impact";
  const proOk = !priceId || prices.pro.some((p) => !p) || isProPrice(priceId, prices);
  if (!proOk) return null;
  return opts.aisg ? "aisg_pro" : "growth";
}

/**
 * Paid plan for a live (active / trialing / past_due) subscription update.
 *
 * A known price wins, so a plan switch made in Stripe follows the price. With
 * an unknown or missing price the org keeps its own paid plan: `impact` stays
 * `impact` (it used to fall back to `growth`), AISchemaGate stays `aisg_pro`.
 */
export function paidPlanForSubscription(
  currentPlan: string | null | undefined,
  priceId: string | null | undefined,
  prices: ConfiguredPrices,
): PaidStripePlan {
  if (isImpactPrice(priceId, prices)) return "impact";
  if (isProPrice(priceId, prices)) return isAisgPlan(currentPlan) ? "aisg_pro" : "growth";
  if (currentPlan === "impact") return "impact";
  return isAisgPlan(currentPlan) ? "aisg_pro" : "growth";
}

/** Price id a checkout should use, or undefined when it is not configured. */
export function checkoutPrice(
  plan: CheckoutPlan,
  annual: boolean,
  prices: { proMonthly?: string; proAnnual?: string; impactMonthly?: string; impactAnnual?: string },
): string | undefined {
  if (plan === "impact") return annual ? prices.impactAnnual : prices.impactMonthly;
  return annual ? prices.proAnnual : prices.proMonthly;
}
