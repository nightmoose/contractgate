import Link from "next/link";
import type { PublicContractFamily } from "@/lib/api";
import { boundaryLabel } from "@/lib/contract-family";
import ForkPipelineButton from "@/components/catalog/ForkPipelineButton";

/**
 * The public catalog chain for one contract: root → leaf, with a link to
 * every neighbor. The current contract is marked and is not a link.
 */
export default function ContractFamilyChain({
  family,
  currentSlug,
}: {
  family: PublicContractFamily;
  currentSlug: string;
}) {
  return (
    <section className="mb-8" aria-labelledby="contract-family-heading">
      <h2 id="contract-family-heading" className="text-base font-semibold text-slate-100 mb-1">
        Contract family
      </h2>
      <p className="text-sm text-slate-400 mb-1">{family.name}</p>
      {family.description ? (
        <p className="text-xs text-slate-500 mb-3 leading-relaxed">{family.description}</p>
      ) : (
        <p className="text-xs text-slate-500 mb-3">
          The same data as it changes shape through the pipeline.
        </p>
      )}
      <ol className="border border-[#1f2937] rounded-xl divide-y divide-[#1f2937]/60">
        {family.links.map((link) => {
          const current = link.slug === currentSlug;
          const href = link.slug ? `/catalog/${link.slug}` : null;
          return (
            <li
              key={link.public_contract_id}
              className="px-4 py-3 flex flex-col gap-1 sm:flex-row sm:items-baseline sm:justify-between"
              aria-current={current ? "page" : undefined}
            >
              <div className="min-w-0">
                <div className="flex flex-wrap items-baseline gap-x-2 gap-y-0.5">
                  <span className="text-xs text-slate-600 tabular-nums w-4">
                    {link.position + 1}
                  </span>
                  {current || !href ? (
                    <span className="font-mono text-sm text-slate-100 break-all">{link.name}</span>
                  ) : (
                    <Link
                      href={href}
                      className="font-mono text-sm text-teal-300 hover:text-teal-200 break-all"
                    >
                      {link.name}
                    </Link>
                  )}
                  {current ? (
                    <span className="text-[10px] uppercase tracking-wider text-teal-400">
                      This contract
                    </span>
                  ) : null}
                </div>
                {link.derived_from_name ? (
                  <p className="text-xs text-slate-500 pl-6">
                    Derived from {link.derived_from_name}
                  </p>
                ) : null}
              </div>
              <span className="text-[10px] uppercase tracking-wider text-slate-400 sm:shrink-0 sm:pl-4 pl-6">
                {boundaryLabel(link.boundary)}
              </span>
            </li>
          );
          })}
      </ol>
      <div className="mt-3 flex flex-wrap items-center gap-3">
        <Link
          href={`/catalog/families/${family.slug}`}
          className="text-sm text-teal-300 hover:text-teal-200"
        >
          View pipeline
        </Link>
        <ForkPipelineButton slug={family.slug} />
      </div>
    </section>
  );
}
