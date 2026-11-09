/**
 * Affiliation and card lines for the public catalog.
 *
 * The detail-page header and the index card used to stamp
 * "compatible with {vendor}; not affiliated with {vendor}" onto every row.
 * That is right for a vendor schema and wrong for two launch rows:
 * GA4's company is Google, not the product name, and the generic CSV is an
 * open column set authored here — it must not say DataContractGate is
 * compatible with, or not affiliated with, itself.
 */

const GENERIC_SLUG = "generic-sheets-transactions-csv";
const GA4_SLUG = "ga4-bigquery-events";

// Pipeline-stage contracts we wrote. They are not a vendor's documented
// object, so the shared "compatible with {vendor}'s documented schema" line
// would over-claim. Not affiliated with Stripe.
const PIPELINE_ASIDE: Record<string, string> = {
  "stripe-charges-raw":
    "(raw Stripe Charge export at ingest; not affiliated with Stripe)",
  "stripe-charges-staged":
    "(cleaned Stripe charge row between stages; not affiliated with Stripe)",
  "revenue-daily-mart":
    "(daily revenue aggregate between stages; not affiliated with Stripe)",
  "partner-payout-feed":
    "(outbound partner payout feed at egress; not affiliated with Stripe)",
};

const PIPELINE_CARD: Record<string, string> = {
  "stripe-charges-raw": "raw Stripe charges at ingest",
  "stripe-charges-staged": "cleaned Stripe charges between stages",
  "revenue-daily-mart": "daily revenue mart between stages",
  "partner-payout-feed": "partner payout feed at egress",
};

/** Parenthetical under the detail-page title. Includes the wrapping parentheses. */
export function catalogHeaderAside(
  slug: string | null | undefined,
  vendor: string
): string {
  if (slug === GENERIC_SLUG) {
    return "(open spreadsheet / CSV transactions export)";
  }
  if (slug && PIPELINE_ASIDE[slug]) {
    return PIPELINE_ASIDE[slug];
  }
  if (slug === GA4_SLUG) {
    return "(compatible with GA4’s documented BigQuery export schema; not affiliated with Google)";
  }
  return `(compatible with ${vendor}’s documented schema; not affiliated with ${vendor})`;
}

/** One-line footer on a catalog index card. */
export function catalogCardLine(
  slug: string | null | undefined,
  vendor: string | null | undefined
): string {
  if (slug === GENERIC_SLUG) {
    return "generic spreadsheet / CSV transactions export";
  }
  if (slug && PIPELINE_CARD[slug]) {
    return PIPELINE_CARD[slug];
  }
  return `compatible with ${vendor ?? "the documented schema"}`;
}
