"use client";

import { Suspense, useState, useEffect, useCallback } from "react";
import { useSearchParams } from "next/navigation";
import useSWR from "swr";
import { getAuditLog, listContracts } from "@/lib/api";
import type { AuditEntry, ContractSummary } from "@/lib/api";
import clsx from "clsx";

// ---------------------------------------------------------------------------
// CSV export helper
// ---------------------------------------------------------------------------

function exportToCSV(entries: AuditEntry[]) {
  const headers = [
    "Time",
    "Contract ID",
    "Status",
    "Violations",
    "Violation Details",
    "Latency (µs)",
    "Source IP",
  ];
  const rows = entries.map((e) => [
    new Date(e.created_at).toLocaleString(),
    e.contract_id,
    e.passed ? "PASS" : "FAIL",
    e.violation_count,
    e.violation_details?.map((v) => `[${v.kind}] ${v.field}: ${v.message}`).join("; ") ?? "",
    e.validation_us,
    e.source_ip ?? "",
  ]);

  const csvContent = [headers, ...rows]
    .map((row) =>
      row.map((cell) => `"${String(cell).replace(/"/g, '""')}"`).join(",")
    )
    .join("\n");

  const blob = new Blob([csvContent], { type: "text/csv;charset=utf-8;" });
  const url = URL.createObjectURL(blob);
  const link = document.createElement("a");
  link.href = url;
  link.download = `contractgate-audit-${new Date().toISOString().slice(0, 10)}.csv`;
  document.body.appendChild(link);
  link.click();
  document.body.removeChild(link);
  URL.revokeObjectURL(url);
}

// ---------------------------------------------------------------------------
// Inner page content (needs Suspense for useSearchParams in Next.js 15)
// ---------------------------------------------------------------------------

function AuditContent() {
  const searchParams = useSearchParams();
  const [contractFilter, setContractFilter] = useState<string>(
    searchParams.get("contract_id") ?? ""
  );
  const [statusFilter, setStatusFilter] = useState<"all" | "pass" | "fail">("all");
  const [page, setPage] = useState(0);
  const PAGE_SIZE = 50;

  // If a contract_id lands via URL (e.g. quick-link from the dashboard), apply it
  useEffect(() => {
    const id = searchParams.get("contract_id");
    if (id) {
      setContractFilter(id);
      setPage(0);
    }
  }, [searchParams]);

  const { data: contracts } = useSWR<ContractSummary[]>("contracts", listContracts);
  const { data: allEntries, isLoading } = useSWR<AuditEntry[]>(
    ["audit", contractFilter, page],
    () =>
      getAuditLog({
        contract_id: contractFilter || undefined,
        limit: PAGE_SIZE,
        offset: page * PAGE_SIZE,
      }),
    { refreshInterval: 10_000 }
  );

  // Client-side status filter applied on top of the current page
  const entries = allEntries?.filter((e) => {
    if (statusFilter === "pass") return e.passed;
    if (statusFilter === "fail") return !e.passed;
    return true;
  });

  const handleExport = useCallback(() => {
    if (entries && entries.length > 0) exportToCSV(entries);
  }, [entries]);

  return (
    <div>
      <div className="flex items-center justify-between mb-8">
        <div>
          <h1 className="text-2xl font-bold">Audit Log</h1>
          <p className="text-sm text-slate-500 mt-1">
            Full history of every ingestion event and its validation outcome
          </p>
        </div>
        <button
          onClick={handleExport}
          disabled={!entries || entries.length === 0}
          className="flex items-center gap-2 text-sm px-4 py-2 bg-[#1f2937] border border-[#374151] rounded-lg text-slate-300 hover:bg-[#374151] hover:text-white disabled:opacity-40 disabled:cursor-not-allowed transition-colors"
          title="Export current view as CSV"
        >
          <span>↓</span>
          Export CSV
        </button>
      </div>

      {/* Filters */}
      <div className="flex flex-wrap gap-3 mb-6">
        {/* Contract filter */}
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

        {/* Status filter */}
        <div className="flex rounded-lg overflow-hidden border border-[#1f2937] text-sm">
          {(["all", "pass", "fail"] as const).map((s) => (
            <button
              key={s}
              onClick={() => { setStatusFilter(s); setPage(0); }}
              className={clsx(
                "px-4 py-2 transition-colors",
                statusFilter === s
                  ? s === "pass"
                    ? "bg-green-900/40 text-green-400"
                    : s === "fail"
                    ? "bg-red-900/40 text-red-400"
                    : "bg-[#1f2937] text-slate-200"
                  : "bg-[#111827] text-slate-500 hover:text-slate-300"
              )}
            >
              {s === "all" ? "All" : s === "pass" ? "✓ Pass" : "✗ Fail"}
            </button>
          ))}
        </div>

        {/* Clear filters button */}
        {(contractFilter || statusFilter !== "all") && (
          <button
            onClick={() => { setContractFilter(""); setStatusFilter("all"); setPage(0); }}
            className="text-xs text-slate-500 hover:text-slate-300 px-2 py-1 rounded border border-[#374151] transition-colors"
          >
            Clear filters ×
          </button>
        )}
      </div>

      {/* Table */}
      <div className="bg-[#111827] border border-[#1f2937] rounded-xl overflow-hidden">
        <table className="w-full text-sm">
          <thead>
            <tr className="text-xs text-slate-500 border-b border-[#1f2937] bg-[#0d1117]">
              <th className="px-4 py-3 text-left font-medium">Time</th>
              <th className="px-4 py-3 text-left font-medium">Contract</th>
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
                  className="border-b border-[#1f2937]/50 hover:bg-[#1f2937]/20"
                >
                  <td className="px-4 py-3 text-slate-400 font-mono text-xs">
                    {new Date(e.created_at).toLocaleString()}
                  </td>
                  <td className="px-4 py-3">
                    <button
                      onClick={() => { setContractFilter(e.contract_id); setPage(0); }}
                      className="text-slate-500 font-mono text-xs hover:text-green-400 transition-colors"
                      title="Filter by this contract"
                    >
                      {e.contract_id.slice(0, 12)}…
                    </button>
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
                  {statusFilter !== "all"
                    ? `No ${statusFilter === "pass" ? "passing" : "failing"} entries on this page`
                    : "No audit entries yet"}
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>

      {/* Pagination + result count */}
      <div className="flex justify-between items-center mt-4">
        <button
          onClick={() => setPage((p) => Math.max(0, p - 1))}
          disabled={page === 0}
          className="text-sm px-3 py-1.5 bg-[#1f2937] rounded-lg disabled:opacity-40 text-slate-300 hover:bg-[#374151]"
        >
          ← Previous
        </button>
        <span className="text-xs text-slate-500">
          Page {page + 1}
          {entries && allEntries && statusFilter !== "all" && (
            <span className="ml-2 text-slate-600">
              ({entries.length} of {allEntries.length} shown after filter)
            </span>
          )}
        </span>
        <button
          onClick={() => setPage((p) => p + 1)}
          disabled={!allEntries || allEntries.length < PAGE_SIZE}
          className="text-sm px-3 py-1.5 bg-[#1f2937] rounded-lg disabled:opacity-40 text-slate-300 hover:bg-[#374151]"
        >
          Next →
        </button>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Page — wraps content in Suspense (required by Next.js 15 for useSearchParams)
// ---------------------------------------------------------------------------

export default function AuditPage() {
  return (
    <Suspense fallback={<div className="text-slate-500 text-sm p-8">Loading audit log…</div>}>
      <AuditContent />
    </Suspense>
  );
}
