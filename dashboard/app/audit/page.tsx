"use client";

import { Suspense, useState, useEffect, useCallback } from "react";
import { useSearchParams } from "next/navigation";
import useSWR from "swr";
import { getAuditLog, listContracts } from "@/lib/api";
import type { AuditEntry, ContractSummary } from "@/lib/api";
import clsx from "clsx";
import AuthGate from "@/components/AuthGate";

// ---------------------------------------------------------------------------
// Raw-event drawer (RFC-004 surfacing)
// ---------------------------------------------------------------------------

/**
 * Side-panel drawer that shows the stored `raw_event` for a single audit
 * row.  The framing matters: under RFC-004 §6, the value in this column
 * has ALREADY been through the transform engine (masks / hashes / drops /
 * redactions applied) — it is the exact bytes the server wrote to
 * `audit_log.raw_event`.  We label the panel "Stored payload" rather than
 * "raw event" so nobody mistakes this for the request body.
 */
function RawEventDrawer({
  entry,
  onClose,
}: {
  entry: AuditEntry | null;
  onClose: () => void;
}) {
  const [copied, setCopied] = useState(false);

  // Escape closes the drawer — standard modal affordance.
  useEffect(() => {
    if (!entry) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [entry, onClose]);

  // Reset the "Copied!" flash when switching rows.
  useEffect(() => {
    setCopied(false);
  }, [entry?.id]);

  if (!entry) return null;

  const prettyJson = JSON.stringify(entry.raw_event, null, 2);

  const handleCopy = async () => {
    try {
      await navigator.clipboard.writeText(prettyJson);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch {
      // clipboard permission can be blocked — fall back silently.
    }
  };

  return (
    <>
      {/* Backdrop */}
      <button
        onClick={onClose}
        className="fixed inset-0 bg-black/50 z-40"
        aria-label="Close audit entry"
      />
      {/* Drawer panel */}
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby="raw-event-drawer-title"
        className="fixed top-0 right-0 h-full w-full md:w-[560px] bg-[#0d1117] border-l border-[#1f2937] z-50 shadow-2xl flex flex-col"
      >
        {/* Header */}
        <div className="flex items-start justify-between px-6 py-5 border-b border-[#1f2937]">
          <div className="min-w-0">
            <h2
              id="raw-event-drawer-title"
              className="text-sm font-semibold text-slate-200 uppercase tracking-wider"
            >
              Audit Entry
            </h2>
            <p className="text-xs text-slate-500 font-mono mt-1 truncate" title={entry.id}>
              {entry.id}
            </p>
          </div>
          <button
            onClick={onClose}
            className="text-slate-500 hover:text-slate-200 text-xl leading-none ml-4"
            aria-label="Close"
          >
            ✕
          </button>
        </div>

        {/* Meta chips */}
        <div className="px-6 py-4 border-b border-[#1f2937] space-y-3">
          <div className="flex flex-wrap items-center gap-2">
            <span
              className={clsx(
                "text-xs px-2 py-0.5 rounded-full font-medium",
                entry.passed
                  ? "bg-green-900/40 text-green-400"
                  : "bg-red-900/40 text-red-400"
              )}
            >
              {entry.passed ? "PASS" : "FAIL"}
            </span>
            {entry.contract_version && (
              <span
                className="text-xs bg-indigo-900/40 text-indigo-300 border border-indigo-700/40 rounded-full px-2 py-0.5 font-mono"
                title="Contract version that produced this decision (RFC-002)"
              >
                v{entry.contract_version}
              </span>
            )}
            <span className="text-xs text-slate-500 font-mono">
              {entry.validation_us}µs
            </span>
            {entry.source_ip && (
              <span className="text-xs text-slate-500 font-mono">
                {entry.source_ip}
              </span>
            )}
          </div>
          <dl className="grid grid-cols-[auto,1fr] gap-x-3 gap-y-1 text-xs">
            <dt className="text-slate-600 uppercase tracking-wider">Contract</dt>
            <dd className="text-slate-400 font-mono truncate">{entry.contract_id}</dd>
            <dt className="text-slate-600 uppercase tracking-wider">Recorded</dt>
            <dd className="text-slate-400">
              {new Date(entry.created_at).toLocaleString()}
            </dd>
          </dl>
        </div>

        {/* Violations (if any) */}
        {entry.violation_count > 0 && (
          <div className="px-6 py-4 border-b border-[#1f2937]">
            <p className="text-xs text-slate-500 uppercase tracking-wider mb-2">
              Violations · {entry.violation_count}
            </p>
            <ul className="space-y-1.5">
              {entry.violation_details?.map((v, i) => (
                <li
                  key={i}
                  className="bg-red-900/20 border border-red-800/30 rounded-lg p-2.5 text-xs"
                >
                  <div className="flex items-center gap-2 mb-0.5">
                    <span className="bg-red-900/50 text-red-400 px-1.5 py-0.5 rounded font-mono text-[10px]">
                      {v.kind}
                    </span>
                    <span className="text-slate-400 font-mono">{v.field}</span>
                  </div>
                  <p className="text-slate-300">{v.message}</p>
                </li>
              ))}
            </ul>
          </div>
        )}

        {/* Stored payload — scrolls independently */}
        <div className="flex-1 overflow-y-auto px-6 py-4">
          <div className="flex items-center justify-between mb-2">
            <p className="text-xs text-slate-500 uppercase tracking-wider">
              Stored payload
            </p>
            <button
              onClick={handleCopy}
              className="text-xs px-2.5 py-1 bg-[#1f2937] hover:bg-[#374151] border border-[#374151] rounded text-slate-300 transition-colors"
            >
              {copied ? "✓ Copied" : "Copy JSON"}
            </button>
          </div>
          <div className="mb-3 flex items-start gap-2 bg-indigo-900/15 border border-indigo-700/30 rounded-lg px-3 py-2">
            <span className="text-indigo-400 text-sm leading-5">🔒</span>
            <p className="text-[11px] text-indigo-200 leading-relaxed">
              Under RFC-004 §6, values here have already been through the
              contract&apos;s PII transforms before being written to{" "}
              <code className="text-indigo-300">audit_log.raw_event</code>. Raw
              PII never reaches this column — fields with a declared{" "}
              <code className="text-indigo-300">mask</code>,{" "}
              <code className="text-indigo-300">hash</code>,{" "}
              <code className="text-indigo-300">drop</code>, or{" "}
              <code className="text-indigo-300">redact</code> transform are
              already scrubbed.
            </p>
          </div>
          <pre className="text-xs text-green-300 font-mono bg-[#0a0d12] border border-[#1f2937] rounded-lg p-3 whitespace-pre-wrap break-all">
            {prettyJson}
          </pre>
        </div>
      </div>
    </>
  );
}

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
  const [selected, setSelected] = useState<AuditEntry | null>(null);
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
              {c.name}
              {c.latest_stable_version
                ? ` (stable v${c.latest_stable_version})`
                : ""}
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
                  onClick={() => setSelected(e)}
                  className="border-b border-[#1f2937]/50 hover:bg-[#1f2937]/30 cursor-pointer"
                  title="Open stored payload"
                >
                  <td className="px-4 py-3 text-slate-400 font-mono text-xs">
                    {new Date(e.created_at).toLocaleString()}
                  </td>
                  <td className="px-4 py-3">
                    <button
                      onClick={(ev) => {
                        ev.stopPropagation();
                        setContractFilter(e.contract_id);
                        setPage(0);
                      }}
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
                      <details
                        className="cursor-pointer"
                        onClick={(ev) => ev.stopPropagation()}
                      >
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

      {/* Drawer — portaled via fixed positioning, only mounts when a row is selected */}
      <RawEventDrawer entry={selected} onClose={() => setSelected(null)} />
    </div>
  );
}

// ---------------------------------------------------------------------------
// Page — wraps content in Suspense (required by Next.js 15 for useSearchParams)
// ---------------------------------------------------------------------------

export default function AuditPage() {
  return (
    <AuthGate page="audit">
      <Suspense fallback={<div className="text-slate-500 text-sm p-8">Loading audit log…</div>}>
        <AuditContent />
      </Suspense>
    </AuthGate>
  );
}
