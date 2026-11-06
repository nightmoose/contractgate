/**
 * /catalog.md — a full markdown listing of the public catalog for coding agents.
 *
 * Generated from the live catalog API (never a stale hardcoded page). Served as
 * text/markdown so Claude/Cursor/Codex can ingest the whole directory in one
 * fetch. Listed in proxy.ts PUBLIC_ROUTES and excluded from the auth matcher.
 */
import { fetchPublicCatalog, SITE_URL } from "@/lib/catalog";

export const revalidate = 3600;

export async function GET() {
  let rows: Awaited<ReturnType<typeof fetchPublicCatalog>> = [];
  try {
    rows = await fetchPublicCatalog();
  } catch {
    // Fall through to an empty listing rather than 500 for an agent.
  }

  const lines: string[] = [
    "# DataContractGate contract catalog",
    "",
    "> Ready-to-use data contracts for common client exports (Stripe, Shopify,",
    "> QuickBooks, HubSpot, Salesforce, GA4, Square, Xero, and generic CSVs).",
    "> Pick a source, fork the contract, and validate your client's file the same",
    "> day. Each contract is compatible with the named vendor's documented schema;",
    "> DataContractGate is not affiliated with or endorsed by any of them.",
    "",
    `Catalog index: ${SITE_URL}/catalog`,
    "",
    "## Contracts",
    "",
  ];

  for (const c of rows) {
    const url = `${SITE_URL}/catalog/${c.slug}`;
    const purpose = (c.description ?? "").replace(/\s+/g, " ").trim();
    lines.push(`- [${c.vendor ?? c.name} — ${c.category ?? "contract"}](${url}): ${purpose}`);
  }

  if (rows.length === 0) {
    lines.push("_Catalog is temporarily unavailable._");
  }

  lines.push("");

  return new Response(lines.join("\n"), {
    headers: {
      "content-type": "text/markdown; charset=utf-8",
      "cache-control": "public, max-age=0, s-maxage=3600, stale-while-revalidate=86400",
    },
  });
}
