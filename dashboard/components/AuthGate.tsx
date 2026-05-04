"use client";

import { useEffect, useState } from "react";
import Link from "next/link";
import { createClient } from "@/lib/supabase/client";
import { DEMO_MODE } from "@/lib/demo";

// ── Per-page preview content ───────────────────────────────────────────────

const PREVIEWS: Record<string, {
  title: string;
  description: string;
  cta: string;
  illustration: React.ReactNode;
}> = {
  dashboard: {
    title: "Live Validation Dashboard",
    description: "Monitor validation rates, p99 latency, and violation trends across all your contracts in real-time. Spot data quality issues the moment they enter your pipeline.",
    cta: "Sign in to view your dashboard",
    illustration: <DashboardIllustration />,
  },
  contracts: {
    title: "Semantic Contract Management",
    description: "Define exactly what valid data looks like using a clean YAML schema. Version your contracts, promote stable releases, and deprecate old ones — without redeploying connectors.",
    cta: "Sign in to manage contracts",
    illustration: <ContractsIllustration />,
  },
  audit: {
    title: "Full Audit Trail",
    description: "Every validation decision logged with field-level violation details, contract version, and latency. Query by contract, time range, or violation type. Full data lineage, zero guesswork.",
    cta: "Sign in to view audit logs",
    illustration: <AuditIllustration />,
  },
  playground: {
    title: "Contract Playground",
    description: "Test your contracts interactively before shipping to production. Paste a contract YAML and sample event JSON, see exactly which fields pass or fail and why — in milliseconds.",
    cta: "Sign in to open the playground",
    illustration: <PlaygroundIllustration />,
  },
  account: {
    title: "API Keys & Account",
    description: "Generate API keys for your Kafka connectors, track usage, and revoke compromised keys instantly. Keys are hashed — only you ever see the full value.",
    cta: "Sign in to manage your account",
    illustration: <AccountIllustration />,
  },
};

// ── Illustrations ─────────────────────────────────────────────────────────────

function DashboardIllustration() {
  return (
    <div className="w-full rounded-xl border border-[#1f2937] bg-[#0d1117] p-5 space-y-4 select-none pointer-events-none">
      {/* Stat cards */}
      <div className="grid grid-cols-3 gap-3">
        {[["Total Events", "2,847,391", "text-slate-300"], ["Pass Rate", "94.2%", "text-green-400"], ["p99 Latency", "8ms", "text-green-400"]].map(([label, val, color]) => (
          <div key={label} className="bg-[#111827] border border-[#1f2937] rounded-lg p-3">
            <div className="text-xs text-slate-500 mb-1">{label}</div>
            <div className={`text-xl font-bold ${color}`}>{val}</div>
          </div>
        ))}
      </div>
      {/* Fake chart bars */}
      <div className="bg-[#111827] border border-[#1f2937] rounded-lg p-3">
        <div className="text-xs text-slate-500 mb-3">Validation rate (last 24h)</div>
        <div className="flex items-end gap-1 h-12">
          {[60,75,55,80,90,70,85,92,88,78,95,82,76,90,94,88,91,85,93,87,96,90,94,92].map((h, i) => (
            <div key={i} className="flex-1 bg-green-900/50 rounded-sm" style={{ height: `${h}%` }} />
          ))}
        </div>
      </div>
      {/* Recent violations */}
      <div className="bg-[#111827] border border-[#1f2937] rounded-lg p-3 space-y-2">
        <div className="text-xs text-slate-500 mb-2">Recent violations</div>
        {[["user_id", "missing_required_field"], ["amount", "range_violation"], ["event_type", "enum_violation"]].map(([field, kind]) => (
          <div key={field} className="flex items-center gap-2 text-xs">
            <span className="w-2 h-2 rounded-full bg-red-500/60 flex-shrink-0" />
            <code className="text-slate-400">{field}</code>
            <span className="text-slate-600">{kind}</span>
          </div>
        ))}
      </div>
    </div>
  );
}

function ContractsIllustration() {
  return (
    <div className="w-full rounded-xl border border-[#1f2937] bg-[#0d1117] p-5 select-none pointer-events-none">
      <div className="flex gap-3">
        {/* Contract list */}
        <div className="w-2/5 space-y-2">
          {[["user_events", "v1.2.0", true], ["order_pipeline", "v2.0.1", false], ["ml_features", "v1.0.0", false]].map(([name, ver, active]) => (
            <div key={name as string} className={`rounded-lg border p-2.5 text-xs ${active ? "border-green-700/50 bg-green-900/20" : "border-[#1f2937] bg-[#111827]"}`}>
              <div className={`font-medium ${active ? "text-green-400" : "text-slate-300"}`}>{name}</div>
              <div className="text-slate-600 mt-0.5">{ver} · stable</div>
            </div>
          ))}
        </div>
        {/* YAML preview */}
        <div className="flex-1 bg-[#111827] border border-[#1f2937] rounded-lg p-3 font-mono text-xs leading-relaxed">
          <div className="text-slate-500">version: <span className="text-green-400">&quot;1.2.0&quot;</span></div>
          <div className="text-slate-500">ontology:</div>
          <div className="text-slate-500 pl-2">entities:</div>
          <div className="text-slate-500 pl-4">- name: <span className="text-blue-400">user_id</span></div>
          <div className="text-slate-500 pl-6">type: <span className="text-yellow-400">string</span></div>
          <div className="text-slate-500 pl-6">required: <span className="text-orange-400">true</span></div>
          <div className="text-slate-500 pl-4">- name: <span className="text-blue-400">amount</span></div>
          <div className="text-slate-500 pl-6">type: <span className="text-yellow-400">number</span></div>
          <div className="text-slate-500 pl-6">min: <span className="text-orange-400">0</span></div>
        </div>
      </div>
    </div>
  );
}

function AuditIllustration() {
  const rows = [
    { time: "12:04:31", contract: "user_events", passed: true, ms: "4ms" },
    { time: "12:04:31", contract: "user_events", passed: false, ms: "6ms" },
    { time: "12:04:30", contract: "order_pipeline", passed: true, ms: "3ms" },
    { time: "12:04:30", contract: "user_events", passed: true, ms: "5ms" },
    { time: "12:04:29", contract: "ml_features", passed: false, ms: "7ms" },
  ];
  return (
    <div className="w-full rounded-xl border border-[#1f2937] bg-[#0d1117] overflow-hidden select-none pointer-events-none">
      <div className="grid grid-cols-4 text-xs text-slate-600 px-4 py-2 border-b border-[#1f2937] bg-[#111827]">
        <span>Time</span><span>Contract</span><span>Result</span><span>Latency</span>
      </div>
      {rows.map((r, i) => (
        <div key={i} className="grid grid-cols-4 text-xs px-4 py-2 border-b border-[#1f2937]/50 hover:bg-[#111827]/50">
          <span className="text-slate-600 font-mono">{r.time}</span>
          <span className="text-slate-400">{r.contract}</span>
          <span className={r.passed ? "text-green-400" : "text-red-400"}>{r.passed ? "✓ pass" : "✗ fail"}</span>
          <span className="text-slate-600">{r.ms}</span>
        </div>
      ))}
    </div>
  );
}

function PlaygroundIllustration() {
  return (
    <div className="w-full rounded-xl border border-[#1f2937] bg-[#0d1117] p-4 select-none pointer-events-none">
      <div className="flex gap-3">
        <div className="flex-1 space-y-2">
          <div className="text-xs text-slate-500 mb-1">Contract YAML</div>
          <div className="bg-[#111827] border border-[#1f2937] rounded-lg p-3 font-mono text-xs text-slate-500 leading-relaxed">
            <div>ontology:</div>
            <div className="pl-2">entities:</div>
            <div className="pl-4">- name: <span className="text-blue-400">user_id</span></div>
            <div className="pl-6">required: <span className="text-orange-400">true</span></div>
          </div>
          <div className="text-xs text-slate-500 mb-1 mt-2">Test Event</div>
          <div className="bg-[#111827] border border-[#1f2937] rounded-lg p-3 font-mono text-xs text-slate-500">
            <div>{"{"}</div>
            <div className="pl-2">&quot;event_type&quot;: <span className="text-green-400">&quot;click&quot;</span>,</div>
            <div className="pl-2">&quot;amount&quot;: <span className="text-orange-400">-5</span></div>
            <div>{"}"}</div>
          </div>
        </div>
        <div className="flex-1">
          <div className="text-xs text-slate-500 mb-1">Result</div>
          <div className="bg-red-900/20 border border-red-700/30 rounded-lg p-3 space-y-2">
            <div className="text-xs font-medium text-red-400">2 violations</div>
            <div className="text-xs text-slate-500 border-t border-red-700/20 pt-2">
              <div className="flex gap-1"><span className="text-red-400">user_id</span><span className="text-slate-600">missing_required_field</span></div>
              <div className="flex gap-1 mt-1"><span className="text-red-400">amount</span><span className="text-slate-600">range_violation</span></div>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}

function AccountIllustration() {
  return (
    <div className="w-full rounded-xl border border-[#1f2937] bg-[#0d1117] p-5 space-y-3 select-none pointer-events-none">
      <div className="text-xs text-slate-500 mb-2">API Keys</div>
      {[["Production S3 connector", "cg_live_a4f2", "2 hours ago"], ["Staging pipeline", "cg_live_b91c", "3 days ago"]].map(([name, prefix, used]) => (
        <div key={name as string} className="flex items-center gap-3 bg-[#111827] border border-[#1f2937] rounded-lg px-4 py-3">
          <div className="flex-1">
            <div className="text-sm text-slate-300">{name}</div>
            <div className="text-xs text-slate-600 mt-0.5"><code>{prefix}…</code> · Last used {used}</div>
          </div>
          <span className="text-xs text-green-400 bg-green-900/30 border border-green-700/30 px-2 py-0.5 rounded">active</span>
        </div>
      ))}
      <div className="border border-dashed border-[#374151] rounded-lg px-4 py-3 text-center text-xs text-slate-600">
        + New API key
      </div>
    </div>
  );
}

// ── AuthGate ──────────────────────────────────────────────────────────────────

interface AuthGateProps {
  page: keyof typeof PREVIEWS;
  children: React.ReactNode;
}

export default function AuthGate({ page, children }: AuthGateProps) {
  // Demo mode: no auth wall — render real content immediately.
  if (DEMO_MODE) return <>{children}</>;

  const [authed, setAuthed] = useState<boolean | null>(null);

  useEffect(() => {
    const supabase = createClient();
    supabase.auth.getUser().then(({ data: { user } }) => {
      setAuthed(!!user);
    });
  }, []);

  // Loading — render nothing to avoid flash
  if (authed === null) return (
    <div className="flex items-center justify-center h-64">
      <div className="w-5 h-5 border-2 border-green-600 border-t-transparent rounded-full animate-spin" />
    </div>
  );

  // Authenticated — show real content
  if (authed) return <>{children}</>;

  // Unauthenticated — show compelling preview
  const preview = PREVIEWS[page];
  return (
    <div className="max-w-2xl mx-auto py-16 px-4">
      {/* Illustration */}
      <div className="mb-8 opacity-75">
        {preview.illustration}
      </div>

      {/* Copy */}
      <div className="text-center">
        <h2 className="text-2xl font-bold text-slate-100 mb-3">{preview.title}</h2>
        <p className="text-slate-400 leading-relaxed mb-8 max-w-md mx-auto">
          {preview.description}
        </p>
        <div className="flex gap-3 justify-center">
          <Link
            href="/auth/signup"
            className="px-6 py-2.5 bg-green-600 hover:bg-green-500 text-white rounded-lg text-sm font-medium transition-colors"
          >
            {preview.cta} →
          </Link>
          <Link
            href="/auth/login"
            className="px-6 py-2.5 bg-[#1f2937] hover:bg-[#374151] text-slate-300 rounded-lg text-sm font-medium transition-colors border border-[#374151]"
          >
            Sign in
          </Link>
        </div>
        <p className="mt-5 text-xs text-slate-600">
          No credit card required · Free to start ·{" "}
          <Link href="/stream-demo" className="text-green-600 hover:text-green-500">
            Try the live demo first →
          </Link>
        </p>
      </div>
    </div>
  );
}
