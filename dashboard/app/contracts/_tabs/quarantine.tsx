"use client";

/**
 * Quarantine tab — RFC-020.
 * Adds: kind/time/text filters, status column, payload preview drawer,
 * per-row Replay button, ConfirmReplayModal, RFC-017 empty-state,
 * ReplaySummaryModal, four outcome colors in history drawer.
 */

import { useState, useEffect } from "react";
import useSWR from "swr";
import clsx from "clsx";
import {
  listVersions,
  listQuarantinedEvents,
  replayEvents,
  getReplayHistory,
} from "@/lib/api";
import type {
  ContractSummary,
  VersionSummary,
  QuarantinedEvent,
  ReplayOutcome,
  ReplayResponse,
} from "@/lib/api";
import {
  ConfirmReplayModal,
  ReplaySummaryModal,
  TooltipWrap,
} from "../_lib";

// ---------------------------------------------------------------------------
// Extended QuarantinedEvent with status (RFC-020 D10)
// ---------------------------------------------------------------------------

type QuarantineStatus = "pending" | "reviewed" | "replayed" | "purged";

interface QuarantinedEventEx extends QuarantinedEvent {
  /** RFC-003 lifecycle state. Present in the wire response; TS type extended here per RFC-020 D10. */
  status?: QuarantineStatus;
}

// ---------------------------------------------------------------------------
// Violation kind filter values
// ---------------------------------------------------------------------------

type KindFilter = "all" | "validation" | "parse" | "transform";

const KIND_LABELS: Record<KindFilter, string> = {
  all: "All kinds",
  validation: "validation",
  parse: "parse",
  transform: "transform",
};

// ---------------------------------------------------------------------------
// Time window filter
// ---------------------------------------------------------------------------

type TimeWindow = "1h" | "6h" | "24h" | "7d" | "all";

const TIME_LABELS: Record<TimeWindow, string> = {
  "1h": "Last 1 h",
  "6h": "Last 6 h",
  "24h": "Last 24 h",
  "7d": "Last 7 d",
  all: "All time",
};

function cutoffFromWindow(w: TimeWindow): Date | null {
  if (w === "all") return null;
  const ms = { "1h": 3_600_000, "6h": 21_600_000, "24h": 86_400_000, "7d": 604_800_000 }[w];
  return new Date(Date.now() - ms);
}

// ---------------------------------------------------------------------------
// Outcome helpers (RFC-020 §Design D)
// ---------------------------------------------------------------------------

function replayOutcomeStyle(passed: boolean, outcome?: string): { label: string; icon: string; color: string; border: string } {
  if (outcome === "already_replayed") {
    return { label: "ALREADY REPLAYED", icon: "↩", color: "text-indigo-400", border: "border-indigo-800/30" };
  }
  if (outcome === "purged" || outcome === "skipped") {
    return { label: "SKIPPED", icon: "⊘", color: "text-slate-500", border: "border-slate-700/30" };
  }
  if (passed) {
    return { label: "PASSED", icon: "✅", color: "text-green-400", border: "border-green-800/30" };
  }
  return { label: "FAILED", icon: "❌", color: "text-red-400", border: "border-red-800/30" };
}

// ---------------------------------------------------------------------------
// QuarantineTab
// ---------------------------------------------------------------------------

export function QuarantineTab({ contracts }: { contracts?: ContractSummary[] }) {
  // Filters
  const [contractFilter, setContractFilter] = useState<string>("");
  const [kindFilter, setKindFilter] = useState<KindFilter>("all");
  const [timeWindow, setTimeWindow] = useState<TimeWindow>("all");
  const [textFilter, setTextFilter] = useState<string>("");

  // Multi-select
  const [selected, setSelected] = useState<Set<string>>(new Set());

  // Replay confirm modal
  const [replayPending, setReplayPending] = useState<{ ids: string[]; version: string } | null>(null);

  // Replay state
  const [replaying, setReplaying] = useState(false);
  const [replayResult, setReplayResult] = useState<ReplayResponse | null>(null);
  const [replayError, setReplayError] = useState<string | null>(null);
  const [showSummary, setShowSummary] = useState(false);

  // Version picker
  const [pickerVersions, setPickerVersions] = useState<VersionSummary[]>([]);
  const [replayVersion, setReplayVersion] = useState<string>("");

  // Payload preview drawer
  const [previewEvent, setPreviewEvent] = useState<QuarantinedEventEx | null>(null);
  const [previewCopied, setPreviewCopied] = useState(false);

  // Replay-history drawer
  const [drawerEventId, setDrawerEventId] = useState<string | null>(null);
  const [drawerHistory, setDrawerHistory] = useState<ReplayOutcome[] | null>(null);
  const [loadingHistory, setLoadingHistory] = useState(false);

  // Fetch versions when contract filter changes
  useEffect(() => {
    if (!contractFilter) {
      setPickerVersions([]);
      setReplayVersion("");
      return;
    }
    let cancelled = false;
    listVersions(contractFilter)
      .then((vs) => {
        if (cancelled) return;
        setPickerVersions(vs);
        const stables = vs.filter((v) => v.state === "stable");
        const def = stables.length > 0
          ? [...stables].sort((a, b) => (b.promoted_at ?? "").localeCompare(a.promoted_at ?? ""))[0].version
          : vs[0]?.version ?? "";
        setReplayVersion(def);
      })
      .catch(() => { if (!cancelled) setPickerVersions([]); });
    return () => { cancelled = true; };
  }, [contractFilter]);

  // Fetch quarantined events
  const swrKey = contractFilter ? `quarantine:${contractFilter}` : "quarantine:all";
  const { data: rawEvents, isLoading, mutate: mutateEvents } = useSWR<QuarantinedEventEx[]>(
    swrKey,
    () => listQuarantinedEvents(contractFilter ? { contract_id: contractFilter, limit: 100 } : { limit: 100 }) as Promise<QuarantinedEventEx[]>,
    { refreshInterval: 30_000 }
  );

  // Apply client-side filters
  const cutoff = cutoffFromWindow(timeWindow);
  const filteredEvents = (rawEvents ?? []).filter((ev) => {
    if (kindFilter !== "all") {
      const hasKind = ev.violation_details?.some((v) => v.kind?.startsWith(kindFilter));
      if (!hasKind) return false;
    }
    if (cutoff && new Date(ev.quarantined_at) < cutoff) return false;
    if (textFilter.trim()) {
      const haystack = JSON.stringify(ev.raw_event ?? {}).toLowerCase();
      if (!haystack.includes(textFilter.trim().toLowerCase())) return false;
    }
    return true;
  });

  // Selection helpers
  const allIds = filteredEvents.map((e) => e.id);
  const allSelected = allIds.length > 0 && allIds.every((id) => selected.has(id));

  const toggleAll = () => setSelected(allSelected ? new Set() : new Set(allIds));
  const toggleOne = (id: string) => {
    const next = new Set(selected);
    if (next.has(id)) { next.delete(id); } else { next.add(id); }
    setSelected(next);
  };

  const someSelected = selected.size > 0;

  // Open replay confirm
  const openReplayConfirm = (ids: string[]) => {
    setReplayPending({ ids, version: replayVersion || "latest-stable" });
  };

  // Execute replay
  const executeReplay = async (ids: string[], version: string) => {
    setReplaying(true);
    setReplayError(null);
    setReplayResult(null);
    try {
      const r = await replayEvents(ids, {
        ...(version && version !== "latest-stable" ? { version } : {}),
        ...(contractFilter ? { contract_id: contractFilter } : {}),
      });
      setReplayResult(r);
      setShowSummary(true);
      await mutateEvents();
      setSelected(new Set());
    } catch (e: unknown) {
      setReplayError(e instanceof Error ? e.message : String(e));
    } finally {
      setReplaying(false);
      setReplayPending(null);
    }
  };

  const handleOpenDrawer = async (eventId: string) => {
    setDrawerEventId(eventId);
    setDrawerHistory(null);
    setLoadingHistory(true);
    try {
      const history = await getReplayHistory({ event_id: eventId, limit: 20 });
      setDrawerHistory(history);
    } catch {
      setDrawerHistory([]);
    } finally {
      setLoadingHistory(false);
    }
  };

  const closeDrawer = () => { setDrawerEventId(null); setDrawerHistory(null); };

  // Batch summary counts (maps current ReplayResponse shape)
  const summaryReplayed = replayResult?.replayed ?? 0;
  const summaryTotal = replayResult?.outcomes?.length ?? 0;
  const summaryFailed = summaryTotal - summaryReplayed;
  const summaryAlreadyReplayed = 0;
  const summarySkipped = 0;

  return (
    <div className="space-y-5">
      {/* Confirm replay modal */}
      {replayPending && (
        <ConfirmReplayModal
          count={replayPending.ids.length}
          version={replayPending.version}
          onConfirm={() => executeReplay(replayPending.ids, replayPending.version)}
          onCancel={() => setReplayPending(null)}
        />
      )}

      {/* Replay summary modal */}
      {showSummary && replayResult && (
        <ReplaySummaryModal
          total={summaryTotal}
          replayed={summaryReplayed}
          stillQuarantined={summaryFailed}
          alreadyReplayed={summaryAlreadyReplayed}
          skipped={summarySkipped}
          targetVersion={replayVersion || "latest-stable"}
          contractId={contractFilter || undefined}
          onClose={() => { setShowSummary(false); setReplayResult(null); }}
        />
      )}

      {/* ── Filter bar ── */}
      <div className="flex flex-wrap items-center gap-3">
        <span className="text-xs text-slate-500 uppercase tracking-wider whitespace-nowrap shrink-0">
          Filter:
        </span>

        {/* Contract */}
        <select
          value={contractFilter}
          onChange={(e) => { setContractFilter(e.target.value); setSelected(new Set()); setReplayResult(null); }}
          className="bg-[#111827] border border-[#1f2937] text-slate-300 text-sm rounded-lg px-3 py-1.5 outline-none focus:border-indigo-600"
        >
          <option value="">All contracts</option>
          {contracts?.map((c) => (
            <option key={c.id} value={c.id}>{c.name}</option>
          ))}
        </select>

        {/* Kind */}
        <select
          value={kindFilter}
          onChange={(e) => setKindFilter(e.target.value as KindFilter)}
          className="bg-[#111827] border border-[#1f2937] text-slate-300 text-sm rounded-lg px-3 py-1.5 outline-none focus:border-indigo-600"
        >
          {(Object.keys(KIND_LABELS) as KindFilter[]).map((k) => (
            <option key={k} value={k}>{KIND_LABELS[k]}</option>
          ))}
        </select>

        {/* Time window */}
        <select
          value={timeWindow}
          onChange={(e) => setTimeWindow(e.target.value as TimeWindow)}
          className="bg-[#111827] border border-[#1f2937] text-slate-300 text-sm rounded-lg px-3 py-1.5 outline-none focus:border-indigo-600"
        >
          {(Object.keys(TIME_LABELS) as TimeWindow[]).map((w) => (
            <option key={w} value={w}>{TIME_LABELS[w]}</option>
          ))}
        </select>

        {/* Free text */}
        <input
          type="search"
          placeholder="Search payload…"
          value={textFilter}
          onChange={(e) => setTextFilter(e.target.value)}
          className="bg-[#111827] border border-[#1f2937] text-slate-300 text-sm rounded-lg px-3 py-1.5 outline-none focus:border-indigo-600 w-48"
        />

        {(contractFilter || kindFilter !== "all" || timeWindow !== "all" || textFilter) && (
          <button
            onClick={() => { setContractFilter(""); setKindFilter("all"); setTimeWindow("all"); setTextFilter(""); setSelected(new Set()); }}
            className="text-xs text-slate-500 hover:text-slate-300 transition-colors"
          >
            ✕ Clear all
          </button>
        )}
      </div>

      {/* ── Replay action bar ── */}
      {someSelected && (
        <div className="flex items-center gap-3 flex-wrap bg-[#111827] border border-indigo-700/40 rounded-xl px-4 py-3">
          <span className="text-sm font-medium text-indigo-300">
            {selected.size} event{selected.size !== 1 ? "s" : ""} selected
          </span>

          {pickerVersions.length > 0 && (
            <div className="flex items-center gap-2">
              <TooltipWrap content="Re-validate the selected events against this contract version. Passes land in the audit log; failures create new quarantine rows.">
                <span className="text-xs text-slate-500 whitespace-nowrap cursor-default underline decoration-dotted">
                  Replay against:
                </span>
              </TooltipWrap>
              <select
                value={replayVersion}
                onChange={(e) => setReplayVersion(e.target.value)}
                className="bg-[#0a0d12] border border-[#1f2937] text-slate-300 text-xs rounded-lg px-2 py-1.5 outline-none focus:border-indigo-600"
              >
                {pickerVersions.map((v) => (
                  <option key={v.version} value={v.version}>
                    v{v.version} · {v.state}
                  </option>
                ))}
              </select>
            </div>
          )}

          <button
            onClick={() => openReplayConfirm(Array.from(selected))}
            disabled={replaying}
            className="px-4 py-1.5 bg-indigo-600 hover:bg-indigo-500 disabled:opacity-50 text-white text-sm font-medium rounded-lg transition-colors"
          >
            {replaying ? "Replaying…" : "▶ Replay"}
          </button>
          <button
            onClick={() => setSelected(new Set())}
            className="px-3 py-1.5 bg-[#1f2937] hover:bg-[#374151] text-slate-400 text-sm rounded-lg transition-colors"
          >
            Deselect all
          </button>
        </div>
      )}

      {replayError && (
        <p className="text-sm text-red-400 bg-red-900/20 border border-red-800/40 rounded p-2">
          {replayError}
        </p>
      )}

      {/* ── Events table ── */}
      {isLoading ? (
        <div className="flex items-center justify-center h-48 text-slate-500 text-sm">
          Loading quarantined events…
        </div>
      ) : !filteredEvents.length ? (
        <EmptyState hasFilter={!!contractFilter || kindFilter !== "all" || timeWindow !== "all" || !!textFilter} />
      ) : (
        <div className="bg-[#111827] border border-[#1f2937] rounded-xl overflow-hidden">
          <table className="w-full text-sm">
            <thead>
              <tr className="border-b border-[#1f2937] text-left">
                <th className="w-10 px-4 py-3">
                  <input
                    type="checkbox"
                    checked={allSelected}
                    onChange={toggleAll}
                    className="rounded border-[#374151] bg-[#0a0d12] accent-indigo-500 cursor-pointer"
                  />
                </th>
                <th className="px-3 py-3 text-xs font-medium text-slate-500 uppercase tracking-wider">Time</th>
                <th className="px-3 py-3 text-xs font-medium text-slate-500 uppercase tracking-wider">Contract</th>
                <th className="px-3 py-3 text-xs font-medium text-slate-500 uppercase tracking-wider">Version</th>
                <th className="px-3 py-3 text-xs font-medium text-slate-500 uppercase tracking-wider">
                  <TooltipWrap content="Number of contract rule violations found in this event.">
                    <span className="cursor-default underline decoration-dotted">Violations</span>
                  </TooltipWrap>
                </th>
                <th className="px-3 py-3 text-xs font-medium text-slate-500 uppercase tracking-wider">Status</th>
                <th className="px-3 py-3 text-xs font-medium text-slate-500 uppercase tracking-wider">Source IP</th>
                <th className="px-3 py-3 text-xs font-medium text-slate-500 uppercase tracking-wider">Replays</th>
                <th className="px-3 py-3 text-xs font-medium text-slate-500 uppercase tracking-wider">Actions</th>
              </tr>
            </thead>
            <tbody className="divide-y divide-[#1f2937]">
              {filteredEvents.map((ev) => {
                const contractName =
                  contracts?.find((c) => c.id === ev.contract_id)?.name ??
                  ev.contract_id.slice(0, 8) + "…";
                const isPurged = ev.status === "purged";
                return (
                  <tr
                    key={ev.id}
                    className={clsx(
                      "transition-colors cursor-pointer",
                      selected.has(ev.id) ? "bg-indigo-900/10" : "hover:bg-[#1f2937]/30"
                    )}
                    onClick={() => setPreviewEvent(ev)}
                  >
                    <td className="px-4 py-3" onClick={(e) => e.stopPropagation()}>
                      <input
                        type="checkbox"
                        checked={selected.has(ev.id)}
                        onChange={() => toggleOne(ev.id)}
                        disabled={isPurged}
                        className="rounded border-[#374151] bg-[#0a0d12] accent-indigo-500 cursor-pointer disabled:opacity-30"
                      />
                    </td>
                    <td className="px-3 py-3 text-xs text-slate-400 whitespace-nowrap">
                      {new Date(ev.quarantined_at).toLocaleString()}
                    </td>
                    <td className="px-3 py-3 text-xs text-slate-300 max-w-[140px] truncate">
                      {contractName}
                    </td>
                    <td className="px-3 py-3 text-xs font-mono text-slate-500">
                      {ev.contract_version ? `v${ev.contract_version}` : "—"}
                    </td>
                    <td className="px-3 py-3">
                      <span className="inline-flex items-center gap-1 text-xs bg-red-900/30 text-red-400 border border-red-800/30 rounded-full px-2 py-0.5">
                        {ev.violation_count}
                      </span>
                    </td>
                    <td className="px-3 py-3">
                      <StatusBadge status={ev.status ?? "pending"} />
                    </td>
                    <td className="px-3 py-3 text-xs text-slate-500 font-mono">
                      {ev.source_ip ?? "—"}
                    </td>
                    <td className="px-3 py-3 text-xs text-slate-500">
                      {ev.replay_count > 0 ? (
                        <span
                          className={clsx(
                            "font-medium",
                            ev.last_replay_passed === true ? "text-green-400"
                              : ev.last_replay_passed === false ? "text-red-400"
                              : "text-slate-400"
                          )}
                        >
                          {ev.replay_count}×
                        </span>
                      ) : (
                        <span className="text-slate-700">—</span>
                      )}
                    </td>
                    <td className="px-3 py-3" onClick={(e) => e.stopPropagation()}>
                      <div className="flex items-center gap-1">
                        {isPurged ? (
                          <TooltipWrap content="Event purged — past retention window. Replay is no longer possible.">
                            <span className="text-xs text-slate-600 cursor-default px-2 py-1">
                              ⊘
                            </span>
                          </TooltipWrap>
                        ) : (
                          <button
                            onClick={() => openReplayConfirm([ev.id])}
                            disabled={replaying}
                            className="text-xs text-indigo-400 hover:text-indigo-300 transition-colors px-2 py-1 rounded hover:bg-indigo-900/20"
                          >
                            ▶
                          </button>
                        )}
                        <button
                          onClick={() => handleOpenDrawer(ev.id)}
                          className="text-xs text-slate-500 hover:text-slate-300 transition-colors px-2 py-1 rounded hover:bg-[#1f2937]"
                        >
                          History →
                        </button>
                      </div>
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      )}

      {/* Payload preview drawer */}
      {previewEvent && (
        <PayloadPreviewDrawer
          event={previewEvent}
          copied={previewCopied}
          onCopy={() => {
            navigator.clipboard.writeText(JSON.stringify(previewEvent.raw_event, null, 2)).catch(() => {});
            setPreviewCopied(true);
            setTimeout(() => setPreviewCopied(false), 1500);
          }}
          onClose={() => { setPreviewEvent(null); setPreviewCopied(false); }}
        />
      )}

      {/* Replay-history drawer */}
      {drawerEventId && (
        <ReplayHistoryDrawer
          eventId={drawerEventId}
          history={drawerHistory}
          loading={loadingHistory}
          onClose={closeDrawer}
        />
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// StatusBadge
// ---------------------------------------------------------------------------

function StatusBadge({ status }: { status: QuarantineStatus }) {
  const map: Record<QuarantineStatus, { label: string; cls: string }> = {
    pending: { label: "pending", cls: "bg-slate-800 text-slate-400" },
    reviewed: { label: "reviewed", cls: "bg-indigo-900/40 text-indigo-300" },
    replayed: { label: "replayed", cls: "bg-green-900/30 text-green-400" },
    purged: { label: "purged", cls: "bg-red-900/30 text-red-500 line-through" },
  };
  const { label, cls } = map[status] ?? map.pending;
  return (
    <span className={clsx("text-[10px] uppercase tracking-wider px-1.5 py-0.5 rounded font-mono", cls)}>
      {label}
    </span>
  );
}

// ---------------------------------------------------------------------------
// EmptyState
// ---------------------------------------------------------------------------

function EmptyState({ hasFilter }: { hasFilter: boolean }) {
  return (
    <div className="flex flex-col items-center justify-center h-64 text-slate-600 text-center px-4">
      <p className="text-4xl mb-4">🔒</p>
      <p className="text-sm text-slate-500">
        No quarantined events{hasFilter ? " matching those filters" : ""}.
      </p>
      {!hasFilter && (
        <>
          <p className="text-xs mt-2 text-slate-600 max-w-sm">
            Events land here when the backend quarantines on a validation, parse, or transform violation.
          </p>
          <p className="text-xs mt-3 text-slate-700 font-mono bg-[#111827] border border-[#1f2937] rounded px-3 py-2">
            make stack-up-demo
          </p>
          <p className="text-xs mt-2 text-slate-700">
            Run the demo seeder to generate sample quarantined events.
          </p>
        </>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// PayloadPreviewDrawer — read-only raw event viewer
// ---------------------------------------------------------------------------

function PayloadPreviewDrawer({
  event,
  copied,
  onCopy,
  onClose,
}: {
  event: QuarantinedEventEx;
  copied: boolean;
  onCopy: () => void;
  onClose: () => void;
}) {
  useEffect(() => {
    const handler = (e: KeyboardEvent) => { if (e.key === "Escape") onClose(); };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [onClose]);

  const prettyJson = JSON.stringify(event.raw_event ?? {}, null, 2);

  return (
    <>
      <button onClick={onClose} className="fixed inset-0 bg-black/40 z-40" aria-label="Close preview" />
      <div
        role="dialog"
        aria-modal="true"
        className="fixed top-0 right-0 h-full w-full md:w-[520px] bg-[#0d1117] border-l border-[#1f2937] z-50 shadow-2xl flex flex-col"
      >
        <div className="flex items-start justify-between px-6 py-5 border-b border-[#1f2937]">
          <div>
            <h3 className="text-sm font-semibold text-slate-200 uppercase tracking-wider">
              <TooltipWrap content="Events that failed contract validation are held here for inspection and optional replay. Nothing is silently dropped.">
                <span className="cursor-default underline decoration-dotted">Quarantined</span>
              </TooltipWrap>{" "}
              Payload
            </h3>
            <p className="text-xs text-slate-500 font-mono mt-1 truncate">{event.id}</p>
          </div>
          <button onClick={onClose} className="text-slate-500 hover:text-slate-200 text-xl leading-none ml-4" aria-label="Close">✕</button>
        </div>

        {/* Meta */}
        <div className="px-6 py-4 border-b border-[#1f2937] space-y-2">
          <div className="flex flex-wrap gap-2">
            <span className="text-xs bg-red-900/40 text-red-400 px-2 py-0.5 rounded-full font-medium">
              {event.violation_count} violation{event.violation_count !== 1 ? "s" : ""}
            </span>
            {event.contract_version && (
              <span className="text-xs font-mono bg-indigo-900/40 text-indigo-300 px-2 py-0.5 rounded-full">
                v{event.contract_version}
              </span>
            )}
            {event.status && <StatusBadge status={event.status} />}
            {event.source_ip && (
              <span className="text-xs text-slate-500 font-mono">{event.source_ip}</span>
            )}
          </div>
          {event.violation_details?.length > 0 && (
            <ul className="space-y-1">
              {event.violation_details.map((v, i) => (
                <li key={i} className="text-xs bg-red-900/20 border border-red-800/30 rounded px-3 py-1.5">
                  <span className="font-mono text-[10px] text-red-400 mr-2">{v.kind}</span>
                  <span className="font-mono text-slate-400">{v.field}</span>
                  <span className="text-slate-500 mx-1">·</span>
                  <span className="text-slate-300">{v.message}</span>
                </li>
              ))}
            </ul>
          )}
        </div>

        {/* Payload */}
        <div className="flex-1 overflow-auto px-6 py-4">
          <div className="flex items-center justify-between mb-2">
            <p className="text-xs text-slate-500 uppercase tracking-wider">Stored payload</p>
            <button
              onClick={onCopy}
              className="text-xs px-2.5 py-1 bg-[#1f2937] hover:bg-[#374151] border border-[#374151] rounded text-slate-300 transition-colors"
            >
              {copied ? "✓ Copied" : "Copy JSON"}
            </button>
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
// ReplayHistoryDrawer — per-event replay attempt chain
// ---------------------------------------------------------------------------

interface ReplayOutcomeEx extends ReplayOutcome {
  outcome?: string;
  transformed_event?: unknown;
}

function ReplayHistoryDrawer({
  eventId,
  history,
  loading,
  onClose,
}: {
  eventId: string;
  history: ReplayOutcomeEx[] | null;
  loading: boolean;
  onClose: () => void;
}) {
  useEffect(() => {
    const handler = (e: KeyboardEvent) => { if (e.key === "Escape") onClose(); };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [onClose]);

  return (
    <>
      <div className="fixed inset-0 z-40 bg-black/40" onClick={onClose} />
      <div
        role="dialog"
        aria-modal="true"
        className="fixed top-0 right-0 h-full w-full md:w-[480px] bg-[#0d1117] border-l border-[#1f2937] z-50 shadow-2xl flex flex-col"
      >
        <div className="flex items-center justify-between p-5 border-b border-[#1f2937]">
          <div>
            <h3 className="font-semibold text-slate-100">
              <TooltipWrap content="Re-validate a quarantined event against a current contract version. If it passes, it is written to the audit log and forwarded downstream.">
                <span className="cursor-default underline decoration-dotted">Replay</span>
              </TooltipWrap>{" "}
              History
            </h3>
            <p className="text-xs text-slate-600 font-mono mt-0.5">{eventId}</p>
          </div>
          <button onClick={onClose} className="text-slate-500 hover:text-slate-300 transition-colors text-xl leading-none" aria-label="Close">✕</button>
        </div>

        <div className="flex-1 overflow-auto p-5">
          {loading ? (
            <div className="flex items-center justify-center h-32 text-slate-500 text-sm">Loading history…</div>
          ) : !history || history.length === 0 ? (
            <div className="flex flex-col items-center justify-center h-32 text-slate-600">
              <p className="text-3xl mb-3">📭</p>
              <p className="text-sm">No replay attempts for this event yet.</p>
            </div>
          ) : (
            <div className="space-y-3">
              {history.map((h, i) => {
                const style = replayOutcomeStyle(h.passed, (h as ReplayOutcomeEx).outcome);
                const hasTransform = (h as ReplayOutcomeEx).transformed_event != null;
                return (
                  <div
                    key={i}
                    className={clsx(
                      "rounded-xl border p-4",
                      h.passed ? "bg-green-900/20 border-green-800/30" : "bg-red-900/20 border-red-800/30",
                      style.border
                    )}
                  >
                    <div className="flex items-center justify-between mb-2">
                      <span className={clsx("text-sm font-semibold", style.color)}>
                        {style.icon} {style.label}
                      </span>
                      <span className="text-xs text-slate-500 font-mono">v{h.version}</span>
                    </div>
                    <p className="text-xs text-slate-500">
                      {new Date(h.replayed_at).toLocaleString()}
                    </p>
                    {h.violations.length > 0 && (
                      <ul className="mt-3 space-y-1.5">
                        {h.violations.map((v, j) => (
                          <li key={j} className="text-xs bg-red-900/20 border border-red-800/30 rounded-lg px-3 py-2">
                            <span className="font-mono text-red-400">{v.field}</span>
                            <span className="text-slate-500 mx-1">·</span>
                            <span className="text-slate-300">{v.message}</span>
                          </li>
                        ))}
                      </ul>
                    )}
                    {/* Transform diff placeholder (RFC-020 D12) */}
                    {h.passed && (
                      <div className="mt-3 text-xs text-slate-600 bg-[#111827] border border-[#1f2937] rounded px-3 py-2">
                        {hasTransform ? (
                          <pre className="text-green-400 whitespace-pre-wrap break-all">
                            {JSON.stringify((h as ReplayOutcomeEx).transformed_event, null, 2)}
                          </pre>
                        ) : (
                          <span>Transform diff not available — server did not return post-transform payload.</span>
                        )}
                      </div>
                    )}
                  </div>
                );
              })}
            </div>
          )}
        </div>
      </div>
    </>
  );
}
