/**
 * Public contract catalog — index (RFC public-catalog phase 1).
 *
 * Server-rendered, ISR (~1h), NO AuthGate. Anonymous visitors (and crawlers)
 * get HTTP 200 with HTML including JSON-LD. The signed-in "Browse & Import" +
 * Egress Validator surface moved to /catalog/library.
 */
import type { Metadata } from "next";
import Link from "next/link";
import { fetchPublicCatalog, SITE_URL } from "@/lib/catalog";
import CatalogBrowser from "./CatalogBrowser";
import CatalogDisclaimer from "./CatalogDisclaimer";

export const revalidate = 3600;

export const metadata: Metadata = {
  title: "Data contract catalog",
  description:
    "Ready-to-use data contracts for the client exports you see every day — Stripe, Shopify, QuickBooks, HubSpot, Salesforce, GA4, Square, Xero and more. Pick a source, fork the contract, validate your client's file the same day.",
  alternates: { canonical: `${SITE_URL}/catalog` },
  openGraph: {
    title: "Data contract catalog — DataContractGate",
    description:
      "Pick your client's source, fork the contract, validate the file the same day. No contract writing up front.",
    url: `${SITE_URL}/catalog`,
    type: "website",
  },
};

export default async function CatalogIndexPage() {
  let contracts: Awaited<ReturnType<typeof fetchPublicCatalog>> = [];
  try {
    contracts = await fetchPublicCatalog();
  } catch {
    // The API may be unreachable at build time; ISR revalidation will fill this
    // in. Render the shell with an empty list rather than failing the build.
    contracts = [];
  }

  const itemList = {
    "@context": "https://schema.org",
    "@type": "ItemList",
    name: "DataContractGate contract catalog",
    numberOfItems: contracts.length,
    itemListElement: contracts.map((c, i) => ({
      "@type": "ListItem",
      position: i + 1,
      url: `${SITE_URL}/catalog/${c.slug}`,
      name: c.vendor ?? c.name,
    })),
  };

  return (
    <div className="max-w-5xl mx-auto">
      <script
        type="application/ld+json"
        dangerouslySetInnerHTML={{ __html: JSON.stringify(itemList) }}
      />

      {/* Hero */}
      <div className="mb-8">
        <h1 className="text-3xl font-bold text-slate-100">Data contract catalog</h1>
        <p className="text-base text-slate-400 mt-3 max-w-2xl leading-relaxed">
          You get the same client exports over and over. Pick the source, fork the
          contract, and start validating your client&rsquo;s file the same day — no
          contract writing up front.
        </p>
        <div className="mt-4 flex items-center gap-4 text-sm">
          <Link href="/catalog/library" className="text-teal-400 hover:text-teal-300">
            Import &amp; validate in your library →
          </Link>
          <a
            href="/catalog.md"
            className="text-slate-500 hover:text-slate-300"
          >
            Agent-friendly listing (/catalog.md)
          </a>
        </div>
      </div>

      <CatalogBrowser contracts={contracts} />

      <CatalogDisclaimer />
    </div>
  );
}
