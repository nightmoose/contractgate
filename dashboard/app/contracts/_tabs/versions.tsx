"use client";

/**
 * Versions tab body for EditContractModal.
 * RFC-020: visual state ladder, proper confirm modals, Compare + diff drawer,
 * latest-stable resolver badge, name-history de-emphasis.
 */

import { useState, useCallback, useEffect } from "react";
import clsx from "clsx";
import { getVersion, exportOdcs, approveImport, getConformanceReport } from "@/lib/api";
import type { ContractResponse, VersionSummary, VersionResponse, NameHistoryEntry, ConformanceReport } from "@/lib/api";
import { ConfirmActionModal, TooltipWrap } from "../_lib";

// ---------------------------------------------------------------------------
// Diff drawer types (mirrors src/infer_diff.rs DiffResponse)
// ---------------------------------------------------------------------------

interface DiffChange {
  kind: string;
  field: string;
  detail: string;
}

interface DiffResponse {
  summary: string;
  changes: DiffChange[];
}

// ---------------------------------------------------------------------------
// VersionsTab
// ---------------------------------------------------------------------------

interface VersionsTabProps {
  contractId: string;
  contract: ContractResponse | null;
  versions: VersionSummary[];
  currentVersion: VersionResponse | null;
  saving: boolean;
  error: string | null;
  setError: (e: string | null) => void;
  nameHistory: NameHistoryEntry[] | null;
  loadingNameHistory: boolean;
  onPromoteVersion: (version: string) => Promise<void>;
  onDeprecateVersion: (version: string) => Promise<void>;
  onViewYaml: (version: string) => void;
}

export function VersionsTab({
  contractId,
  contract,
  versions,
  //currentVersion,
  saving,
  error,
  //setError,
  nameHistory,
  loadingNameHistory,
  onPromoteVersion,
  onDeprecateVersion,
  onViewYaml,
}: VersionsTabProps) {
  // Confirm modal state
  const [confirmModal, setConfirmModal] = useState<{
    title: string;
    body: string;
    confirmLabel: string;
    destructive: boolean;
    onConfirm: () => void;
  } | null>(null);

  // Compare: at most 2 versions selected (ordered pair)
  const [compareSet, setCompareSet] = useState<string[]>([]);
  const [diffDrawerOpen, setDiffDrawerOpen] = useState(false);
  const [diffLoading, setDiffLoading] = useState(false);
  const [diffResult, setDiffResult] = useState<DiffResponse | null>(null);
  const [diffError, setDiffError] = useState<string | null>(null);

  const BASE = process.env.NEXT_PUBLIC_API_URL ?? "http://localhost:3001";

  // ODCS export loading map: version → loading state
  const [exportingVersion, setExportingVersion] = useState<string | null>(null);
  // Approve-import loading map: version → loading state
  const [approvingVersion, setApprovingVersion] = useState<string | null>(null);
  // Conformance scores: version → report (lazy-loaded on hover/click)
  const [conformanceMap, setConformanceMap] = useState<Record<string, ConformanceReport | "loading" | "error">>({});

  const handleExportOdcs = useCallback(async (version: string) => {
    setExportingVersion(version);
    try {
      const yaml = await exportOdcs(contractId, version);
      // Trigger browser download
      const blob = new Blob([yaml], { type: "text/yaml" });
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = `${contractId}_${version}_odcs.yaml`;
      a.click();
      URL.revokeObjectURL(url);
    } catch (e: unknown) {
      alert(e instanceof Error ? e.message : String(e));
    } finally {
      setExportingVersion(null);
    }
  }, [contractId]);

  const handleApproveImport = useCallback(async (version: string) => {
    setApprovingVersion(version);
    try {
      await approveImport(contractId, version);
      // Reload versions list via parent re-fetch
      window.dispatchEvent(new CustomEvent("contractgate:versions-changed", { detail: { contractId } }));
    } catch (e: unknown) {
      alert(e instanceof Error ? e.message : String(e));
    } finally {
      setApprovingVersion(null);
    }
  }, [contractId]);

  const loadConformance = useCallback(async (version: string) => {
    if (conformanceMap[version]) return; // already loaded or loading
    setConformanceMap((m) => ({ ...m, [version]: "loading" }));
    try {
      const report = await getConformanceReport(contractId, version);
      setConformanceMap((m) => ({ ...m, [version]: report }));
    } catch {
      setConformanceMap((m) => ({ ...m, [version]: "error" }));
    }
  }, [contractId, conformanceMap]);

  // Sorted list — newest first
  const sortedVersions = [...versions].sort((a, b) =>
    b.created_at.localeCompare(a.created_at)
  );

  // Latest stable for the resolver badge
  const latestStable = contract?.latest_stable_version;

  // ---------------------------------------------------------------------------
  // Helpers
  // ---------------------------------------------------------------------------

  const handlePromoteClick = (version: string) => {
    setConfirmModal({
      title: `Promote v${version} to Stable?`,
      body: "This freezes the YAML forever. Once promoted, the YAML of this version cannot be edited.\n\nPinned clients will continue to use this version. Unpinned traffic will resolve to the newest stable.",
      confirmLabel: "Promote to Stable",
      destructive: false,
      onConfirm: async () => {
        setConfirmModal(null);
        await onPromoteVersion(version);
      },
    });
  };

  const handleDeprecateClick = (version: string) => {
    setConfirmModal({
      title: `Deprecate v${version}?`,
      body: "Pinned traffic will still validate against this version.\n\nNew unpinned traffic routes to the next stable (or fails closed if none remains, depending on policy).\n\nClients that pin a deprecated version will have their entire batch quarantined.",
      confirmLabel: "Deprecate",
      destructive: true,
      onConfirm: async () => {
        setConfirmModal(null);
        await onDeprecateVersion(version);
      },
    });
  };

  const toggleCompare = (version: string) => {
    setCompareSet((prev) => {
      if (prev.includes(version)) return prev.filter((v) => v !== version);
      const next = [...prev, version];
      // Keep only the most recent 2
      return next.length > 2 ? next.slice(next.length - 2) : next;
    });
  };

  const handleCompare = async () => {
    if (compareSet.length !== 2) return;
    setDiffLoading(true);
    setDiffError(null);
    setDiffResult(null);
    setDiffDrawerOpen(true);
    try {
      const [vA, vB] = await Promise.all([
        getVersion(contractId, compareSet[0]),
        getVersion(contractId, compareSet[1]),
      ]);
      const res = await fetch(`${BASE}/contracts/diff`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          contract_yaml_a: vA.yaml_content,
          contract_yaml_b: vB.yaml_content,
        }),
      });
      if (!res.ok) {
        const text = await res.text();
        throw new Error(text || `Diff failed: ${res.status}`);
      }
      const data: DiffResponse = await res.json();
      setDiffResult(data);
    } catch (e: unknown) {
      setDiffError(e instanceof Error ? e.message : String(e));
    } finally {
      setDiffLoading(false);
    }
  };

  // ---------------------------------------------------------------------------
  // Render
  // ---------------------------------------------------------------------------

  return (
    <div className="space-y-8">
      {/* Confirm modal (replaces window.confirm) */}
      {confirmModal && (
        <ConfirmActionModal
          title={confirmModal.title}
          body={confirmModal.body}
          confirmLabel={confirmModal.confirmLabel}
          destructive={confirmModal.destructive}
          onConfirm={confirmModal.onConfirm}
          onCancel={() => setConfirmModal(null)}
        />
      )}

      {/* Diff drawer */}
      {diffDrawerOpen && (
        <DiffDrawer
          versionA={compareSet[0]}
          versionB={compareSet[1]}
          loading={diffLoading}
          result={diffResult}
          error={diffError}
          onClose={() => { setDiffDrawerOpen(false); setDiffResult(null); setDiffError(null); }}
        />
      )}

      {/* Latest-stable resolver badge */}
      <div className="flex items-center gap-3 bg-[#0a0d12] border border-[#1f2937] rounded-xl px-4 py-3">
        <span className="text-xs text-slate-500 uppercase tracking-wider shrink-0">Routing</span>
        {latestStable ? (
          <TooltipWrap
            content={
              contract?.multi_stable_resolution === "fallback"
                ? "Unpinned traffic first tries the latest stable version. On failure it retries other stable versions in order; the first that passes wins."
                : "Unpinned traffic validates against only this latest stable version. On failure the event is quarantined — no retry."
            }
            rfc="RFC-002"
          >
            <span className="text-xs font-mono text-green-400 cursor-default">
              → v{latestStable}
              <span className={clsx(
                "ml-2 px-1.5 py-0.5 rounded text-[10px] uppercase tracking-wider",
                contract?.multi_stable_resolution === "fallback"
                  ? "bg-indigo-900/40 text-indigo-300"
                  : "bg-slate-800 text-slate-400"
              )}>
                {contract?.multi_stable_resolution ?? "strict"}
              </span>
            </span>
          </TooltipWrap>
        ) : (
          <span className="text-xs text-amber-400">
            No stable version — unpinned traffic will receive 409 NoStableVersion
          </span>
        )}
      </div>

      {/* Visual state ladder */}
      <StateLadder />

      {/* Compare toolbar */}
      <div className="flex items-center gap-3">
        <h3 className="text-xs font-semibold text-slate-400 uppercase tracking-wider flex-1">
          All Versions
        </h3>
        {compareSet.length > 0 && (
          <span className="text-xs text-slate-500">
            {compareSet.length}/2 selected for compare
          </span>
        )}
        {compareSet.length === 2 && (
          <button
            onClick={handleCompare}
            disabled={diffLoading}
            className="px-3 py-1.5 text-xs bg-indigo-600 hover:bg-indigo-500 disabled:opacity-50 text-white rounded-lg transition-colors"
          >
            {diffLoading ? "Diffing…" : "Compare selected (2)"}
          </button>
        )}
        {compareSet.length > 0 && (
          <button
            onClick={() => setCompareSet([])}
            className="text-xs text-slate-500 hover:text-slate-300 transition-colors"
          >
            Clear
          </button>
        )}
      </div>

      {/* Versions table */}
      {versions.length === 0 ? (
        <p className="text-xs text-slate-500 italic">No versions yet.</p>
      ) : (
        <div className="space-y-2">
          {sortedVersions.map((v) => {
            const isLatestStable = v.version === latestStable;
            return (
              <div
                key={v.version}
                className="flex items-center justify-between bg-[#0a0d12] border border-[#1f2937] rounded-xl px-4 py-3 gap-4"
              >
                {/* Compare checkbox */}
                <input
                  type="checkbox"
                  checked={compareSet.includes(v.version)}
                  onChange={() => toggleCompare(v.version)}
                  className="rounded border-[#374151] bg-[#111827] accent-indigo-500 cursor-pointer shrink-0"
                  aria-label={`Select v${v.version} for comparison`}
                />

                {/* Version info */}
                <div className="flex items-center gap-3 min-w-0 flex-1">
                  <span className="font-mono text-sm text-slate-200 shrink-0">
                    v{v.version}
                  </span>
                  <TooltipWrap
                    content={
                      v.state === "stable"
                        ? "A frozen, immutable version eligible to receive inbound traffic. YAML cannot be edited after promotion."
                        : v.state === "draft"
                        ? "A work-in-progress version. YAML is freely editable. Promotes to Stable when ready."
                        : "A retired version. No new unpinned traffic routes to it. Clients that explicitly pin this version get their batch quarantined."
                    }
                    rfc="RFC-002"
                  >
                    <span
                      className={clsx(
                        "shrink-0 px-1.5 py-0.5 rounded text-[10px] uppercase tracking-wider font-sans cursor-default",
                        v.state === "stable" && "bg-green-900/40 text-green-400",
                        v.state === "draft" && "bg-amber-900/40 text-amber-400",
                        v.state === "deprecated" && "bg-slate-800 text-slate-500"
                      )}
                    >
                      {v.state}
                    </span>
                  </TooltipWrap>
                  {isLatestStable && (
                    <TooltipWrap
                      content="Unpinned traffic resolves to this version by default (latest stable by promotion timestamp)."
                      rfc="RFC-002"
                    >
                      <span className="text-[10px] px-1.5 py-0.5 rounded bg-green-900/20 text-green-600 border border-green-800/30 cursor-default">
                        default
                      </span>
                    </TooltipWrap>
                  )}
                  {/* ODCS source badge */}
                  {v.import_source && v.import_source !== "native" && (
                    <TooltipWrap
                      content={
                        v.import_source === "odcs"
                          ? "Imported from a full ODCS v3.1.0 document (lossless round-trip)."
                          : "Imported from a foreign ODCS document without ContractGate extensions. Best-effort reconstruction — review required before promotion."
                      }
                      rfc="D-003"
                    >
                      <span className={clsx(
                        "text-[10px] px-1.5 py-0.5 rounded border cursor-default",
                        v.import_source === "odcs"
                          ? "bg-blue-900/30 text-blue-400 border-blue-800/40"
                          : "bg-orange-900/30 text-orange-400 border-orange-800/40"
                      )}>
                        {v.import_source === "odcs" ? "ODCS" : "ODCS ⚠"}
                      </span>
                    </TooltipWrap>
                  )}
                  {/* requires_review warning */}
                  {v.requires_review && (
                    <TooltipWrap
                      content="This version was imported from a foreign ODCS document without ContractGate extensions. A human must review the reconstructed contract before it can be promoted. Click 'Approve' to clear this flag."
                      rfc="D-002"
                    >
                      <span className="text-[10px] px-1.5 py-0.5 rounded bg-orange-900/30 text-orange-300 border border-orange-800/40 cursor-default animate-pulse">
                        review required
                      </span>
                    </TooltipWrap>
                  )}
                  {/* Conformance score chip — loads on mount */}
                  <ConformanceChip
                    report={conformanceMap[v.version]}
                    onLoad={() => loadConformance(v.version)}
                  />
                  <span className="text-xs text-slate-600 truncate">
                    Created {new Date(v.created_at).toLocaleString()}
                    {v.promoted_at &&
                      ` · promoted ${new Date(v.promoted_at).toLocaleString()}`}
                    {v.deprecated_at &&
                      ` · deprecated ${new Date(v.deprecated_at).toLocaleString()}`}
                  </span>
                </div>

                {/* Actions */}
                <div className="flex items-center gap-2 shrink-0">
                  {/* Approve import — only for drafts that need review */}
                  {v.state === "draft" && v.requires_review && (
                    <button
                      onClick={() => handleApproveImport(v.version)}
                      disabled={approvingVersion === v.version}
                      className="px-3 py-1 text-xs bg-orange-700 hover:bg-orange-600 disabled:opacity-40 text-white rounded-lg transition-colors"
                    >
                      {approvingVersion === v.version ? "Approving…" : "Approve ✓"}
                    </button>
                  )}
                  {v.state === "draft" && !v.requires_review && (
                    <button
                      onClick={() => handlePromoteClick(v.version)}
                      disabled={saving}
                      className="px-3 py-1 text-xs bg-indigo-600 hover:bg-indigo-500 disabled:opacity-40 text-white rounded-lg transition-colors"
                      title="Promote this draft to stable"
                    >
                      Promote
                    </button>
                  )}
                  {v.state === "stable" && (
                    <button
                      onClick={() => handleDeprecateClick(v.version)}
                      disabled={saving}
                      className="px-3 py-1 text-xs bg-amber-900/30 hover:bg-amber-900/50 disabled:opacity-40 text-amber-300 rounded-lg transition-colors"
                    >
                      Deprecate
                    </button>
                  )}
                  {/* Export ODCS YAML */}
                  <button
                    onClick={() => handleExportOdcs(v.version)}
                    disabled={exportingVersion === v.version}
                    className="px-3 py-1 text-xs bg-blue-900/30 hover:bg-blue-900/50 disabled:opacity-40 text-blue-300 rounded-lg transition-colors"
                    title="Export as ODCS v3.1.0 YAML"
                  >
                    {exportingVersion === v.version ? "…" : "↓ ODCS"}
                  </button>
                  <button
                    onClick={() => onViewYaml(v.version)}
                    className="px-3 py-1 text-xs bg-[#1f2937] hover:bg-[#374151] text-slate-300 rounded-lg transition-colors"
                  >
                    View YAML →
                  </button>
                </div>
              </div>
            );
          })}
        </div>
      )}

      {/* Name history */}
      <div>
        {loadingNameHistory ? (
          <p className="text-xs text-slate-500 animate-pulse">Loading name history…</p>
        ) : !nameHistory || nameHistory.length === 0 ? (
          <p className="text-xs text-slate-600 italic">
            Contract has always been named &ldquo;{contract?.name ?? "…"}&rdquo;.
          </p>
        ) : (
          <div className="space-y-2">
            <h3 className="text-xs font-semibold text-slate-500 uppercase tracking-wider mb-2">
              Name History
            </h3>
            {[...nameHistory]
              .sort((a, b) => b.changed_at.localeCompare(a.changed_at))
              .map((h) => (
                <div
                  key={h.id}
                  className="flex items-center gap-3 text-xs bg-[#0a0d12] border border-[#1f2937] rounded-lg px-4 py-2.5 opacity-75"
                >
                  <span className="font-mono text-slate-500 line-through">{h.old_name}</span>
                  <span className="text-slate-600">→</span>
                  <span className="font-mono text-slate-300">{h.new_name}</span>
                  <span className="ml-auto text-slate-600">
                    {new Date(h.changed_at).toLocaleString()}
                  </span>
                </div>
              ))}
          </div>
        )}
      </div>

      {error && (
        <p className="text-sm text-red-400 bg-red-900/20 border border-red-800/40 rounded p-2">
          {error}
        </p>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// ConformanceChip — lazy-loaded ODCS conformance score badge per version
// ---------------------------------------------------------------------------

function ConformanceChip({
  report,
  onLoad,
}: {
  report: ConformanceReport | "loading" | "error" | undefined;
  onLoad: () => void;
}) {
  // Trigger load on first render
  // eslint-disable-next-line react-hooks/exhaustive-deps
  useEffect(() => { onLoad(); }, []);

  if (!report || report === "loading") {
    return (
      <span className="text-[10px] px-1.5 py-0.5 rounded bg-[#111827] text-slate-600 border border-[#1f2937] cursor-default animate-pulse">
        score…
      </span>
    );
  }
  if (report === "error") {
    return null; // silently skip — conformance is optional
  }

  const pct = Math.round(report.overall_score * 100);
  const color =
    pct >= 90 ? "bg-green-900/40 text-green-400 border-green-800/40" :
    pct >= 70 ? "bg-blue-900/40 text-blue-400 border-blue-800/40" :
    pct >= 50 ? "bg-amber-900/40 text-amber-400 border-amber-800/40" :
    "bg-red-900/40 text-red-400 border-red-800/40";

  const tooltip = [
    `ODCS v3.1.0 Conformance: ${pct}%`,
    `Mandatory fields: ${Math.round(report.mandatory_fields_score * 100)}%`,
    `Extensions: ${Math.round(report.extensions_score * 100)}%`,
    `Round-trip fidelity: ${Math.round(report.round_trip_fidelity_score * 100)}%`,
    `Quality coverage: ${Math.round(report.quality_coverage_pct * 100)}% (${report.quality_covered_fields}/${report.total_fields} fields)`,
  ].join("\n");

  return (
    <TooltipWrap content={tooltip} rfc="ODCS">
      <span className={clsx(
        "text-[10px] px-1.5 py-0.5 rounded border cursor-default font-mono",
        color
      )}>
        {pct}%
      </span>
    </TooltipWrap>
  );
}

// ---------------------------------------------------------------------------
// StateLadder — visual draft → stable → deprecated diagram
// ---------------------------------------------------------------------------

function StateLadder() {
  return (
    <div className="flex items-center gap-2 text-xs py-2">
      <TooltipWrap content="A work-in-progress version. YAML is freely editable. Promotes to Stable when ready." rfc="RFC-002">
        <span className="px-2.5 py-1 rounded-lg bg-amber-900/30 text-amber-400 border border-amber-800/30 cursor-default">
          Draft
        </span>
      </TooltipWrap>
      <span className="text-slate-600 select-none">──promote──▶</span>
      <TooltipWrap content="A frozen, immutable version eligible to receive inbound traffic. YAML cannot be edited after promotion." rfc="RFC-002">
        <span className="px-2.5 py-1 rounded-lg bg-green-900/30 text-green-400 border border-green-800/30 cursor-default">
          Stable
        </span>
      </TooltipWrap>
      <span className="text-slate-600 select-none">──deprecate──▶</span>
      <TooltipWrap content="A retired version. No new unpinned traffic routes to it. Clients that explicitly pin this version get their batch quarantined." rfc="RFC-002">
        <span className="px-2.5 py-1 rounded-lg bg-slate-800 text-slate-500 border border-slate-700/30 cursor-default">
          Deprecated
        </span>
      </TooltipWrap>
    </div>
  );
}

// ---------------------------------------------------------------------------
// DiffDrawer — right-panel comparison view
// ---------------------------------------------------------------------------

function DiffDrawer({
  versionA,
  versionB,
  loading,
  result,
  error,
  onClose,
}: {
  versionA: string;
  versionB: string;
  loading: boolean;
  result: DiffResponse | null;
  error: string | null;
  onClose: () => void;
}) {
  return (
    <>
      <div className="fixed inset-0 z-40 bg-black/40" onClick={onClose} />
      <div
        role="dialog"
        aria-modal="true"
        aria-label={`Diff v${versionA} vs v${versionB}`}
        className="fixed top-0 right-0 h-full w-full md:w-[600px] bg-[#0d1117] border-l border-[#1f2937] z-50 shadow-2xl flex flex-col"
      >
        {/* Header */}
        <div className="flex items-start justify-between px-6 py-5 border-b border-[#1f2937]">
          <div>
            <h3 className="text-sm font-semibold text-slate-200 uppercase tracking-wider">
              Compare Versions
            </h3>
            <p className="text-xs text-slate-500 font-mono mt-1">
              v{versionA} → v{versionB}
            </p>
          </div>
          <button
            onClick={onClose}
            className="text-slate-500 hover:text-slate-200 text-xl leading-none ml-4"
            aria-label="Close diff"
          >
            ✕
          </button>
        </div>

        {/* Body */}
        <div className="flex-1 overflow-auto px-6 py-5">
          {loading ? (
            <div className="flex items-center justify-center h-48 text-slate-500 text-sm">
              Computing diff…
            </div>
          ) : error ? (
            <p className="text-sm text-red-400 bg-red-900/20 border border-red-800/40 rounded p-3">
              {error}
            </p>
          ) : result ? (
            <div className="space-y-5">
              {/* Summary */}
              <div className="bg-[#111827] border border-[#1f2937] rounded-xl px-4 py-3">
                <p className="text-xs text-slate-500 uppercase tracking-wider mb-1">Summary</p>
                <p className="text-sm text-slate-200">{result.summary}</p>
              </div>

              {/* Changes table */}
              {result.changes.length === 0 ? (
                <p className="text-xs text-slate-500 italic text-center py-8">
                  No structural changes detected.
                </p>
              ) : (
                <div className="rounded-xl border border-[#1f2937] overflow-hidden">
                  <table className="w-full text-xs">
                    <thead>
                      <tr className="border-b border-[#1f2937] bg-[#111827]">
                        <th className="px-4 py-2.5 text-left text-slate-500 uppercase tracking-wider font-medium">Kind</th>
                        <th className="px-4 py-2.5 text-left text-slate-500 uppercase tracking-wider font-medium">Field</th>
                        <th className="px-4 py-2.5 text-left text-slate-500 uppercase tracking-wider font-medium">Detail</th>
                        <th className="px-4 py-2.5 text-left text-slate-500 uppercase tracking-wider font-medium">
                          <TooltipWrap content="Severity scoring will be available with RFC-015 breaking-change taxonomy.">
                            <span className="cursor-default underline decoration-dotted">Severity</span>
                          </TooltipWrap>
                        </th>
                      </tr>
                    </thead>
                    <tbody className="divide-y divide-[#1f2937]">
                      {result.changes.map((c, i) => (
                        <tr
                          key={i}
                          className={clsx(
                            "transition-colors",
                            c.kind.includes("added") && "bg-green-900/10",
                            c.kind.includes("removed") && "bg-red-900/10",
                            !c.kind.includes("added") && !c.kind.includes("removed") && "bg-amber-900/5"
                          )}
                        >
                          <td className="px-4 py-2.5">
                            <span className={clsx(
                              "font-mono px-1.5 py-0.5 rounded text-[10px]",
                              c.kind.includes("added") && "bg-green-900/40 text-green-400",
                              c.kind.includes("removed") && "bg-red-900/40 text-red-400",
                              !c.kind.includes("added") && !c.kind.includes("removed") && "bg-amber-900/30 text-amber-400"
                            )}>
                              {c.kind}
                            </span>
                          </td>
                          <td className="px-4 py-2.5 font-mono text-slate-300">{c.field}</td>
                          <td className="px-4 py-2.5 text-slate-400">{c.detail}</td>
                          <td className="px-4 py-2.5 text-slate-600">—</td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              )}
            </div>
          ) : null}
        </div>
      </div>
    </>
  );
}
