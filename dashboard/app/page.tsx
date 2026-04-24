"use client";

import { useState } from "react";
import useSWR from "swr";
import { getGlobalStats, getAuditLog, listContracts } from "@/lib/api";
import type { IngestionStats, AuditEntry, ContractSummary } from "@/lib/api";
import clsx from "clsx";
import { useRouter } from "next/navigation";
import AuthGate from "@/components/AuthGate";

// ---------------------------------------------------------------------------
// Hero banner — dismissible pitch for first-time / non-technical visitors
// ---------------------------------------------------------------------------

function HeroBanner() {
  const [dismissed, setDismissed] = useState(false);
  const [expanded, setExpanded] = useState(false);

  if (dismissed) return null;

  return (
    <div className="mb-8 bg-gradient-to-r from-green-950/30 to-[#111827] border border-green-800/30 rounded-xl p-5">
      <div className="flex items-start justify-between gap-4">
        <div className="flex-1">
          <div className="flex items-center gap-2 mb-2">
            <span className="text-xs bg-green-900/50 text-green-400 border border-green-700/50 px-2 py-0.5 rounded-full font-medium">
              Patent Pending
            </span>
            <span className="text-xs text-slate-600">ContractGate v0.1</span>
          </div>
          <p className="text-slate-200 font-semibold text-base leading-snug">
            Enforce ontology + glossary + metric rules in{" "}
            <span className="text-green-400">&lt;50µs</span> — before data ever lands.
          </p>
          <div className="flex flex-wrap gap-2 mt-3">
            {[
              "60–90% data cleaning cost reduction",
              "Reduced LLM hallucination & model drift",
              "Streaming-compatible",
            ].map((b) => (
              <span
                key={b}
                className="text-xs text-slate-400 bg-[#1f2937]/70 border border-[#374151] px-3 py-1 rounded-full"
              >
                ✓ {b}
              </span>
            ))}
          </div>
          <button
            onClick={() => setExpanded((x) => !x)}
            className="mt-3 text-xs text-green-500 hover:text-green-400 flex items-center gap-1 transition-colors"
          >
            {expanded ? "▲ Hide" : "▼ How it works"}
          </button>

          {expanded && (
            <div className="mt-4 grid grid-cols-1 md:grid-cols-3 gap-4 border-t border-[#1f2937] pt-4">
              {[
                {
                  step: "1",
                  title: "Define a Contract",
                  desc: "Write a YAML schema declaring entities, glossary terms, and metric rules — your semantic truth.",
                },
                {
                  step: "2",
                  title: "Stream Events In",
                  desc: "POST events to /ingest/{contract_id}. ContractGate validates in microseconds, inline in your pipeline.",
                },
                {
                  step: "3",
                  title: "Pass or Quarantine",
                  desc: "Clean events flow to storage. Violations are flagged, quarantined, or rejected with structured error details.",
                },
              ].map(({ step, title, desc }) => (
                <div key={step} className="flex gap-3">
                  <span className="text-green-400 font-bold text-lg leading-none">{step}.</span>
                  <div>
                    <p className="text-sm font-medium text-slate-300">{title}</p>
                    <p className="text-xs text-slate-500 mt-0.5 leading-relaxed">{desc}</p>
                  </div>
                </div>
              ))}
            </div>
          )}
        </div>
        <button
          onClick={() => setDismissed(true)}
          className="text-slate-600 hover:text-slate-400 text-sm transition-colors flex-shrink-0"
          aria-label="Dismiss"
        >
          ✕
        </button>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Demo mode context banner
// ---------------------------------------------------------------------------

const HEALTHY_BASELINE: IngestionStats = {
  total_events: 14382,
  passed_events: 14137,
  failed_events: 245,
  pass_rate: 0.983,
  avg_validation_us: 38,
  p50_validation_us: 31,
  p95_validation_us: 72,
  p99_validation_us: 114,
};

function DemoContextBanner({
  passRate,
  healthyMode,
  onToggle,
}: {
  passRate: number | null;
  healthyMode: boolean;
  onToggle: () => void;
}) {
  if (passRate === null) return null;
  // Show when pass rate is below 90% (stress-test territory), or when healthy mode is on
  if (passRate >= 0.9 && !healthyMode) return null;

  return (
    <div className="mb-6 flex items-center justify-between gap-3 bg-amber-950/20 border border-amber-700/30 rounded-xl px-4 py-3">
      <div className="flex items-center gap-2">
        <span className="text-amber-400 text-base">⚡</span>
        <p className="text-sm text-amber-300/90">
          {healthyMode ? (
            <>
              <span className="font-medium text-green-400">Healthy baseline preview</span>
              {" — "}simulated 98%+ pass-rate to show the product at its best
            </>
          ) : (
            <>
              <span className="font-medium">Stress-test mode</span>
              {" — "}intentionally sending malformed events to demonstrate enforcement in action
            </>
          )}
        </p>
      </div>
      <button
        onClick={onToggle}
        className={clsx(
          "text-xs px-3 py-1.5 rounded-lg border transition-colors whitespace-nowrap",
          healthyMode
            ? "border-slate-600 text-slate-400 hover:text-slate-200 hover:border-slate-400"
            : "border-green-700/50 text-green-400 hover:bg-green-900/20"
        )}
      >
        {healthyMode ? "← Show real data" : "Preview healthy baseline →"}
      </button>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Stat card
// ---------------------------------------------------------------------------

function StatCard({
  label,
  value,
  sub,
  color = "text-white",
}: {
  label: string;
  value: string | number;
  sub?: string;
  color?: string;
}) {
  return (
    <div className="bg-[#111827] border border-[#1f2937] rounded-xl p-5">
      <p className="text-xs text-slate-500 uppercase tracking-wider mb-1">{label}</p>
      <p className={clsx("text-3xl font-bold", color)}>{value}</p>
      {sub && <p className="text-xs text-slate-500 mt-1">{sub}</p>}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Recent audit table — rows link to /audit for quick drill-down
// ---------------------------------------------------------------------------

function AuditTable({ entries }: { entries: AuditEntry[] }) {
  const router = useRouter();

  return (
    <div className="overflow-x-auto">
      <table className="w-full text-sm">
        <thead>
          <tr className="text-xs text-slate-500 border-b border-[#1f2937]">
            <th className="pb-2 text-left font-medium">Time</th>
            <th className="pb-2 text-left font-medium">Contract</th>
            <th className="pb-2 text-left font-medium">Status</th>
            <th className="pb-2 text-left font-medium">Violations</th>
            <th className="pb-2 text-left font-medium">Latency</th>
            <th className="pb-2 text-left font-medium"></th>
          </tr>
        </thead>
        <tbody>
          {entries.map((e) => (
            <tr
              key={e.id}
              className="border-b border-[#1f2937]/50 hover:bg-[#1f2937]/30 cursor-pointer group"
              onClick={() => router.push(`/audit?contract_id=${e.contract_id}`)}
              title="View in Audit Log"
            >
              <td className="py-2.5 text-slate-400 font-mono text-xs">
                {new Date(e.created_at).toLocaleTimeString()}
              </td>
              <td className="py-2.5 text-slate-400 font-mono text-xs truncate max-w-[120px]">
                {e.contract_id.slice(0, 8)}…
              </td>
              <td className="py-2.5">
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
              <td className="py-2.5 text-slate-400">{e.violation_count}</td>
              <td className="py-2.5 text-slate-400 font-mono text-xs">
                {e.validation_us}µs
              </td>
              <td className="py-2.5">
                <span className="text-xs text-slate-600 group-hover:text-green-500 transition-colors">
                  → details
                </span>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Page
// ---------------------------------------------------------------------------

function DashboardContent() {
  const [healthyMode, setHealthyMode] = useState(false);

  const { data: rawStats } = useSWR<IngestionStats>("stats", getGlobalStats, {
    refreshInterval: 5000,
  });
  const { data: audit } = useSWR<AuditEntry[]>(
    "audit-recent",
    () => getAuditLog({ limit: 20 }),
    { refreshInterval: 5000 }
  );
  const { data: contracts } = useSWR<ContractSummary[]>("contracts", listContracts);

  const stats = healthyMode ? HEALTHY_BASELINE : rawStats;

  const passRate = stats ? (stats.pass_rate * 100).toFixed(1) : "—";
  const avgLatency = stats
    ? stats.avg_validation_us < 1000
      ? `${stats.avg_validation_us.toFixed(0)}µs`
      : `${(stats.avg_validation_us / 1000).toFixed(2)}ms`
    : "—";

  return (
    <div>
      {/* Hero pitch banner */}
      <HeroBanner />

      {/* Header */}
      <div className="flex items-center justify-between mb-6">
        <div>
          <h1 className="text-2xl font-bold">Live Monitor</h1>
          <p className="text-sm text-slate-500 mt-1">
            Real-time ingestion health — refreshes every 5s
          </p>
        </div>
        <span className="text-xs bg-green-900/40 text-green-400 border border-green-700/50 px-3 py-1 rounded-full font-medium animate-pulse">
          ● Live
        </span>
      </div>

      {/* Demo context banner */}
      <DemoContextBanner
        passRate={rawStats?.pass_rate ?? null}
        healthyMode={healthyMode}
        onToggle={() => setHealthyMode((x) => !x)}
      />

      {/* Stats row */}
      <div className="grid grid-cols-2 lg:grid-cols-4 gap-4 mb-8">
        <StatCard
          label="Total Events"
          value={stats?.total_events.toLocaleString() ?? "—"}
        />
        <StatCard
          label="Pass Rate"
          value={`${passRate}%`}
          color={
            stats && stats.pass_rate > 0.95
              ? "text-green-400"
              : stats && stats.pass_rate > 0.8
              ? "text-yellow-400"
              : "text-red-400"
          }
        />
        <StatCard
          label="Violations"
          value={stats?.failed_events.toLocaleString() ?? "—"}
          color="text-red-400"
        />
        <StatCard
          label="Avg Latency"
          value={avgLatency}
          sub={
            stats?.p99_validation_us != null
              ? `p99: ${
                  stats.p99_validation_us < 1000
                    ? `${stats.p99_validation_us}µs`
                    : `${(stats.p99_validation_us / 1000).toFixed(2)}ms`
                } (target <15ms)`
              : "p99 target: <15ms"
          }
          color={
            stats?.p99_validation_us != null && stats.p99_validation_us < 15000
              ? "text-green-400"
              : "text-yellow-400"
          }
        />
      </div>

      {/* Contracts summary + recent events */}
      <div className="grid grid-cols-1 lg:grid-cols-3 gap-6 mb-8">
        <div className="bg-[#111827] border border-[#1f2937] rounded-xl p-5">
          <h2 className="text-sm font-semibold text-slate-400 mb-3 uppercase tracking-wider">
            Contracts
          </h2>
          {contracts && contracts.length > 0 ? (
            <ul className="space-y-2">
              {contracts.slice(0, 6).map((c) => (
                <li
                  key={c.id}
                  className="flex items-center justify-between text-sm"
                >
                  <a
                    href="/contracts"
                    className="text-slate-300 font-medium hover:text-green-400 transition-colors"
                  >
                    {c.name}
                  </a>
                  <span className="flex items-center gap-2">
                    {c.latest_stable_version ? (
                      <span className="text-xs text-slate-500">
                        stable v{c.latest_stable_version}
                      </span>
                    ) : (
                      <span className="text-xs text-amber-500">draft only</span>
                    )}
                    <span
                      className={clsx(
                        "w-2 h-2 rounded-full",
                        c.latest_stable_version
                          ? "bg-green-400"
                          : "bg-amber-500"
                      )}
                    />
                  </span>
                </li>
              ))}
            </ul>
          ) : (
            <p className="text-sm text-slate-600">No contracts yet</p>
          )}
        </div>

        {/* Recent audit */}
        <div className="lg:col-span-2 bg-[#111827] border border-[#1f2937] rounded-xl p-5">
          <div className="flex items-center justify-between mb-3">
            <h2 className="text-sm font-semibold text-slate-400 uppercase tracking-wider">
              Recent Events
            </h2>
            <a
              href="/audit"
              className="text-xs text-green-600 hover:text-green-400 transition-colors"
            >
              View full audit log →
            </a>
          </div>
          {audit && audit.length > 0 ? (
            <AuditTable entries={audit.slice(0, 8)} />
          ) : (
            <div className="flex items-center justify-center h-32 text-slate-600 text-sm">
              No events yet — send data to{" "}
              <code className="ml-1 text-green-600">
                POST /ingest/{"{"}
                &lt;contract_id&gt;
                {"}"}
              </code>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

export default function DashboardPage() {
  return (
    <AuthGate page="dashboard">
      <DashboardContent />
    </AuthGate>
  );
}
