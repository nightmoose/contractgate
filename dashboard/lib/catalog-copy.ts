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

/** Parenthetical under the detail-page title. Includes the wrapping parentheses. */
export function catalogHeaderAside(
  slug: string | null | undefined,
  vendor: string
): string {
  if (slug === GENERIC_SLUG) {
    return "(open spreadsheet / CSV transactions export)";
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
  return `compatible with ${vendor ?? "the documented schema"}`;
}
