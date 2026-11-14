"use client";

import { use, useMemo, useState } from "react";
import Link from "next/link";
import useSWR from "swr";
import AuthGate from "@/components/AuthGate";
import PrivateFamilyChain from "@/components/families/PrivateFamilyChain";
import {
  addContractFamilyLink,
  getContractFamily,
  listContracts,
  type ContractFamily,
  type ContractSummary,
} from "@/lib/api";

const BOUNDARIES = [
  { value: "ingest", label: "Ingest" },
  { value: "stage", label: "Between stages" },
  { value: "egress", label: "Egress" },
];

function nextBoundary(family: ContractFamily | undefined): string {
  const tail = family?.links[family.links.length - 1];
  if (!tail) return "ingest";
  if (tail.boundary === "ingest") return "stage";
  return "egress";
}

function FamilyDetail({ id }: { id: string }) {
  const { data, error, isLoading, mutate } = useSWR<ContractFamily>(
    ["contract-family", id],
    () => getContractFamily(id)
  );
  const { data: contracts } = useSWR<ContractSummary[]>("contracts", listContracts);
  const [contractId, setContractId] = useState("");
  const [boundary, setBoundary] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);

  const used = useMemo(
    () => new Set(data?.links.map((link) => link.contract_id) ?? []),
    [data]
  );
  const available = contracts?.filter((contract) => !used.has(contract.id)) ?? [];
  const chosenBoundary = boundary ?? nextBoundary(data);
  const tail = data && data.links.length > 0 ? data.links[data.links.length - 1] : null;

  async function handleAdd(e: React.FormEvent) {
    e.preventDefault();
    if (!contractId) return;
    setSaving(true);
    setSaveError(null);
    try {
      const updated = await addContractFamilyLink(id, {
        contract_id: contractId,
        boundary: chosenBoundary,
        ...(tail ? { derived_from_contract_id: tail.contract_id } : {}),
      });
      setContractId("");
      setBoundary(null);
      await mutate(updated, { revalidate: false });
    } catch (err) {
      setSaveError(err instanceof Error ? err.message : String(err));
    } finally {
      setSaving(false);
    }
  }

  return (
    <div className="max-w-3xl">
      <Link href="/contracts/families" className="text-xs text-slate-500 hover:text-green-400">
        ← Families
      </Link>

      {isLoading ? <p className="text-sm text-slate-500 mt-6">Loading…</p> : null}
      {error ? (
        <p className="mt-6 text-sm text-red-400 bg-red-900/20 border border-red-800/40 rounded p-3">
          {error instanceof Error ? error.message : String(error)}
        </p>
      ) : null}

      {data ? (
        <>
          <header className="mt-2 mb-6">
            <h1 className="text-2xl font-bold text-slate-100">{data.name}</h1>
            {data.description ? (
              <p className="text-sm text-slate-400 mt-2 leading-relaxed">{data.description}</p>
            ) : (
              <p className="text-sm text-slate-500 mt-2">
                Ingest, then between stages, then egress.
              </p>
            )}
          </header>

          {data.links.length > 0 ? (
            <PrivateFamilyChain family={data} />
          ) : (
            <p className="text-sm text-slate-500 bg-[#111827] border border-[#1f2937] rounded-xl p-5">
              This family has no contracts yet. Add the root below.
            </p>
          )}

          <form
            onSubmit={handleAdd}
            className="mt-6 bg-[#111827] border border-[#1f2937] rounded-xl p-5 space-y-4"
          >
            <h2 className="text-sm font-semibold text-slate-100">
              {tail ? "Add the next link" : "Add the root"}
            </h2>
            <p className="text-xs text-slate-500">
              {tail
                ? `The new contract is derived from ${tail.name}. Pick one that is not already in a family.`
                : "The first contract has no parent. It is the start of the chain."}
            </p>
            <label className="block">
              <span className="text-xs uppercase tracking-wider text-slate-500">Contract</span>
              <select
                value={contractId}
                onChange={(e) => setContractId(e.target.value)}
                className="mt-1 w-full bg-[#0a0d12] border border-[#1f2937] rounded-lg px-3 py-2 text-sm text-slate-100 outline-none focus:border-green-700"
              >
                <option value="">Select a contract</option>
                {available.map((contract) => (
                  <option key={contract.id} value={contract.id}>
                    {contract.name}
                  </option>
                ))}
              </select>
            </label>
            <label className="block">
              <span className="text-xs uppercase tracking-wider text-slate-500">Boundary</span>
              <select
                value={chosenBoundary}
                onChange={(e) => setBoundary(e.target.value)}
                className="mt-1 w-full bg-[#0a0d12] border border-[#1f2937] rounded-lg px-3 py-2 text-sm text-slate-100 outline-none focus:border-green-700"
              >
                {BOUNDARIES.map((option) => (
                  <option key={option.value} value={option.value}>
                    {option.label}
                  </option>
                ))}
              </select>
            </label>
            {saveError ? (
              <p className="text-sm text-red-400 bg-red-900/20 border border-red-800/40 rounded p-3">
                {saveError}
              </p>
            ) : null}
            <button
              type="submit"
              disabled={saving || !contractId}
              className="px-4 py-2 bg-green-600 hover:bg-green-500 disabled:opacity-50 text-white text-sm font-medium rounded-lg transition-colors"
            >
              {saving ? "Adding…" : tail ? "Add next link" : "Add root"}
            </button>
          </form>
        </>
      ) : null}
    </div>
  );
}

export default function FamilyDetailPage({
  params,
}: {
  params: Promise<{ id: string }>;
}) {
  const { id } = use(params);
  return (
    <AuthGate page="contracts">
      <FamilyDetail id={id} />
    </AuthGate>
  );
}
