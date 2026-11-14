/**
 * Public contract catalog — index (RFC public-catalog phase 1).
 *
 * Server-rendered, data revalidated about once a minute, NO AuthGate. Anonymous visitors (and crawlers)
 * get HTTP 200 with HTML including JSON-LD. The signed-in "Browse & Import" +
 * Egress Validator surface moved to /catalog/library.
 */
import type { Metadata } from "next";
import Link from "next/link";
import { fetchPublicCatalog, fetchPublicFamilies, SITE_URL } from "@/lib/catalog";
import CatalogBrowser from "./CatalogBrowser";
import CatalogDisclaimer from "./CatalogDisclaimer";

export const revalidate = 60;

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
  let families: Awaited<ReturnType<typeof fetchPublicFamilies>> = [];
  try {
    contracts = await fetchPublicCatalog();
  } catch {
    // The API may be unreachable at build time; ISR revalidation will fill this
    // in. Render the shell with an empty list rather than failing the build.
    contracts = [];
  }
  try {
    families = await fetchPublicFamilies();
  } catch {
    families = [];
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

      {families.length > 0 ? (
        <section className="mb-8" aria-labelledby="catalog-pipelines-heading">
          <h2 id="catalog-pipelines-heading" className="text-base font-semibold text-slate-100 mb-1">
            Pipelines
          </h2>
          <p className="text-sm text-slate-500 mb-3">
            A family is the same data at each boundary. Fork the whole chain into your org in one step.
          </p>
          <ul className="grid grid-cols-1 sm:grid-cols-2 gap-3">
            {families.map((family) => (
              <li key={family.id}>
                <Link
                  href={`/catalog/families/${family.slug}`}
                  className="block bg-[#111827] border border-[#1f2937] rounded-xl p-4 hover:border-teal-700/50 transition-colors"
                >
                  <p className="text-sm font-semibold text-slate-100">{family.name}</p>
                  <p className="text-xs text-slate-500 mt-1 line-clamp-2">
                    {family.description ?? "Ingest, then between stages, then egress."}
                  </p>
                  <p className="text-[10px] uppercase tracking-wider text-teal-400 mt-2">
                    {family.links.length} contract{family.links.length === 1 ? "" : "s"}
                  </p>
                </Link>
              </li>
            ))}
          </ul>
        </section>
      ) : null}

      <CatalogBrowser contracts={contracts} />

      <CatalogDisclaimer />
    </div>
  );
}
