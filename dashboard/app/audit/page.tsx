"use client";

import { useState } from "react";
import useSWR from "swr";
import { getAuditLog, listContracts } from "@/lib/api";
import type { AuditEntry, ContractSummary } from "@/lib/api";
import clsx from "clsx";

export default function AuditPage() {
  const [contractFilter, setContractFilter] = useState<string>("");
  const [page, setPage] = useState(0);
  const PAGE_SIZE = 50;

  const { data: contracts } = useSWR<ContractSummary[]>("contracts", listContracts);
  const { data: entries, isLoading } = useSWR<AuditEntry[]>(
    ["audit", contractFilter, page],
    () =>
      getAuditLog({
        contract_id: contractFilter || undefined,
        limit: PAGE_SIZE,
        offset: page * PAGE_SIZE,
      }),
    { refreshInterval: 10_000 }
  );

  return (
    <div>
      <div className="flex items-center justify-between mb-8">
        <div>
          <h1 className="text-2xl font-bold">Audit Log</h1>
          <p className="text-sm text-slate-500 mt-1">
            Full history of every ingestion event and its validation outcome
          </p>
        </div>
      </div>

      {/* Filters */}
      <div className="flex gap-4 mb-6">
        <select
          value={contractFilter}
          onChange={(e) => { setContractFilter(e.target.value); setPage(0); }}
          className="bg-[#111827] border border-[#1f2937] text-slate-300 text-sm rounded-lg px-3 py-2 outline-none focus:border-green-700"
        >
          <option value="">All contracts</option>
          {contracts?.map((c) => (
            <option key={c.id} value={c.id}>
              {c.name} (v{c.version})
            </option>
          ))}
        </select>
      </div>

      {/* Table */}
      <div className="bg-[#111827] border border-[#1f2937] rounded-xl overflow-hidden">
        <table className="w-full text-sm">
          <thead>
            <tr className="text-xs text-slate-500 border-b border-[#1f2937] bg-[#0d1117]">
              <th className="px-4 py-3 text-left font-medium">Time</th>
              <th className="px-4 py-3 text-left font-medium">Contract ID</th>
              <th className="px-4 py-3 text-left font-medium">Status</th>
              <th className="px-4 py-3 text-left font-medium">Violations</th>
              <th className="px-4 py-3 text-left font-medium">Latency</th>
              <th className="px-4 py-3 text-left font-medium">Source IP</th>
            </tr>
          </thead>
          <tbody>
            {isLoading ? (
              <tr>
                <td colSpan={6} className="px-4 py-8 text-center text-slate-600">
                  Loading…
                </td>
              </tr>
            ) : entries && entries.length > 0 ? (
              entries.map((e) => (
                <tr
                  key={e.id}
                  className="border-b border-[#1f2937]/50 hover:bg-[#1f2937]/20 cursor-pointer"
                >
                  <td className="px-4 py-3 text-slate-400 font-mono text-xs">
                    {new Date(e.created_at).toLocaleString()}
                  </td>
                  <td className="px-4 py-3 text-slate-500 font-mono text-xs">
                    {e.contract_id.slice(0, 12)}…
                  </td>
                  <td className="px-4 py-3">
                    <span
                      className={clsx(
                        "text-xs px-2 py-0.5 rounded-full font-medium",
                        e.passed
                          ? "bg-green-900/40 text-green-400"
                          : "bg-red-900/40 text-red-400"
                      )}
                    >
                      {e.passed ? "PASS" : "FAIL"}
                    </span>
                  </td>
                  <td className="px-4 py-3 text-slate-400">
                    {e.violation_count > 0 ? (
                      <details className="cursor-pointer">
                        <summary className="text-red-400">
                          {e.violation_count} violation{e.violation_count > 1 ? "s" : ""}
                        </summary>
                        <ul className="mt-1 text-xs text-slate-500 space-y-0.5">
                          {e.violation_details?.map((v, i) => (
                            <li key={i} className="font-mono">
                              [{v.kind}] {v.field}: {v.message}
                            </li>
                          ))}
                        </ul>
                      </details>
                    ) : (
                      <span className="text-slate-600">—</span>
                    )}
                  </td>
                  <td className="px-4 py-3 text-slate-400 font-mono text-xs">
                    {e.validation_us}µs
                  </td>
                  <td className="px-4 py-3 text-slate-500 text-xs">
                    {e.source_ip ?? "—"}
                  </td>
                </tr>
              ))
            ) : (
              <tr>
                <td colSpan={6} className="px-4 py-16 text-center text-slate-600">
                  No audit entries yet
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>

      {/* Pagination */}
      <div className="flex justify-between items-center mt-4">
        <button
          onClick={() => setPage((p) => Math.max(0, p - 1))}
          disabled={page === 0}
          className="text-sm px-3 py-1.5 bg-[#1f2937] rounded-lg disabled:opacity-40 text-slate-300 hover:bg-[#374151]"
        >
          ← Previous
        </button>
        <span className="text-xs text-slate-500">Page {page + 1}</span>
        <button
          onClick={() => setPage((p) => p + 1)}
          disabled={!entries || entries.length < PAGE_SIZE}
          className="text-sm px-3 py-1.5 bg-[#1f2937] rounded-lg disabled:opacity-40 text-slate-300 hover:bg-[#374151]"
        >
          Next →
        </button>
      </div>
    </div>
  );
}
