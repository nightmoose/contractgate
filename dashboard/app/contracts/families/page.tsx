"use client";

import Link from "next/link";
import useSWR from "swr";
import AuthGate from "@/components/AuthGate";
import { listContractFamilies, type ContractFamilySummary } from "@/lib/api";

function FamiliesList() {
  const { data, error, isLoading } = useSWR<ContractFamilySummary[]>(
    "contract-families",
    listContractFamilies
  );

  return (
    <div className="max-w-3xl">
      <div className="flex items-start justify-between gap-4 mb-6">
        <div>
          <h1 className="text-2xl font-bold text-slate-100">Families</h1>
          <p className="text-sm text-slate-500 mt-1">
            A family is one dataset as it moves from ingest, through the stages in between, to egress.
          </p>
        </div>
        <Link
          href="/contracts/families/new"
          className="shrink-0 px-4 py-2 bg-green-600 hover:bg-green-500 text-white text-sm font-medium rounded-lg transition-colors"
        >
          + New family
        </Link>
      </div>

      {isLoading ? <p className="text-sm text-slate-500">Loading…</p> : null}
      {error ? (
        <p className="text-sm text-red-400 bg-red-900/20 border border-red-800/40 rounded p-3">
          {error instanceof Error ? error.message : String(error)}
        </p>
      ) : null}

      {data && data.length === 0 ? (
        <div className="bg-[#111827] border border-[#1f2937] rounded-xl p-8 text-center">
          <p className="text-sm text-slate-400">No families yet.</p>
          <p className="text-xs text-slate-500 mt-2">
            Create one from contracts you already have, or{" "}
            <Link href="/catalog" className="text-teal-300 hover:text-teal-200">
              fork a catalog pipeline
            </Link>
            .
          </p>
        </div>
      ) : null}

      <ul className="space-y-3">
        {data?.map((family) => (
          <li key={family.id}>
            <Link
              href={`/contracts/families/${family.id}`}
              className="block bg-[#111827] border border-[#1f2937] rounded-xl p-5 hover:border-green-800/50 transition-colors"
            >
              <div className="flex items-baseline justify-between gap-3">
                <h2 className="font-semibold text-slate-100">{family.name}</h2>
                <span className="text-xs text-slate-500 shrink-0">
                  {family.link_count} contract{family.link_count === 1 ? "" : "s"}
                </span>
              </div>
              {family.description ? (
                <p className="text-sm text-slate-500 mt-1.5 line-clamp-2">{family.description}</p>
              ) : null}
            </Link>
          </li>
        ))}
      </ul>
    </div>
  );
}

export default function FamiliesPage() {
  return (
    <AuthGate page="contracts">
      <FamiliesList />
    </AuthGate>
  );
}
