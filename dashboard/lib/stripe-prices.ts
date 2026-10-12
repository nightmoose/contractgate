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
