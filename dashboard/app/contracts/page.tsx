"use client";

import { useState } from "react";
import useSWR, { mutate } from "swr";
import {
  listContracts,
  createContract,
  updateContract,
  deleteContract,
} from "@/lib/api";
import type { ContractSummary } from "@/lib/api";
import clsx from "clsx";

const EXAMPLE_YAML = `version: "1.0"
name: "my_events"
description: "Replace this with your contract"

ontology:
  entities:
    - name: user_id
      type: string
      required: true
      pattern: "^[a-zA-Z0-9_-]{3,64}$"

    - name: event_type
      type: string
      required: true
      enum:
        - "click"
        - "view"
        - "purchase"

    - name: timestamp
      type: integer
      required: true
      min: 0

    - name: amount
      type: number
      required: false
      min: 0

glossary:
  - field: "user_id"
    description: "Unique user identifier"
  - field: "amount"
    description: "Monetary value in USD"
    constraints: "must be non-negative"

metrics:
  - name: "total_revenue"
    formula: "sum(amount) where event_type = 'purchase'"
`;

export default function ContractsPage() {
  const { data: contracts, isLoading } = useSWR<ContractSummary[]>(
    "contracts",
    listContracts
  );
  const [showCreate, setShowCreate] = useState(false);
  const [yaml, setYaml] = useState(EXAMPLE_YAML);
  const [creating, setCreating] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const handleCreate = async () => {
    setCreating(true);
    setError(null);
    try {
      await createContract(yaml);
      await mutate("contracts");
      setShowCreate(false);
      setYaml(EXAMPLE_YAML);
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setCreating(false);
    }
  };

  const handleToggleActive = async (c: ContractSummary) => {
    await updateContract(c.id, { active: !c.active });
    await mutate("contracts");
  };

  const handleDelete = async (id: string) => {
    if (!confirm("Delete this contract? This cannot be undone.")) return;
    await deleteContract(id);
    await mutate("contracts");
  };

  return (
    <div>
      <div className="flex items-center justify-between mb-8">
        <div>
          <h1 className="text-2xl font-bold">Contracts</h1>
          <p className="text-sm text-slate-500 mt-1">
            Create and manage versioned semantic contracts
          </p>
        </div>
        <button
          onClick={() => setShowCreate(true)}
          className="px-4 py-2 bg-green-600 hover:bg-green-500 text-white text-sm font-medium rounded-lg transition-colors"
        >
          + New Contract
        </button>
      </div>

      {/* Create contract panel */}
      {showCreate && (
        <div className="mb-8 bg-[#111827] border border-[#1f2937] rounded-xl p-6">
          <h2 className="text-base font-semibold mb-4">New Contract (YAML)</h2>
          <textarea
            className="w-full h-80 bg-[#0a0d12] text-green-300 font-mono text-sm p-4 rounded-lg border border-[#1f2937] outline-none focus:border-green-700 resize-y"
            value={yaml}
            onChange={(e) => setYaml(e.target.value)}
            spellCheck={false}
          />
          {error && (
            <p className="mt-2 text-sm text-red-400 bg-red-900/20 border border-red-800/40 rounded p-2">
              {error}
            </p>
          )}
          <div className="flex gap-3 mt-4">
            <button
              onClick={handleCreate}
              disabled={creating}
              className="px-4 py-2 bg-green-600 hover:bg-green-500 disabled:opacity-50 text-white text-sm font-medium rounded-lg transition-colors"
            >
              {creating ? "Creating…" : "Create Contract"}
            </button>
            <button
              onClick={() => setShowCreate(false)}
              className="px-4 py-2 bg-[#1f2937] hover:bg-[#374151] text-slate-300 text-sm font-medium rounded-lg transition-colors"
            >
              Cancel
            </button>
          </div>
        </div>
      )}

      {/* Contract list */}
      {isLoading ? (
        <p className="text-slate-500 text-sm">Loading…</p>
      ) : contracts && contracts.length > 0 ? (
        <div className="space-y-3">
          {contracts.map((c) => (
            <div
              key={c.id}
              className="bg-[#111827] border border-[#1f2937] rounded-xl p-5 flex items-center justify-between"
            >
              <div>
                <div className="flex items-center gap-3">
                  <h3 className="font-semibold text-slate-200">{c.name}</h3>
                  <span className="text-xs text-slate-500">v{c.version}</span>
                  <span
                    className={clsx(
                      "text-xs px-2 py-0.5 rounded-full font-medium",
                      c.active
                        ? "bg-green-900/40 text-green-400"
                        : "bg-slate-800 text-slate-500"
                    )}
                  >
                    {c.active ? "Active" : "Inactive"}
                  </span>
                </div>
                <p className="text-xs text-slate-600 mt-1 font-mono">{c.id}</p>
              </div>
              <div className="flex items-center gap-2">
                <button
                  onClick={() => handleToggleActive(c)}
                  className="px-3 py-1.5 text-xs bg-[#1f2937] hover:bg-[#374151] text-slate-300 rounded-lg transition-colors"
                >
                  {c.active ? "Deactivate" : "Activate"}
                </button>
                <button
                  onClick={() => handleDelete(c.id)}
                  className="px-3 py-1.5 text-xs bg-red-900/30 hover:bg-red-900/50 text-red-400 rounded-lg transition-colors"
                >
                  Delete
                </button>
              </div>
            </div>
          ))}
        </div>
      ) : (
        <div className="flex flex-col items-center justify-center h-64 text-slate-600">
          <p className="text-4xl mb-4">📋</p>
          <p className="text-sm">No contracts yet — create your first one above.</p>
        </div>
      )}
    </div>
  );
}
