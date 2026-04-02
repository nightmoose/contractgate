"use client";

import useSWR from "swr";
import { getGlobalStats, getAuditLog, listContracts } from "@/lib/api";
import type { IngestionStats, AuditEntry, ContractSummary } from "@/lib/api";
import {
  AreaChart,
  Area,
  XAxis,
  YAxis,
  Tooltip,
  ResponsiveContainer,
} from "recharts";
import clsx from "clsx";

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
// Recent audit table
// ---------------------------------------------------------------------------

function AuditTable({ entries }: { entries: AuditEntry[] }) {
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
          </tr>
        </thead>
        <tbody>
          {entries.map((e) => (
            <tr key={e.id} className="border-b border-[#1f2937]/50 hover:bg-[#1f2937]/30">
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

export default function DashboardPage() {
  const { data: stats } = useSWR<IngestionStats>("stats", getGlobalStats, {
    refreshInterval: 5000,
  });
  const { data: audit } = useSWR<AuditEntry[]>(
    "audit-recent",
    () => getAuditLog({ limit: 20 }),
    { refreshInterval: 5000 }
  );
  const { data: contracts } = useSWR<ContractSummary[]>("contracts", listContracts);

  const passRate = stats ? (stats.pass_rate * 100).toFixed(1) : "—";
  const avgLatency = stats
    ? stats.avg_validation_us < 1000
      ? `${stats.avg_validation_us.toFixed(0)}µs`
      : `${(stats.avg_validation_us / 1000).toFixed(2)}ms`
    : "—";

  return (
    <div>
      {/* Header */}
      <div className="flex items-center justify-between mb-8">
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
          sub="p99 target: <15ms"
          color="text-green-400"
        />
      </div>

      {/* Contracts summary */}
      <div className="grid grid-cols-1 lg:grid-cols-3 gap-6 mb-8">
        <div className="bg-[#111827] border border-[#1f2937] rounded-xl p-5">
          <h2 className="text-sm font-semibold text-slate-400 mb-3 uppercase tracking-wider">
            Active Contracts
          </h2>
          {contracts && contracts.length > 0 ? (
            <ul className="space-y-2">
              {contracts.slice(0, 6).map((c) => (
                <li
                  key={c.id}
                  className="flex items-center justify-between text-sm"
                >
                  <span className="text-slate-300 font-medium">{c.name}</span>
                  <span className="flex items-center gap-2">
                    <span className="text-xs text-slate-500">v{c.version}</span>
                    <span
                      className={clsx(
                        "w-2 h-2 rounded-full",
                        c.active ? "bg-green-400" : "bg-slate-600"
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
          <h2 className="text-sm font-semibold text-slate-400 mb-3 uppercase tracking-wider">
            Recent Events
          </h2>
          {audit && audit.length > 0 ? (
            <AuditTable entries={audit.slice(0, 8)} />
          ) : (
            <div className="flex items-center justify-center h-32 text-slate-600 text-sm">
              No events yet — send data to{" "}
              <code className="ml-1 text-green-600">
                POST /ingest/{"{"}&lt;contract_id&gt;{"}"}
              </code>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
