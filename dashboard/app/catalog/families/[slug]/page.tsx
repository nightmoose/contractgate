/**
 * Public catalog family — the whole pipeline, with a one-step fork.
 *
 * Server-rendered, no AuthGate. Logged-out visitors see the chain; "Fork this
 * pipeline" sends them to signup and back to the fork step.
 */
import type { Metadata } from "next";
import Link from "next/link";
import { notFound } from "next/navigation";
import { fetchPublicFamily, SITE_URL } from "@/lib/catalog";
import { boundaryLabel } from "@/lib/contract-family";
import CatalogDisclaimer from "../../CatalogDisclaimer";
import ForkPipelineButton from "@/components/catalog/ForkPipelineButton";

export const dynamic = "force-dynamic";

export async function generateMetadata({
  params,
}: {
  params: Promise<{ slug: string }>;
}): Promise<Metadata> {
  const { slug } = await params;
  const family = await fetchPublicFamily(slug);
  if (!family) return { title: "Pipeline not found" };
  const title = `${family.name} — contract family`;
  const description =
    family.description ??
    "One dataset checked at ingest, between stages, and at egress.";
  const url = `${SITE_URL}/catalog/families/${slug}`;
  return {
    title,
    description,
    alternates: { canonical: url },
    openGraph: { title: `${title} — DataContractGate`, description, url, type: "article" },
  };
}

export default async function CatalogFamilyPage({
  params,
}: {
  params: Promise<{ slug: string }>;
}) {
  const { slug } = await params;
  const family = await fetchPublicFamily(slug);
  if (!family) notFound();

  return (
    <div className="max-w-4xl mx-auto">
      <Link href="/catalog" className="text-xs text-slate-500 hover:text-teal-400 transition-colors">
        ← Catalog
      </Link>

      <header className="mt-2 mb-6">
        <span className="text-[10px] uppercase tracking-wider text-teal-400 bg-teal-900/20 border border-teal-800/40 rounded-full px-2 py-0.5">
          Pipeline
        </span>
        <h1 className="text-2xl font-bold text-slate-100 mt-3">{family.name}</h1>
        <p className="text-sm text-slate-400 mt-3 leading-relaxed">
          {family.description ??
            "The same data as it changes shape: ingest, then between stages, then egress."}
        </p>
      </header>

      <div className="mb-3">
        <ForkPipelineButton slug={family.slug} />
      </div>
      <p className="text-xs text-slate-500 mb-8 max-w-2xl leading-relaxed">
        Forks every contract in this chain into your org and keeps the same
        boundaries. The Free plan allows 3 contracts. A longer pipeline, or an
        org that already has contracts, needs room under that cap or a paid plan.
        Logged-out visitors sign up first.
      </p>

      <ol className="border border-[#1f2937] rounded-xl divide-y divide-[#1f2937]/60 mb-8">
        {family.links.map((link) => {
          const href = link.slug ? `/catalog/${link.slug}` : null;
          return (
            <li
              key={link.public_contract_id}
              className="px-4 py-3 flex flex-col gap-1 sm:flex-row sm:items-baseline sm:justify-between"
            >
              <div className="min-w-0">
                <div className="flex flex-wrap items-baseline gap-x-2 gap-y-0.5">
                  <span className="text-xs text-slate-600 tabular-nums w-4">{link.position + 1}</span>
                  {href ? (
                    <Link
                      href={href}
                      className="font-mono text-sm text-teal-300 hover:text-teal-200 break-all"
                    >
                      {link.name}
                    </Link>
                  ) : (
                    <span className="font-mono text-sm text-slate-100 break-all">{link.name}</span>
                  )}
                </div>
                {link.derived_from_name ? (
                  <p className="text-xs text-slate-500 pl-6">Derived from {link.derived_from_name}</p>
                ) : (
                  <p className="text-xs text-slate-500 pl-6">Start of the chain</p>
                )}
              </div>
              <span className="text-[10px] uppercase tracking-wider text-slate-400 sm:shrink-0 sm:pl-4 pl-6">
                {boundaryLabel(link.boundary)}
              </span>
            </li>
          );
        })}
      </ol>

      <CatalogDisclaimer />
    </div>
  );
}
