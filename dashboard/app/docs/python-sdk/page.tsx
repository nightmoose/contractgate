"use client";

import { useState } from "react";
import Link from "next/link";

// ── Shared primitives (same design language as kafka-connect page) ────────────

function Code({
  children,
  language = "python",
}: {
  children: string;
  language?: string;
}) {
  const [copied, setCopied] = useState(false);
  const copy = () => {
    navigator.clipboard.writeText(children.trim());
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  };
  return (
    <div className="relative group my-4">
      <pre className="bg-[#0d1117] border border-[#1f2937] rounded-lg p-4 overflow-x-auto text-sm text-slate-300 font-mono leading-relaxed">
        <code>{children.trim()}</code>
      </pre>
      <button
        onClick={copy}
        className="absolute top-3 right-3 px-2 py-1 text-xs rounded bg-[#1f2937] text-slate-400 hover:text-slate-200 border border-[#374151] opacity-0 group-hover:opacity-100 transition-opacity"
      >
        {copied ? "Copied!" : "Copy"}
      </button>
      {language && (
        <span className="absolute top-3 left-4 text-xs text-slate-600 font-mono">
          {language}
        </span>
      )}
    </div>
  );
}

function H2({ id, children }: { id: string; children: React.ReactNode }) {
  return (
    <h2
      id={id}
      className="text-xl font-semibold text-slate-100 mt-12 mb-4 scroll-mt-8 flex items-center gap-2"
    >
      <a href={`#${id}`} className="text-slate-600 hover:text-green-400 text-sm">
        ¶
      </a>
      {children}
    </h2>
  );
}

function H3({ children }: { children: React.ReactNode }) {
  return (
    <h3 className="text-base font-semibold text-slate-200 mt-6 mb-3">
      {children}
    </h3>
  );
}

function Callout({
  kind = "info",
  children,
}: {
  kind?: "info" | "warning" | "tip";
  children: React.ReactNode;
}) {
  const styles = {
    info: "bg-indigo-900/15 border-indigo-700/30 text-indigo-200",
    warning: "bg-amber-900/15 border-amber-700/30 text-amber-200",
    tip: "bg-green-900/15 border-green-700/30 text-green-200",
  };
  const icons = { info: "ℹ", warning: "⚠", tip: "✓" };
  return (
    <div
      className={`flex gap-3 border rounded-lg px-4 py-3 my-4 text-sm leading-relaxed ${styles[kind]}`}
    >
      <span className="mt-0.5 flex-shrink-0">{icons[kind]}</span>
      <div>{children}</div>
    </div>
  );
}

// ── Table components ───────────────────────────────────────────────────────────

function Table({
  headers,
  rows,
}: {
  headers: string[];
  rows: (string | React.ReactNode)[][];
}) {
  return (
    <div className="overflow-x-auto my-4">
      <table className="w-full text-sm border-collapse">
        <thead>
          <tr className="border-b border-[#374151]">
            {headers.map((h) => (
              <th
                key={h}
                className="text-left py-2 pr-4 text-slate-500 font-medium text-xs uppercase tracking-wide"
              >
                {h}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {rows.map((row, i) => (
            <tr key={i} className="border-b border-[#1f2937] hover:bg-[#0d1117]/50">
              {row.map((cell, j) => (
                <td key={j} className="py-3 pr-4 align-top text-slate-400 text-sm">
                  {cell}
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

// ── Tab nav ────────────────────────────────────────────────────────────────────

type Tab = "readme" | "rfc";

function TabNav({
  active,
  onChange,
}: {
  active: Tab;
  onChange: (t: Tab) => void;
}) {
  return (
    <div className="flex gap-1 border-b border-[#1f2937] mb-8">
      {(
        [
          { id: "readme" as Tab, label: "README · Quick Guide" },
          { id: "rfc" as Tab, label: "Design Spec" },
        ] as { id: Tab; label: string }[]
      ).map(({ id, label }) => (
        <button
          key={id}
          onClick={() => onChange(id)}
          className={`px-4 py-2.5 text-sm font-medium border-b-2 transition-colors -mb-px ${
            active === id
              ? "border-green-400 text-green-400"
              : "border-transparent text-slate-500 hover:text-slate-300"
          }`}
        >
          {label}
        </button>
      ))}
    </div>
  );
}

// ── TOC definitions per tab ────────────────────────────────────────────────────

const README_TOC = [
  { id: "install", label: "Installation" },
  { id: "quickstart-http", label: "HTTP Client" },
  { id: "quickstart-async", label: "Async Client" },
  { id: "quickstart-local", label: "Local Validator" },
  { id: "caveats", label: "Caveats" },
];

const RFC_TOC = [
  { id: "summary", label: "Summary" },
  { id: "goals", label: "Goals" },
  { id: "non-goals", label: "Non-goals" },
  { id: "design", label: "Design" },
  { id: "api-surface", label: "API Surface" },
  { id: "error-hierarchy", label: "Error Hierarchy" },
  { id: "type-mapping", label: "Type Mapping" },
  { id: "decisions", label: "Decisions" },
  { id: "test-plan", label: "Test Plan" },
  { id: "rollout", label: "Rollout" },
];

// ── README content ─────────────────────────────────────────────────────────────

function ReadmeContent() {
  return (
    <>
      {/* Installation */}
      <H2 id="install">Installation</H2>
      <p className="text-slate-400 text-sm mb-2">
        Requires <strong className="text-slate-300">Python 3.9+</strong>. Runtime
        dependencies:{" "}
        <code className="text-green-400 text-xs">httpx</code>,{" "}
        <code className="text-green-400 text-xs">PyYAML</code>.
      </p>
      <Code language="bash">pip install contractgate</Code>
      <div className="flex gap-2 mt-2 flex-wrap">
        <span className="text-xs bg-[#1f2937] text-slate-400 border border-[#374151] px-2 py-1 rounded">
          Python 3.9+
        </span>
        <span className="text-xs bg-[#1f2937] text-slate-400 border border-[#374151] px-2 py-1 rounded">
          httpx ≥0.25,&lt;1.0
        </span>
        <span className="text-xs bg-[#1f2937] text-slate-400 border border-[#374151] px-2 py-1 rounded">
          PyYAML
        </span>
        <span className="text-xs bg-green-900/30 text-green-400 border border-green-700/40 px-2 py-1 rounded">
          MIT License
        </span>
      </div>

      {/* HTTP client */}
      <H2 id="quickstart-http">Quick Start — HTTP Client (Sync)</H2>
      <p className="text-slate-400 text-sm mb-2">
        Use <code className="text-green-400">Client</code> to send events to the
        ContractGate gateway and inspect per-event results.
      </p>
      <Code language="python">{`from contractgate import Client

cg = Client(base_url="https://gw.example.com", api_key="cg_live_...")

result = cg.ingest(
    contract_id="11111111-1111-1111-1111-111111111111",
    events=[
        {"user_id": "alice_01", "event_type": "click", "timestamp": 1712000000},
    ],
)

print(result.passed, "/", result.total, "events passed")
for r in result.results:
    if not r.passed:
        for v in r.violations:
            print(v.field, v.kind, v.message)`}</Code>

      {/* Async client */}
      <H2 id="quickstart-async">Quick Start — HTTP Client (Async)</H2>
      <p className="text-slate-400 text-sm mb-2">
        <code className="text-green-400">AsyncClient</code> is the async
        equivalent. It wraps <code className="text-slate-300">httpx.AsyncClient</code>{" "}
        and supports <code className="text-slate-300">async with</code> for
        lifecycle management.
      </p>
      <Code language="python">{`import asyncio
from contractgate import AsyncClient

async def main():
    async with AsyncClient(base_url="...", api_key="...") as cg:
        result = await cg.ingest(contract_id="...", events=[...])

asyncio.run(main())`}</Code>
      <Callout kind="info">
        <code className="text-indigo-300">Client</code> and{" "}
        <code className="text-indigo-300">AsyncClient</code> share the same
        transport layer and response models — they can never drift on header or
        auth shape.
      </Callout>

      {/* Local validator */}
      <H2 id="quickstart-local">Quick Start — Local Validator</H2>
      <p className="text-slate-400 text-sm mb-2">
        A pure-Python port of the Rust validator — no network required. Useful
        in unit tests and pre-commit hooks.
      </p>
      <Code language="python">{`from contractgate import Contract

contract = Contract.from_yaml(open("user_events.yaml").read())
compiled = contract.compile()

vr = compiled.validate({
    "user_id": "alice_01",
    "event_type": "click",
    "timestamp": 1712000000,
})
assert vr.passed, vr.violations`}</Code>
      <Callout kind="warning">
        The local validator does <strong>not</strong> run PII transforms
        (<code className="text-amber-300">mask</code>,{" "}
        <code className="text-amber-300">hash</code>,{" "}
        <code className="text-amber-300">drop</code>,{" "}
        <code className="text-amber-300">redact</code>). It is read-only —
        pass/fail and violations only. The gateway&apos;s{" "}
        <code className="text-amber-300">transformed_event</code> field is the
        authoritative &quot;what got stored&quot; view.
      </Callout>

      {/* Caveats */}
      <H2 id="caveats">Caveats</H2>

      <div className="space-y-4">
        {[
          {
            title: "Local validator does not run PII transforms",
            body: (
              <>
                The per-contract PII salt is server-side only and never returned
                in API responses. The local validator is for{" "}
                <em>assert event_passes_contract(…)</em> in user tests — not for
                replicating the gateway&apos;s storage path. Read{" "}
                <code className="text-slate-300">transformed_event</code> from
                each per-event result for the post-transform payload.
              </>
            ),
          },
          {
            title: "Audit honesty",
            body: (
              <>
                Every per-event result carries the{" "}
                <code className="text-slate-300">contract_version</code> that{" "}
                <em>actually matched</em> the event (relevant under{" "}
                <code className="text-slate-300">
                  multi_stable_resolution: fallback
                </code>
                ). Surface it as-is — do not substitute the requested version.
              </>
            ),
          },
          {
            title: "Retries are off by default",
            body: (
              <>
                Layer{" "}
                <code className="text-slate-300">
                  httpx.HTTPTransport(retries=)
                </code>{" "}
                or <code className="text-slate-300">tenacity</code> if you need
                them. Avoid client-side retry on ingest to prevent double-write
                — use the gateway&apos;s quarantine replay endpoint instead.
              </>
            ),
          },
          {
            title: "httpx pin",
            body: (
              <>
                <code className="text-slate-300">httpx</code> is pinned{" "}
                <code className="text-slate-300">{">=0.25,<1.0"}</code>; the
                upper bound will widen once httpx 1.x ships and is tested.
              </>
            ),
          },
        ].map(({ title, body }) => (
          <div
            key={title}
            className="border border-[#1f2937] rounded-lg p-4 bg-[#0d1117]"
          >
            <div className="text-sm font-semibold text-slate-200 mb-1">
              {title}
            </div>
            <div className="text-sm text-slate-400 leading-relaxed">{body}</div>
          </div>
        ))}
      </div>
    </>
  );
}

// ── RFC content ────────────────────────────────────────────────────────────────

function RFCContent() {
  return (
    <>
      {/* Header badge */}
      <div className="overflow-x-auto my-4">
        <table className="text-sm border-collapse">
          <tbody>
            {[
              ["Status", "Accepted (2026-04-26)"],
              ["Author", "ContractGate team"],
              ["Accepted", "2026-04-26 — Alex sign-off on Q1, Q2, Q4, Q6 (recommendations); Q3, Q5 default"],
              ["Target branch", "nightly-maintenance-2026-04-26"],
              ["Depends on", "Versioning + PII transforms — both landed"],
            ].map(([k, v]) => (
              <tr key={k} className="border-b border-[#1f2937]">
                <td className="py-2 pr-6 text-slate-500 font-medium text-xs uppercase tracking-wide whitespace-nowrap">
                  {k}
                </td>
                <td className="py-2 text-slate-400 text-sm">{v}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>

      {/* Summary */}
      <H2 id="summary">Summary</H2>
      <p className="text-slate-400 text-sm leading-relaxed mb-4">
        Ship a first-party Python SDK at{" "}
        <code className="text-green-400">sdks/python/</code> (PyPI name{" "}
        <code className="text-green-400">contractgate</code>). Three capabilities,
        in adoption order:
      </p>
      <ol className="list-decimal list-inside space-y-2 text-slate-400 text-sm mb-4">
        <li>
          <strong className="text-slate-300">HTTP client for the gateway</strong>{" "}
          — <code className="text-green-400">validate()</code>,{" "}
          <code className="text-green-400">ingest()</code>,{" "}
          <code className="text-green-400">audit()</code>, contract CRUD reads.
          Wraps the existing REST API. Sync + async (httpx).
        </li>
        <li>
          <strong className="text-slate-300">Local validator</strong> — pure-Python
          port of <code className="text-slate-300">src/validation.rs</code>. Parse
          contract YAML, compile once, validate many. No network.
        </li>
        <li>
          <strong className="text-slate-300">Audit log query helper</strong> —
          paginated reads of <code className="text-slate-300">/audit</code>,{" "}
          <code className="text-slate-300">/quarantine</code>, and replay history.
          Iterator + filter ergonomics.
        </li>
      </ol>

      {/* Goals */}
      <H2 id="goals">Goals</H2>
      <ol className="list-decimal list-inside space-y-2 text-slate-400 text-sm">
        <li>First-class sync + async in v0.1; shared transport layer (httpx) and response models.</li>
        <li>
          No external runtime deps beyond{" "}
          <code className="text-slate-300">httpx</code> and{" "}
          <code className="text-slate-300">PyYAML</code>.
        </li>
        <li>Python 3.9+ for broad adoption.</li>
        <li>Validator parity with the Rust engine — same violation kinds, message shape, field-path format, and per-event ordering. Locked via shared JSON fixtures.</li>
        <li>Respect the audit-honesty invariant — surface <code className="text-slate-300">contract_version</code> from the response on each per-event result; never substitute.</li>
        <li>
          Stable error taxonomy:{" "}
          <code className="text-slate-300">ContractGateError</code> →{" "}
          <code className="text-slate-300">HTTPError</code>,{" "}
          <code className="text-slate-300">ValidationError</code>,{" "}
          <code className="text-slate-300">ContractCompileError</code>,{" "}
          <code className="text-slate-300">AuthError</code>.
        </li>
      </ol>

      {/* Non-goals */}
      <H2 id="non-goals">Non-goals (v0.1)</H2>
      <ul className="list-disc list-inside space-y-2 text-slate-400 text-sm">
        <li>PII transform parity in the local validator (read-only by design)</li>
        <li>Auto-retry / circuit breakers</li>
        <li>Streaming ingest / Kafka shim</li>
        <li>A CLI</li>
        <li>Rust-backed validator via PyO3 (deferred)</li>
        <li>Browser / Pyodide compatibility</li>
      </ul>

      {/* Design */}
      <H2 id="design">Design — Package Layout</H2>
      <Code language="text">{`sdks/python/
├── pyproject.toml               # PEP 621 metadata, hatchling build
├── README.md                    # quickstart + parity caveats
├── LICENSE                      # MIT
├── src/
│   └── contractgate/
│       ├── __init__.py          # public re-exports
│       ├── _version.py          # 0.1.0
│       ├── client.py            # sync Client
│       ├── async_client.py      # async AsyncClient
│       ├── _transport.py        # shared httpx wiring (auth, base_url)
│       ├── models.py            # @dataclass response shapes
│       ├── exceptions.py        # error hierarchy
│       ├── contract.py          # YAML → Contract dataclass + compile
│       └── validator.py         # pure-Python validate()
└── tests/
    ├── test_client_sync.py
    ├── test_client_async.py
    ├── test_validator.py
    ├── test_contract_parse.py
    └── fixtures/
        ├── contracts/*.yaml
        └── parity/*.json        # shared with Rust parity tests`}</Code>

      {/* API Surface */}
      <H2 id="api-surface">Public API Surface (v0.1)</H2>
      <Code language="python">{`from contractgate import Client, AsyncClient, Contract, ValidationResult

# 1. HTTP client (sync)
c = Client(base_url="https://gw.example.com", api_key="cg_live_...")
result = c.ingest(contract_id="...", events=[{...}, {...}])
for r in result.results:
    if not r.passed:
        for v in r.violations:
            print(v.field, v.kind, v.message)

# Audit log reads (paginated iterator)
for entry in c.audit(contract_id="...", limit=200):
    ...

# 2. Async equivalent
async with AsyncClient(base_url=..., api_key=...) as ac:
    result = await ac.ingest(contract_id="...", events=[...])

# 3. Local validator
contract = Contract.from_yaml(open("user_events.yaml").read())
compiled = contract.compile()
vr = compiled.validate({"user_id": "alice_01", "event_type": "click", ...})
assert vr.passed, vr.violations`}</Code>

      {/* Error hierarchy */}
      <H2 id="error-hierarchy">Error Hierarchy</H2>
      <Code language="python">{`ContractGateError
├── HTTPError                     # response-attached, has .status, .body
│   ├── BadRequestError           # 400
│   ├── AuthError                 # 401
│   ├── NotFoundError             # 404
│   ├── ConflictError             # 409 (NoStableVersion)
│   ├── ValidationFailedError     # 422 (atomic reject, all-failed batch)
│   └── ServerError               # 5xx
├── ConnectionError               # transport/DNS/timeout
└── ContractCompileError          # local YAML parse / compile failure`}</Code>
      <Callout kind="info">
        Per-event validation <strong>failures</strong> in a 207 Multi-Status
        response do <em>not</em> raise — they are reflected in{" "}
        <code className="text-indigo-300">BatchIngestResponse.results</code>.
        Only whole-batch rejects (422) raise.
      </Callout>

      {/* Type mapping */}
      <H2 id="type-mapping">Rust → Python Type Mapping</H2>
      <Table
        headers={["Rust type", "Python type"]}
        rows={[
          [<code key="a" className="text-green-400 text-xs">Contract</code>, <code key="b" className="text-slate-300 text-xs">Contract (frozen dataclass)</code>],
          [<code key="a" className="text-green-400 text-xs">FieldDefinition</code>, <code key="b" className="text-slate-300 text-xs">FieldDefinition</code>],
          [<code key="a" className="text-green-400 text-xs">FieldType</code>, <code key="b" className="text-slate-300 text-xs">FieldType (Enum)</code>],
          [<code key="a" className="text-green-400 text-xs">MetricDefinition</code>, <code key="b" className="text-slate-300 text-xs">MetricDefinition</code>],
          [<code key="a" className="text-green-400 text-xs">Transform</code>, <code key="b" className="text-slate-300 text-xs">Transform (declared, not run)</code>],
          [<code key="a" className="text-green-400 text-xs">CompiledContract</code>, <code key="b" className="text-slate-300 text-xs">CompiledContract</code>],
          [<code key="a" className="text-green-400 text-xs">ValidationResult</code>, <code key="b" className="text-slate-300 text-xs">ValidationResult</code>],
          [<code key="a" className="text-green-400 text-xs">Violation</code>, <code key="b" className="text-slate-300 text-xs">Violation</code>],
          [<code key="a" className="text-green-400 text-xs">ViolationKind</code>, <code key="b" className="text-slate-300 text-xs">ViolationKind (Enum)</code>],
        ]}
      />

      {/* Decisions */}
      <H2 id="decisions">Decisions (signed off 2026-04-26)</H2>
      <div className="space-y-3">
        {[
          { q: "Q1 — Package name", a: "contractgate. Reserve contractgate-sdk and cg-sdk as redirect packages once the primary name is claimed." },
          { q: "Q2 — Sync/async surface", a: "Separate Client and AsyncClient. Matches httpx's own pattern; avoids runtime-flag ambiguity at call sites." },
          { q: "Q3 — httpx pin", a: "httpx>=0.25,<1.0. Wide enough for adoption; the pre-1.0 caveat lives in the README. Bump to >=1.0 once httpx 1.x ships and is tested." },
          { q: "Q4 — Validator output equivalence", a: "Strict — byte-identical message text and field paths to the Rust validator. Locked via the shared tests/fixtures/parity/ corpus consumed by both pytest and a Rust integration test." },
          { q: "Q5 — License", a: "MIT. Matches the gateway's existing code license intent. Patent-pending status is independent of the SDK's source license." },
          { q: "Q6 — PII transforms in local validator", a: "No. Read-only validator. The per-contract pii_salt is never exposed to clients. The wire response's transformed_event field is what callers should rely on for 'what got stored'." },
        ].map(({ q, a }) => (
          <div key={q} className="border border-[#1f2937] rounded-lg p-4 bg-[#0d1117]">
            <div className="text-sm font-semibold text-green-400 mb-1">{q}</div>
            <div className="text-sm text-slate-400 leading-relaxed">{a}</div>
          </div>
        ))}
      </div>

      {/* Test plan */}
      <H2 id="test-plan">Test Plan (summary)</H2>
      <div className="space-y-4">
        {[
          { section: "I. Contract parse", items: ["Parse canonical YAML — all fields populated", "compliance_mode: true parses; default is false when omitted", "transform: { kind: mask, style: opaque } parses", "Bad regex → ContractCompileError", "Transform on non-string field → ContractCompileError (same wording as Rust)"] },
          { section: "II. Local validator", items: ["Pass: valid event for the canonical contract", "Each ViolationKind triggers (one test per kind)", "Compliance mode: undeclared field → UndeclaredField", "Nested object: user.address.zip violation reports dotted path", "Array items: per-index path tags[3] reports correctly", "Metric range: min/max violation maps to MetricRangeViolation"] },
          { section: "III. Sync HTTP client", items: ["ingest happy path: posts to /ingest/{id}, parses BatchIngestResponse", "401 → AuthError, 422 → ValidationFailedError, 409 → ConflictError, 404 → NotFoundError", "207 Multi-Status: NO exception, both passed and failed events visible", "audit(contract_id=..., limit=...) paginates correctly", "x-api-key and x-org-id headers attached to every request"] },
          { section: "IV. Async HTTP client", items: ["Same matrix as §III against AsyncClient", "async with AsyncClient(...) cleans up the underlying httpx.AsyncClient"] },
          { section: "V. Cross-language parity", items: ["tests/fixtures/parity/*.json — each fixture is {contract_yaml, event, expected_violations}", "Same fixtures consumed by pytest (Python) and tests/parity_python.rs (Rust)", "At least one fixture per ViolationKind; at least one with three violations in a row"] },
          { section: "VI. Build / packaging", items: ["pip install -e sdks/python succeeds on Python 3.9, 3.11, 3.12", "pyproject.toml declares correct deps; pip-audit runs clean"] },
        ].map(({ section, items }) => (
          <div key={section}>
            <H3>{section}</H3>
            <ul className="list-disc list-inside space-y-1 text-slate-400 text-sm">
              {items.map((item) => (
                <li key={item}>{item}</li>
              ))}
            </ul>
          </div>
        ))}
      </div>

      {/* Rollout */}
      <H2 id="rollout">Rollout</H2>
      <ol className="list-decimal list-inside space-y-2 text-slate-400 text-sm">
        {[
          "Land the Python SDK on nightly-maintenance-2026-04-26 once Q1–Q6 are signed off.",
          "Scaffold sdks/python/ with pyproject.toml, package skeleton, README, __init__.py re-exports.",
          "Implement contract.py + validator.py (the canonical-spec port). Unit tests §I + §II.",
          "Implement _transport.py, models.py, exceptions.py, client.py. Unit tests §III + §V (sync side).",
          "Implement async_client.py. Unit tests §IV.",
          "Add tests/parity_python.rs to the Rust crate; wire cargo test to consume the shared fixture directory.",
          "Document the audit-honesty invariant in the SDK README.",
          "Document the PII-transform caveat: the local validator does not run transforms.",
          "Reserve PyPI name in a separate ops task.",
          "Update MAINTENANCE_LOG.md with the rollout entry.",
        ].map((step, i) => (
          <li key={i}>{step}</li>
        ))}
      </ol>
    </>
  );
}

// ── Page ───────────────────────────────────────────────────────────────────────

export default function PythonSDKDocsPage() {
  const [activeTab, setActiveTab] = useState<Tab>("readme");
  const toc = activeTab === "readme" ? README_TOC : RFC_TOC;

  return (
    <div className="flex gap-8 min-h-screen">
      {/* Sticky TOC sidebar */}
      <aside className="hidden xl:block w-52 flex-shrink-0 pt-10">
        <div className="sticky top-8">
          <p className="text-xs font-semibold text-slate-500 uppercase tracking-widest mb-3">
            On this page
          </p>
          <nav className="space-y-1">
            {toc.map(({ id, label }) => (
              <a
                key={id}
                href={`#${id}`}
                className="block text-sm text-slate-500 hover:text-green-400 py-0.5 transition-colors"
              >
                {label}
              </a>
            ))}
          </nav>

          <div className="mt-8 p-3 bg-green-900/20 border border-green-700/30 rounded-lg">
            <p className="text-xs text-green-400 font-medium mb-1">SDK source</p>
            <p className="text-xs text-slate-500 mb-2">
              Lives at <code className="text-slate-400">sdks/python/</code> in
              the ContractGate repo.
            </p>
            <Link
              href="/playground"
              className="block text-center text-xs bg-green-600 hover:bg-green-500 text-white rounded px-3 py-1.5 transition-colors"
            >
              Try Playground →
            </Link>
          </div>
        </div>
      </aside>

      {/* Main content */}
      <main className="flex-1 max-w-3xl py-10 px-2">
        {/* Breadcrumb + header */}
        <div className="mb-8">
          <div className="flex items-center gap-2 text-sm text-slate-500 mb-4">
            <Link href="/docs" className="hover:text-slate-300">
              Docs
            </Link>
            <span>/</span>
            <span className="text-slate-300">Python SDK</span>
          </div>
          <h1 className="text-3xl font-bold text-slate-100 mb-3">
            Python SDK
          </h1>
          <p className="text-slate-400 text-lg leading-relaxed">
            First-party Python SDK for ContractGate — sync &amp; async HTTP
            client, plus a pure-Python local validator for unit tests and
            pre-commit hooks.
          </p>
          <div className="flex gap-3 mt-4 flex-wrap">
            <span className="text-xs bg-[#1f2937] text-slate-400 border border-[#374151] px-2 py-1 rounded">
              Python 3.9+
            </span>
            <span className="text-xs bg-[#1f2937] text-slate-400 border border-[#374151] px-2 py-1 rounded">
              httpx async/sync
            </span>
            <span className="text-xs bg-[#1f2937] text-slate-400 border border-[#374151] px-2 py-1 rounded">
              MIT License
            </span>
            <span className="text-xs bg-green-900/30 text-green-400 border border-green-700/40 px-2 py-1 rounded">
              v0.1.0
            </span>
            <span className="text-xs bg-amber-900/30 text-amber-400 border border-amber-700/40 px-2 py-1 rounded">
              Accepted
            </span>
          </div>
        </div>

        {/* Tab navigation */}
        <TabNav active={activeTab} onChange={setActiveTab} />

        {/* Tab content */}
        {activeTab === "readme" ? <ReadmeContent /> : <RFCContent />}

        {/* Bottom CTA */}
        <div className="mt-16 p-6 bg-green-900/20 border border-green-700/30 rounded-xl text-center">
          <h3 className="text-lg font-semibold text-green-400 mb-2">
            Ready to integrate?
          </h3>
          <p className="text-slate-400 text-sm mb-4">
            Get your API key and contract UUID from the Account page, then{" "}
            <code className="text-slate-300">pip install contractgate</code>.
          </p>
          <div className="flex gap-3 justify-center">
            <Link
              href="/auth/signup"
              className="px-5 py-2 bg-green-600 hover:bg-green-500 text-white rounded-lg text-sm font-medium transition-colors"
            >
              Sign up free →
            </Link>
            <Link
              href="/playground"
              className="px-5 py-2 bg-[#1f2937] hover:bg-[#374151] text-slate-300 rounded-lg text-sm font-medium transition-colors border border-[#374151]"
            >
              Try the Playground
            </Link>
          </div>
        </div>
      </main>
    </div>
  );
}
