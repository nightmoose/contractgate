/**
 * Public contract catalog — detail page (RFC public-catalog phase 1).
 *
 * Server-rendered on the request, NO AuthGate. Anonymous visitors and crawlers
 * get HTTP 200 with HTML including canonical, Open Graph, and JSON-LD `Dataset`.
 *
 * Do not opt this segment into ISR (`revalidate` + `generateStaticParams`).
 * The root layout reads the brand cookie in `generateMetadata` and while
 * rendering the shell. That read is valid on a dynamic render and throws
 * DYNAMIC_SERVER_USAGE during static generation. When `generateStaticParams`
 * returned no paths (the catalog API was unreachable at build), Next.js never
 * prerendered one, so it never saw that bailout and left the route as
 * on-demand SSG. The request then failed in MetadataOutlet with HTTP 500
 * while the page body was already in the RSC payload. `force-dynamic` keeps
 * the route dynamic in that build so metadata resolves on the request.
 */
import type { Metadata } from "next";
import Link from "next/link";
import { notFound } from "next/navigation";
import {
  fetchCatalogDetail,
  parseContractFields,
  yamlDownloadUrl,
  odcsDownloadUrl,
  curlSnippet,
  SITE_URL,
} from "@/lib/catalog";
import { catalogHeaderAside } from "@/lib/catalog-copy";
import CatalogDisclaimer from "../CatalogDisclaimer";
import DetailActions from "./DetailActions";

export const dynamic = "force-dynamic";

export async function generateMetadata({
  params,
}: {
  params: Promise<{ slug: string }>;
}): Promise<Metadata> {
  const { slug } = await params;
  const detail = await fetchCatalogDetail(slug);
  if (!detail) return { title: "Contract not found" };
  const vendor = detail.vendor ?? detail.name;
  const title = `${vendor} ${detail.category ?? ""} — data contract`.trim();
  const description =
    detail.description ??
    `Data contract compatible with ${vendor}'s documented schema.`;
  const url = `${SITE_URL}/catalog/${slug}`;
  return {
    title,
    description,
    alternates: { canonical: url },
    openGraph: { title: `${title} — DataContractGate`, description, url, type: "article" },
    twitter: { card: "summary_large_image", title, description },
  };
}

export default async function CatalogDetailPage({
  params,
}: {
  params: Promise<{ slug: string }>;
}) {
  const { slug } = await params;
  const detail = await fetchCatalogDetail(slug);
  if (!detail) notFound();

  const vendor = detail.vendor ?? detail.name;
  const fields = parseContractFields(detail.contract_yaml);
  const samples = Array.isArray(detail.sample_json) ? detail.sample_json : [];
  const whoUses = `Validate a client's ${vendor} export before it hits your warehouse.`;

  const dataset = {
    "@context": "https://schema.org",
    "@type": "Dataset",
    name: `${vendor} data contract`,
    description: detail.description ?? undefined,
    license: "https://opensource.org/licenses/MIT",
    creator: { "@type": "Organization", name: "DataContractGate" },
    isBasedOn: detail.verified_source_url ?? undefined,
    dateModified: detail.updated_at,
    version: detail.version,
    distribution: [
      {
        "@type": "DataDownload",
        encodingFormat: "application/yaml",
        contentUrl: yamlDownloadUrl(slug),
      },
    ],
  };

  return (
    <div className="max-w-4xl mx-auto">
      <script
        type="application/ld+json"
        dangerouslySetInnerHTML={{ __html: JSON.stringify(dataset) }}
      />

      {/* Breadcrumb */}
      <Link href="/catalog" className="text-xs text-slate-500 hover:text-teal-400 transition-colors">
        ← Catalog
      </Link>

      {/* Header */}
      <header className="mt-2 mb-6">
        <span className="text-[10px] uppercase tracking-wider text-teal-400 bg-teal-900/20 border border-teal-800/40 rounded-full px-2 py-0.5">
          {detail.category ?? "Contract"}
        </span>
        <h1 className="text-2xl font-bold text-slate-100 mt-3">
          {vendor} — data contract{" "}
          <span className="text-slate-500 font-normal text-lg">
            {catalogHeaderAside(slug, vendor)}
          </span>
        </h1>
        <p className="text-sm text-slate-400 mt-3 leading-relaxed">{detail.description}</p>
        <p className="text-sm text-teal-300/90 mt-3">{whoUses}</p>
      </header>

      {/* CTAs */}
      <div className="mb-8">
        <DetailActions slug={slug} yamlUrl={yamlDownloadUrl(slug)} curl={curlSnippet(slug)} />
        <p className="text-xs text-slate-600 mt-3">
          Also available:{" "}
          <a href={odcsDownloadUrl(slug)} className="text-slate-400 hover:text-teal-300 underline">
            ODCS v3.1.0 YAML
          </a>
          . &ldquo;Use this contract&rdquo; forks it into your org, then opens the validation
          playground so you can upload the client file.
        </p>
      </div>

      {/* Field table */}
      <section className="mb-8">
        <h2 className="text-base font-semibold text-slate-100 mb-3">Fields</h2>
        <div className="overflow-x-auto border border-[#1f2937] rounded-xl">
          <table className="w-full text-sm">
            <thead>
              <tr className="text-left text-xs uppercase tracking-wider text-slate-500 border-b border-[#1f2937]">
                <th className="px-4 py-2.5 font-medium">Name</th>
                <th className="px-4 py-2.5 font-medium">Type</th>
                <th className="px-4 py-2.5 font-medium">Required</th>
                <th className="px-4 py-2.5 font-medium">Rule</th>
                <th className="px-4 py-2.5 font-medium">Description</th>
              </tr>
            </thead>
            <tbody className="divide-y divide-[#1f2937]/60">
              {fields.map((f) => (
                <tr key={f.name} className="text-slate-300">
                  <td className="px-4 py-2.5 font-mono text-teal-300 whitespace-nowrap">{f.name}</td>
                  <td className="px-4 py-2.5 text-slate-400">{f.type}</td>
                  <td className="px-4 py-2.5">
                    {f.required ? (
                      <span className="text-amber-400">required</span>
                    ) : (
                      <span className="text-slate-600">optional</span>
                    )}
                  </td>
                  <td className="px-4 py-2.5 text-slate-400 text-xs">{f.rule || "—"}</td>
                  <td className="px-4 py-2.5 text-slate-500 text-xs">{f.description || "—"}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </section>

      {/* Samples */}
      <section className="mb-8">
        <h2 className="text-base font-semibold text-slate-100 mb-1">Sample rows</h2>
        <p className="text-xs text-slate-600 mb-3">
          Three synthetic rows we wrote. They validate against this contract with zero
          violations. No real customer data.
        </p>
        <pre className="text-[11px] text-green-300 font-mono bg-[#0a0d12] border border-[#1f2937] rounded-lg p-4 overflow-auto max-h-80 leading-relaxed">
          {JSON.stringify(samples, null, 2)}
        </pre>
      </section>

      {/* Provenance */}
      <section className="mb-8 bg-[#111827] border border-[#1f2937] rounded-xl p-5 text-sm">
        <h2 className="text-base font-semibold text-slate-100 mb-3">Provenance</h2>
        <dl className="space-y-1.5 text-slate-400">
          <div className="flex gap-2">
            <dt className="text-slate-500 w-32 shrink-0">Verified against</dt>
            <dd>
              {detail.verified_source_url ? (
                <a
                  href={detail.verified_source_url}
                  target="_blank"
                  rel="noopener noreferrer nofollow"
                  className="text-teal-400 hover:text-teal-300 break-all"
                >
                  {detail.verified_source_url}
                </a>
              ) : (
                "—"
              )}
            </dd>
          </div>
          <div className="flex gap-2">
            <dt className="text-slate-500 w-32 shrink-0">Checked</dt>
            <dd>{detail.verified_at ?? "—"}</dd>
          </div>
          <div className="flex gap-2">
            <dt className="text-slate-500 w-32 shrink-0">Version</dt>
            <dd>v{detail.version}</dd>
          </div>
          <div className="flex gap-2">
            <dt className="text-slate-500 w-32 shrink-0">Author</dt>
            <dd>DataContractGate</dd>
          </div>
          <div className="flex gap-2">
            <dt className="text-slate-500 w-32 shrink-0">License</dt>
            <dd>{detail.license_note ?? "MIT"}</dd>
          </div>
        </dl>
        <p className="text-xs text-slate-600 mt-4">
          Spotted a mismatch with the vendor&rsquo;s current schema?{" "}
          <a
            href={`https://github.com/nightmoose/contractgate/issues/new?title=${encodeURIComponent(
              `Catalog issue: ${slug}`
            )}`}
            target="_blank"
            rel="noopener noreferrer"
            className="text-teal-400 hover:text-teal-300"
          >
            Report an issue
          </a>
          .
        </p>
      </section>

      <CatalogDisclaimer />
    </div>
  );
}
