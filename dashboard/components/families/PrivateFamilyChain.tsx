import Link from "next/link";
import type { ContractFamily } from "@/lib/api";
import { boundaryLabel } from "@/lib/contract-family";

/** Private family chain: ingest, then between stages, then egress. Each name opens the contract. */
export default function PrivateFamilyChain({ family }: { family: ContractFamily }) {
  return (
    <ol className="border border-[#1f2937] rounded-xl divide-y divide-[#1f2937]/60">
      {family.links.map((link) => (
        <li
          key={link.contract_id}
          className="px-4 py-3 flex flex-col gap-1 sm:flex-row sm:items-baseline sm:justify-between"
        >
          <div className="min-w-0">
            <div className="flex flex-wrap items-baseline gap-x-2 gap-y-0.5">
              <span className="text-xs text-slate-600 tabular-nums w-4">{link.position + 1}</span>
              <Link
                href={`/contracts?contract=${encodeURIComponent(link.contract_id)}`}
                className="font-mono text-sm text-green-300 hover:text-green-200 break-all"
              >
                {link.name}
              </Link>
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
      ))}
    </ol>
  );
}
