"use client";

/**
 * Shared preview content for AuthGate (logged-out) and PlanGate (free tier).
 *
 * Each entry has:
 *   - title / description: copy shown below the illustration
 *   - cta: text used by AuthGate's "sign in" button
 *   - illustration: mock UI screenshot (no interactions, pointer-events-none)
 *
 * PlanGate reuses the same illustration + description but swaps the CTA for
 * an "Upgrade to Growth" button.
 */

export interface PreviewEntry {
  title: string;
  description: string;
  /** AuthGate sign-in button label */
  cta: string;
  illustration: React.ReactNode;
}

// ---------------------------------------------------------------------------
// Illustrations
// ---------------------------------------------------------------------------

export function DashboardIllustration() {
  return (
    <div className="w-full rounded-xl border border-[#1f2937] bg-[#0d1117] p-5 space-y-4 select-none pointer-events-none">
      <div className="grid grid-cols-3 gap-3">
        {[["Total Events", "2,847,391", "text-slate-300"], ["Pass Rate", "94.2%", "text-green-400"], ["p99 Latency", "8ms", "text-green-400"]].map(([label, val, color]) => (
          <div key={label} className="bg-[#111827] border border-[#1f2937] rounded-lg p-3">
            <div className="text-xs text-slate-500 mb-1">{label}</div>
            <div className={`text-xl font-bold ${color}`}>{val}</div>
          </div>
        ))}
      </div>
      <div className="bg-[#111827] border border-[#1f2937] rounded-lg p-3">
        <div className="text-xs text-slate-500 mb-3">Validation rate (last 24h)</div>
        <div className="flex items-end gap-1 h-12">
          {[60,75,55,80,90,70,85,92,88,78,95,82,76,90,94,88,91,85,93,87,96,90,94,92].map((h, i) => (
            <div key={i} className="flex-1 bg-green-900/50 rounded-sm" style={{ height: `${h}%` }} />
          ))}
        </div>
      </div>
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

export function ContractsIllustration() {
  return (
    <div className="w-full rounded-xl border border-[#1f2937] bg-[#0d1117] p-5 select-none pointer-events-none">
      <div className="flex gap-3">
        <div className="w-2/5 space-y-2">
          {[["user_events", "v1.2.0", true], ["order_pipeline", "v2.0.1", false], ["ml_features", "v1.0.0", false]].map(([name, ver, active]) => (
            <div key={name as string} className={`rounded-lg border p-2.5 text-xs ${active ? "border-green-700/50 bg-green-900/20" : "border-[#1f2937] bg-[#111827]"}`}>
              <div className={`font-medium ${active ? "text-green-400" : "text-slate-300"}`}>{name}</div>
              <div className="text-slate-600 mt-0.5">{ver} · stable</div>
            </div>
          ))}
        </div>
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

export function AuditIllustration() {
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
        <div key={i} className="grid grid-cols-4 text-xs px-4 py-2 border-b border-[#1f2937]/50">
          <span className="text-slate-600 font-mono">{r.time}</span>
          <span className="text-slate-400">{r.contract}</span>
          <span className={r.passed ? "text-green-400" : "text-red-400"}>{r.passed ? "✓ pass" : "✗ fail"}</span>
          <span className="text-slate-600">{r.ms}</span>
        </div>
      ))}
    </div>
  );
}

export function PlaygroundIllustration() {
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

export function ScaffoldIllustration() {
  return (
    <div className="w-full rounded-xl border border-[#1f2937] bg-[#0d1117] p-5 space-y-3 select-none pointer-events-none font-mono text-xs">
      <div className="flex gap-2 mb-2">
        {["json", "ndjson", "avro_schema", "proto"].map((fmt, i) => (
          <span key={fmt} className={`px-2 py-0.5 rounded text-xs border ${i === 0 ? "border-green-700/50 bg-green-900/30 text-green-400" : "border-[#1f2937] text-slate-600"}`}>{fmt}</span>
        ))}
      </div>
      <div className="bg-[#111827] border border-[#1f2937] rounded-lg p-3 space-y-0.5 leading-relaxed">
        <div className="text-slate-500">version: <span className="text-green-400">&quot;1.0&quot;</span></div>
        <div className="text-slate-500">name: <span className="text-blue-400">user_events</span></div>
        <div className="text-slate-500">ontology:</div>
        <div className="text-slate-500 pl-2">entities:</div>
        <div className="text-red-400 pl-4">{`- name: email  # scaffold: pii_candidate confidence=0.95`}</div>
        <div className="text-slate-500 pl-6">type: string</div>
        <div className="text-orange-400 pl-6">{`  # TODO: apply transform: hash`}</div>
        <div className="text-green-400 pl-4">{`- name: user_id`}</div>
        <div className="text-slate-500 pl-6">type: string</div>
        <div className="text-slate-600 pl-6">{`  # scaffold: seen_in=100% samples`}</div>
      </div>
    </div>
  );
}

export function AccountIllustration() {
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

export function ScorecardIllustration() {
  const providers = [
    { name: "payments-svc", pass: 97.4, quarantine: 2.6, drift: false },
    { name: "user-events", pass: 84.1, quarantine: 15.9, drift: true },
    { name: "ml-features", pass: 99.1, quarantine: 0.9, drift: false },
  ];
  return (
    <div className="w-full rounded-xl border border-[#1f2937] bg-[#0d1117] p-5 space-y-4 select-none pointer-events-none">
      {/* Summary strip */}
      <div className="grid grid-cols-3 gap-3">
        {[["Providers tracked", "3", "text-slate-300"], ["Avg pass rate", "93.5%", "text-green-400"], ["Active drift signals", "1", "text-amber-400"]].map(([label, val, color]) => (
          <div key={label} className="bg-[#111827] border border-[#1f2937] rounded-lg p-3">
            <div className="text-xs text-slate-500 mb-1">{label}</div>
            <div className={`text-xl font-bold ${color}`}>{val}</div>
          </div>
        ))}
      </div>
      {/* Provider rows */}
      <div className="bg-[#111827] border border-[#1f2937] rounded-xl overflow-hidden">
        <div className="grid grid-cols-4 text-xs text-slate-600 px-4 py-2 border-b border-[#1f2937]">
          <span>Provider</span><span>Pass rate</span><span>Quarantine</span><span>Drift</span>
        </div>
        {providers.map((p) => (
          <div key={p.name} className="grid grid-cols-4 text-xs px-4 py-2.5 border-b border-[#1f2937]/50 items-center">
            <span className="text-slate-300 font-medium">{p.name}</span>
            <span className="text-green-400 font-mono">{p.pass}%</span>
            <span className={p.quarantine > 5 ? "text-red-400 font-mono" : "text-slate-500 font-mono"}>{p.quarantine}%</span>
            {p.drift
              ? <span className="text-amber-400 text-[10px] font-medium">⚠ enum drift</span>
              : <span className="text-slate-700">—</span>}
          </div>
        ))}
      </div>
    </div>
  );
}

export function CatalogIllustration() {
  return (
    <div className="w-full rounded-xl border border-[#1f2937] bg-[#0d1117] p-5 space-y-4 select-none pointer-events-none">
      {/* Open data entries */}
      <div>
        <div className="text-xs text-slate-500 mb-2 uppercase tracking-wider">Open Data Contracts</div>
        <div className="divide-y divide-[#1f2937] border border-[#1f2937] rounded-xl overflow-hidden">
          {[["US Census ACS 5-Year", "ODCS"], ["OpenStreetMap POIs", "JSON"], ["NOAA Weather Events", "CSV"]].map(([name, fmt]) => (
            <div key={name} className="flex items-center justify-between px-4 py-2.5">
              <div>
                <p className="text-sm text-slate-300">{name}</p>
                <p className="text-xs text-slate-600 font-mono">{fmt}</p>
              </div>
              <span className="text-xs text-teal-500">Fork →</span>
            </div>
          ))}
        </div>
      </div>
      {/* Egress validator stub */}
      <div>
        <div className="text-xs text-slate-500 mb-2 uppercase tracking-wider">Egress Validator</div>
        <div className="bg-[#111827] border border-[#1f2937] rounded-xl p-3 space-y-2">
          <div className="flex gap-2 text-xs">
            <span className="text-slate-600">Contract:</span>
            <span className="text-slate-300">user_events</span>
            <span className="text-slate-600 ml-auto">disposition: block</span>
          </div>
          <div className="grid grid-cols-4 gap-2 pt-1">
            {[["Total", "12", "text-white"], ["Passed", "10", "text-green-400"], ["Failed", "2", "text-red-400"], ["Latency", "6µs", "text-slate-400"]].map(([l, v, c]) => (
              <div key={l} className="bg-[#0d1117] rounded-lg px-2 py-1.5 text-center">
                <div className="text-[10px] text-slate-600">{l}</div>
                <div className={`text-sm font-bold ${c}`}>{v}</div>
              </div>
            ))}
          </div>
        </div>
      </div>
    </div>
  );
}

export function WorkbenchIllustration() {
  const endpoints = [
    { method: "GET",  path: "/users/{id}",   conf: 92 },
    { method: "POST", path: "/events",        conf: 78 },
    { method: "GET",  path: "/orders",        conf: 55 },
  ];
  const fields = [
    { name: "user_id",    type: "string",  conf: 95, req: true  },
    { name: "email",      type: "string",  conf: 88, req: true  },
    { name: "created_at", type: "string",  conf: 90, req: false },
    { name: "amount",     type: "number",  conf: 52, req: false },
  ];
  const methodColor: Record<string, string> = {
    GET:  "bg-green-900/40 text-green-400",
    POST: "bg-blue-900/40 text-blue-400",
  };
  return (
    <div className="w-full rounded-xl border border-[#1f2937] bg-[#0d1117] overflow-hidden select-none pointer-events-none text-[11px]">
      {/* Header bar */}
      <div className="px-4 py-2.5 border-b border-[#1f2937] flex items-center gap-3 bg-[#111827]">
        <span className="text-slate-200 font-semibold">API Workbench</span>
        <span className="ml-auto text-[9px] bg-amber-900/30 text-amber-400 border border-amber-700/30 px-2 py-0.5 rounded-full">Try It — 1 endpoint</span>
      </div>
      <div className="flex">
        {/* Endpoint list */}
        <div className="w-36 border-r border-[#1f2937] p-2 space-y-1 bg-[#111827]">
          {endpoints.map(ep => (
            <div key={ep.path} className={`flex items-center gap-1.5 px-2 py-1.5 rounded ${ep.method === "GET" && ep.path === "/users/{id}" ? "bg-green-900/30 border border-green-800/40" : ""}`}>
              <span className={`text-[8px] font-bold px-1 py-0.5 rounded w-8 text-center ${methodColor[ep.method] ?? "bg-slate-700/40 text-slate-400"}`}>{ep.method}</span>
              <span className="font-mono text-slate-500 truncate text-[9px]">{ep.path}</span>
            </div>
          ))}
        </div>
        {/* Right panel */}
        <div className="flex-1 p-3 space-y-2">
          {/* URL bar */}
          <div className="flex items-center gap-2">
            <span className="text-[9px] bg-green-900/40 text-green-400 px-1.5 py-0.5 rounded font-bold">GET</span>
            <span className="flex-1 font-mono text-slate-500 text-[9px] bg-[#111827] border border-[#1f2937] rounded px-2 py-1">https://api.example.com/users/u123</span>
            <span className="text-[9px] bg-green-600 text-white px-2 py-1 rounded font-semibold">Send</span>
          </div>
          {/* Response */}
          <div className="bg-[#111827] border border-[#1f2937] rounded p-2">
            <span className="text-green-400 font-bold text-[9px]">200 </span><span className="text-slate-600 text-[9px]">OK · 42ms</span>
            <pre className="text-[8px] font-mono text-slate-500 mt-1">{`{ "user_id": "u123", "email": "alice@...",\n  "created_at": "2026-05-18T...", "amount": 42.5 }`}</pre>
          </div>
          {/* Inferred fields */}
          <div className="bg-[#111827] border border-[#1f2937] rounded p-2 space-y-1">
            <span className="text-slate-600 text-[9px] uppercase tracking-wider">Inferred schema</span>
            {fields.map(f => (
              <div key={f.name} className="flex items-center gap-1.5">
                <span className="font-mono text-slate-400 w-20 truncate text-[9px]">{f.name}</span>
                <span className="text-[8px] bg-slate-700/50 text-slate-500 px-1 rounded">{f.type}</span>
                <div className="flex-1 h-1 bg-[#0d1117] rounded-full overflow-hidden">
                  <div className={`h-full rounded-full ${f.conf >= 70 ? "bg-green-500" : f.conf >= 40 ? "bg-amber-500" : "bg-red-500"}`} style={{ width: `${f.conf}%` }} />
                </div>
                <span className={`text-[8px] ${f.conf >= 70 ? "text-green-500" : f.conf >= 40 ? "text-amber-500" : "text-red-500"}`}>{f.conf}</span>
              </div>
            ))}
          </div>
        </div>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// PREVIEWS registry
// ---------------------------------------------------------------------------

export const PREVIEWS: Record<string, PreviewEntry> = {
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
  scaffold: {
    title: "Brownfield Contract Scaffolder",
    description: "Drop in a JSON sample, NDJSON stream, Avro schema, or Protobuf definition and get a ready-to-use contract YAML in seconds — complete with PII detection and stat annotations.",
    cta: "Sign in to scaffold a contract",
    illustration: <ScaffoldIllustration />,
  },
  scorecard: {
    title: "Provider Scorecard",
    description: "See per-provider pass and quarantine rates, ranked field violations, and active drift signals. Share objective data-quality evidence with your upstream producers.",
    cta: "Sign in to view scorecards",
    illustration: <ScorecardIllustration />,
  },
  catalog: {
    title: "Contract Catalog & Egress Validator",
    description: "Browse curated open-data contracts, import community schemas by reference, and validate outbound payloads against your contracts before they leave your API.",
    cta: "Sign in to browse the catalog",
    illustration: <CatalogIllustration />,
  },
  workbench: {
    title: "API Workbench",
    description: "Paste a base URL, OpenAPI spec, curl command, or Postman collection — explore endpoints live in your browser, infer a contract schema from real responses, refine fields, and deploy enforcement in one workflow. Credentials never leave your browser.",
    cta: "Sign in to open the Workbench",
    illustration: <WorkbenchIllustration />,
  },
};
