"use client";

/**
 * Provider Scorecard page — RFC-031.
 *
 * Shows per-provider pass/quarantine rates, ranked field violations,
 * and active drift signals. Producers use this to share objective
 * data-quality evidence with upstream providers. Consumers can see their
 * own data quality at a glance.
 *
 * URL: /scorecard
 * Searches by ?source= query param or inline source picker.
 */

import { useState, useEffect, Suspense } from "react";
import { useSearchParams } from "next/navigation";
import AuthGate from "@/components/AuthGate";
import {
  getScorecard,
  getScorecardExportUrl,
} from "@/lib/api";
import type { FullScorecard, ScorecardSummaryRow, FieldHealthRow, DriftSignal } from "@/lib/api";
import clsx from "clsx";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function pct(n: number) {
  return `${n.toFixed(2)}%`;
}

function deltaColor(delta: number) {
  if (delta > 5) return "text-red-400";
  if (delta > 2) return "text-amber-400";
  return "text-green-400";
}

// ---------------------------------------------------------------------------
// Summary cards
// ---------------------------------------------------------------------------

function SummaryCards({ rows }: { rows: ScorecardSummaryRow[] }) {
  const totals = rows.reduce(
    (acc, r) => ({
      total_events: acc.total_events + r.total_events,
      passed: acc.passed + r.passed,
      quarantined: acc.quarantined + r.quarantined,
    }),
    { total_events: 0, passed: 0, quarantined: 0 }
  );
  const overallQpct =
    totals.total_events > 0
      ? ((totals.quarantined / totals.total_events) * 100).toFixed(2)
      : "—";

  return (
    <div className="grid grid-cols-2 lg:grid-cols-4 gap-4">
      {[
        { label: "Total Events", value: totals.total_events.toLocaleString(), color: "text-white" },
        { label: "Passed", value: totals.passed.toLocaleString(), color: "text-green-400" },
        { label: "Quarantined", value: totals.quarantined.toLocaleString(), color: "text-red-400" },
        {
          label: "Quarantine Rate",
          value: `${overallQpct}%`,
          color:
            parseFloat(overallQpct) > 10
              ? "text-red-400"
              : parseFloat(overallQpct) > 3
              ? "text-amber-400"
              : "text-green-400",
        },
      ].map((c) => (
        <div key={c.label} className="bg-[#111827] border border-[#1f2937] rounded-xl p-5">
          <p className="text-xs text-slate-500 uppercase tracking-wider mb-1">{c.label}</p>
          <p className={clsx("text-2xl font-bold", c.color)}>{c.value}</p>
        </div>
      ))}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Per-contract breakdown table
// ---------------------------------------------------------------------------

function ContractTable({ rows }: { rows: ScorecardSummaryRow[] }) {
  return (
    <div className="bg-[#111827] border border-[#1f2937] rounded-xl overflow-hidden">
      <div className="px-5 py-4 border-b border-[#1f2937]">
        <h2 className="text-sm font-semibold text-slate-300 uppercase tracking-wider">
          Per-Contract Breakdown
        </h2>
      </div>
      <div className="overflow-x-auto">
        <table className="w-full text-sm">
          <thead>
            <tr className="text-xs text-slate-500 border-b border-[#1f2937]">
              <th className="px-5 py-3 text-left font-medium">Contract</th>
              <th className="px-5 py-3 text-right font-medium">Total</th>
              <th className="px-5 py-3 text-right font-medium">Passed</th>
              <th className="px-5 py-3 text-right font-medium">Quarantined</th>
              <th className="px-5 py-3 text-right font-medium">Quarantine %</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((r) => (
              <tr key={r.contract_name} className="border-b border-[#1f2937]/50 hover:bg-[#1f2937]/30">
                <td className="px-5 py-3 font-mono text-slate-200">{r.contract_name}</td>
                <td className="px-5 py-3 text-right text-slate-400">{r.total_events.toLocaleString()}</td>
                <td className="px-5 py-3 text-right text-green-400">{r.passed.toLocaleString()}</td>
                <td className="px-5 py-3 text-right text-red-400">{r.quarantined.toLocaleString()}</td>
                <td className="px-5 py-3 text-right">
                  <span
                    className={clsx(
                      "font-medium",
                      r.quarantine_pct > 10
                        ? "text-red-400"
                        : r.quarantine_pct > 3
                        ? "text-amber-400"
                        : "text-green-400"
                    )}
                  >
                    {pct(r.quarantine_pct)}
                  </span>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Field health table
// ---------------------------------------------------------------------------

function FieldHealthTable({ rows }: { rows: FieldHealthRow[] }) {
  const top = rows.slice(0, 20); // top 20 by violation count
  return (
    <div className="bg-[#111827] border border-[#1f2937] rounded-xl overflow-hidden">
      <div className="px-5 py-4 border-b border-[#1f2937]">
        <h2 className="text-sm font-semibold text-slate-300 uppercase tracking-wider">
          Field-Level Violations
          <span className="ml-2 text-slate-600 normal-case font-normal text-xs">top {top.length}</span>
        </h2>
      </div>
      {top.length === 0 ? (
        <p className="px-5 py-8 text-sm text-slate-600 text-center">No field violations recorded.</p>
      ) : (
        <div className="overflow-x-auto">
          <table className="w-full text-sm">
            <thead>
              <tr className="text-xs text-slate-500 border-b border-[#1f2937]">
                <th className="px-5 py-3 text-left font-medium">Contract</th>
                <th className="px-5 py-3 text-left font-medium">Field</th>
                <th className="px-5 py-3 text-left font-medium">Violation Code</th>
                <th className="px-5 py-3 text-right font-medium">Count</th>
              </tr>
            </thead>
            <tbody>
              {top.map((r, i) => (
                <tr key={i} className="border-b border-[#1f2937]/50 hover:bg-[#1f2937]/30">
                  <td className="px-5 py-3 font-mono text-slate-400 text-xs">{r.contract_name}</td>
                  <td className="px-5 py-3 font-mono text-sky-300">{r.field}</td>
                  <td className="px-5 py-3">
                    <span className="text-xs bg-red-900/30 text-red-300 border border-red-800/40 rounded px-2 py-0.5 font-mono">
                      {r.code}
                    </span>
                  </td>
                  <td className="px-5 py-3 text-right text-red-400 font-medium">
                    {r.violations.toLocaleString()}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Drift signals
// ---------------------------------------------------------------------------

function DriftPanel({ signals }: { signals: DriftSignal[] }) {
  if (signals.length === 0) {
    return (
      <div className="bg-[#111827] border border-[#1f2937] rounded-xl px-5 py-8 text-center">
        <p className="text-green-400 text-sm font-medium">✓ No active drift signals</p>
        <p className="text-xs text-slate-600 mt-1">
          Field null- and violation-rates are within the 30-day baseline.
        </p>
      </div>
    );
  }
  return (
    <div className="bg-[#111827] border border-[#1f2937] rounded-xl overflow-hidden">
      <div className="px-5 py-4 border-b border-[#1f2937] flex items-center justify-between">
        <h2 className="text-sm font-semibold text-slate-300 uppercase tracking-wider">
          Active Drift Signals
        </h2>
        <span className="text-xs bg-amber-900/40 text-amber-300 border border-amber-700/40 rounded-full px-2.5 py-0.5 font-medium">
          {signals.length} alert{signals.length !== 1 ? "s" : ""}
        </span>
      </div>
      <div className="divide-y divide-[#1f2937]">
        {signals.map((s, i) => (
          <div key={i} className="px-5 py-4 flex items-start justify-between gap-3">
            <div className="flex-1 min-w-0">
              <div className="flex items-center gap-2 flex-wrap mb-1">
                <span className="font-mono text-sm text-slate-200">{s.field}</span>
                <span className="text-[10px] bg-sky-900/30 text-sky-300 border border-sky-800/40 rounded px-1.5 py-0.5">
                  {s.signal_type === "null_rate" ? "null rate" : "violation rate"}
                </span>
                <span className="text-xs text-slate-500 font-mono">{s.contract_name}</span>
              </div>
              <p className="text-xs text-slate-500">
                Baseline: <span className="text-slate-300">{pct(s.baseline_rate * 100)}</span>
                {" → "}
                Current: <span className="text-slate-300">{pct(s.current_rate * 100)}</span>
              </p>
            </div>
            <div className="shrink-0 text-right">
              <p className={clsx("text-lg font-bold", deltaColor(Math.abs(s.delta * 100)))}>
                {s.delta > 0 ? "+" : ""}
                {pct(s.delta * 100)}
              </p>
              <p className="text-[10px] text-slate-600">delta</p>
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Page content
// ---------------------------------------------------------------------------

function ScorecardContent() {
  const searchParams = useSearchParams();
  const [source, setSource] = useState(searchParams?.get("source") ?? "");
  const [input, setInput] = useState(source);
  const [scorecard, setScorecard] = useState<FullScorecard | null>(null);
  const [loading, setLoading] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  const load = (src: string) => {
    if (!src.trim()) return;
    setLoading(true); setErr(null); setScorecard(null);
    getScorecard(src.trim())
      .then(setScorecard)
      .catch((e) => setErr(e instanceof Error ? e.message : String(e)))
      .finally(() => setLoading(false));
  };

  // Load on mount if source is in the URL
  useEffect(() => {
    if (source) load(source);
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  const handleSearch = () => {
    setSource(input.trim());
    load(input.trim());
  };

  return (
    <div>
      {/* Header */}
      <div className="flex items-start justify-between mb-6 flex-wrap gap-4">
        <div>
          <h1 className="text-2xl font-bold">Provider Scorecard</h1>
          <p className="text-sm text-slate-500 mt-1">
            Objective data-quality evidence — turn "your data is bad" into a verifiable report.
          </p>
        </div>
        {scorecard && (
          <a
            href={getScorecardExportUrl(scorecard.source)}
            target="_blank"
            rel="noopener noreferrer"
            className="px-4 py-2 bg-[#1f2937] hover:bg-[#374151] text-slate-300 text-sm font-medium rounded-lg transition-colors border border-[#374151] flex items-center gap-2"
          >
            ↓ Export CSV
          </a>
        )}
      </div>

      {/* Source picker */}
      <div className="bg-[#111827] border border-[#1f2937] rounded-xl p-5 mb-6">
        <label className="text-xs font-medium text-slate-400 uppercase tracking-wider mb-2 block">
          Provider Source
        </label>
        <p className="text-xs text-slate-600 mb-3">
          Enter the source name as configured in your contracts (the <code className="font-mono">source</code> field
          set at deploy time — e.g. a PMS vendor name or feed identifier).
        </p>
        <div className="flex gap-2">
          <input
            type="text"
            value={input}
            onChange={(e) => setInput(e.target.value)}
            onKeyDown={(e) => { if (e.key === "Enter") handleSearch(); }}
            placeholder="e.g. yardi, entrata, appfolio…"
            className="flex-1 bg-[#0a0d12] border border-[#1f2937] rounded-lg px-4 py-2 text-sm text-slate-200 placeholder-slate-600 outline-none focus:border-indigo-600 transition-colors"
          />
          <button
            onClick={handleSearch}
            disabled={loading || !input.trim()}
            className="px-4 py-2 bg-indigo-700 hover:bg-indigo-600 disabled:opacity-40 text-white text-sm font-medium rounded-lg transition-colors"
          >
            {loading ? "Loading…" : "Load Scorecard"}
          </button>
        </div>
      </div>

      {/* Error */}
      {err && (
        <div className="mb-6 bg-red-900/20 border border-red-800/40 rounded-xl p-4">
          <p className="text-sm text-red-400">✕ {err}</p>
        </div>
      )}

      {/* Scorecard body */}
      {scorecard && (
        <div className="space-y-6">
          {/* Source badge */}
          <div className="flex items-center gap-3">
            <span className="text-lg font-semibold text-slate-200">{scorecard.source}</span>
            <span className="text-xs bg-sky-900/30 text-sky-300 border border-sky-800/40 rounded-full px-3 py-0.5">
              provider
            </span>
          </div>

          <SummaryCards rows={scorecard.summary} />

          <div className="grid grid-cols-1 lg:grid-cols-3 gap-6">
            <div className="lg:col-span-2">
              <ContractTable rows={scorecard.summary} />
            </div>
            <div>
              <DriftPanel signals={scorecard.drift} />
            </div>
          </div>

          <FieldHealthTable rows={scorecard.field_health} />

          {/* RFC note */}
          <p className="text-xs text-slate-700 text-center">
            Data aggregated from <code className="font-mono">audit_log</code> and{" "}
            <code className="font-mono">quarantine_events</code> · Drift baseline: 30-day rolling, 24h window
          </p>
        </div>
      )}

      {!scorecard && !loading && !err && (
        <div className="flex flex-col items-center justify-center h-48 text-slate-600">
          <p className="text-4xl mb-3">📊</p>
          <p className="text-sm">Enter a provider source name above to load its scorecard.</p>
        </div>
      )}
    </div>
  );
}

export default function ScorecardPage() {
  return (
    <AuthGate page="scorecard">
      <Suspense>
        <ScorecardContent />
      </Suspense>
    </AuthGate>
  );
}
