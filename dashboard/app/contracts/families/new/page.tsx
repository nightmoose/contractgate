"use client";

import { useState } from "react";
import Link from "next/link";
import { useRouter } from "next/navigation";
import useSWR from "swr";
import AuthGate from "@/components/AuthGate";
import { createContractFamily, listContracts, type ContractSummary } from "@/lib/api";

const BOUNDARIES = [
  { value: "ingest", label: "Ingest" },
  { value: "stage", label: "Between stages" },
  { value: "egress", label: "Egress" },
];

function NewFamilyForm() {
  const router = useRouter();
  const { data: contracts, isLoading } = useSWR<ContractSummary[]>("contracts", listContracts);
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [contractId, setContractId] = useState("");
  const [boundary, setBoundary] = useState("ingest");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    setSaving(true);
    setError(null);
    try {
      const family = await createContractFamily({
        name: name.trim(),
        ...(description.trim() ? { description: description.trim() } : {}),
        ...(contractId
          ? { links: [{ contract_id: contractId, boundary }] }
          : {}),
      });
      router.push(`/contracts/families/${family.id}`);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
      setSaving(false);
    }
  }

  return (
    <div className="max-w-xl">
      <Link href="/contracts/families" className="text-xs text-slate-500 hover:text-green-400">
        ← Families
      </Link>
      <h1 className="text-2xl font-bold text-slate-100 mt-2 mb-1">New family</h1>
      <p className="text-sm text-slate-500 mb-6">
        Start with a name. The first contract is the root. Add the next link from the family page.
      </p>

      <form onSubmit={handleSubmit} className="space-y-4 bg-[#111827] border border-[#1f2937] rounded-xl p-6">
        <label className="block">
          <span className="text-xs uppercase tracking-wider text-slate-500">Name</span>
          <input
            required
            value={name}
            onChange={(e) => setName(e.target.value)}
            className="mt-1 w-full bg-[#0a0d12] border border-[#1f2937] rounded-lg px-3 py-2 text-sm text-slate-100 outline-none focus:border-green-700"
          />
        </label>
        <label className="block">
          <span className="text-xs uppercase tracking-wider text-slate-500">Description</span>
          <textarea
            value={description}
            onChange={(e) => setDescription(e.target.value)}
            rows={3}
            className="mt-1 w-full bg-[#0a0d12] border border-[#1f2937] rounded-lg px-3 py-2 text-sm text-slate-100 outline-none focus:border-green-700 resize-y"
          />
        </label>
        <label className="block">
          <span className="text-xs uppercase tracking-wider text-slate-500">Root contract</span>
          <select
            value={contractId}
            onChange={(e) => setContractId(e.target.value)}
            className="mt-1 w-full bg-[#0a0d12] border border-[#1f2937] rounded-lg px-3 py-2 text-sm text-slate-100 outline-none focus:border-green-700"
          >
            <option value="">{isLoading ? "Loading contracts…" : "None yet — add it later"}</option>
            {contracts?.map((contract) => (
              <option key={contract.id} value={contract.id}>
                {contract.name}
              </option>
            ))}
          </select>
        </label>
        <label className="block">
          <span className="text-xs uppercase tracking-wider text-slate-500">Boundary</span>
          <select
            value={boundary}
            onChange={(e) => setBoundary(e.target.value)}
            disabled={!contractId}
            className="mt-1 w-full bg-[#0a0d12] border border-[#1f2937] rounded-lg px-3 py-2 text-sm text-slate-100 outline-none focus:border-green-700 disabled:opacity-50"
          >
            {BOUNDARIES.map((option) => (
              <option key={option.value} value={option.value}>
                {option.label}
              </option>
            ))}
          </select>
        </label>
        {error ? (
          <p className="text-sm text-red-400 bg-red-900/20 border border-red-800/40 rounded p-3">{error}</p>
        ) : null}
        <button
          type="submit"
          disabled={saving || !name.trim()}
          className="px-4 py-2 bg-green-600 hover:bg-green-500 disabled:opacity-50 text-white text-sm font-medium rounded-lg transition-colors"
        >
          {saving ? "Creating…" : "Create family"}
        </button>
      </form>
    </div>
  );
}

export default function NewFamilyPage() {
  return (
    <AuthGate page="contracts">
      <NewFamilyForm />
    </AuthGate>
  );
}
