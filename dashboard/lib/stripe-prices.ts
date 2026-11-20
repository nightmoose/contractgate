import type { ConfiguredPrices } from "./billing-plan";

// Pro prices: one Stripe product ("Pro") serves DataContractGate and
// AISchemaGate (decided 2026-10-12). STRIPE_PRICE_PRO_* is the name going
// forward; STRIPE_PRICE_GROWTH_* is still read so a deploy works whichever
// name Vercel has. Server-only.
export function proPriceMonthly(): string | undefined {
  return process.env.STRIPE_PRICE_PRO_MONTHLY || process.env.STRIPE_PRICE_GROWTH_MONTHLY;
}

export function proPriceAnnual(): string | undefined {
  return process.env.STRIPE_PRICE_PRO_ANNUAL || process.env.STRIPE_PRICE_GROWTH_ANNUAL;
}

// Pro + Impact (stored plan `impact`): one Stripe product with a monthly
// ($499) and an annual ($4,990) recurring price. Unset means self-serve
// checkout is off; the pricing card shows "Talk to us" instead. Server-only.
export function impactPriceMonthly(): string | undefined {
  return process.env.STRIPE_PRICE_IMPACT_MONTHLY || undefined;
}

export function impactPriceAnnual(): string | undefined {
  return process.env.STRIPE_PRICE_IMPACT_ANNUAL || undefined;
}

/** Which Pro + Impact billing periods can start a Stripe Checkout. */
export function impactCheckoutAvailability(): { monthly: boolean; annual: boolean } {
  return { monthly: !!impactPriceMonthly(), annual: !!impactPriceAnnual() };
}

export function configuredPrices(): ConfiguredPrices {
  return {
    pro: [proPriceMonthly(), proPriceAnnual()],
    impact: [impactPriceMonthly(), impactPriceAnnual()],
  };
}
