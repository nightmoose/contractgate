"use client";

import { useState, useEffect, useMemo } from "react";
import useSWR from "swr";
import yaml from "js-yaml";
import { playgroundValidate, listContracts, getLatestStableVersion, listVersions, getVersion } from "@/lib/api";
import type { PlaygroundResponse, ContractSummary, Violation } from "@/lib/api";
import clsx from "clsx";
import AuthGate from "@/components/AuthGate";

const DEFAULT_YAML = `version: "1.0"
name: "user_events"
description: "Quick test contract"

ontology:
  entities:
    - name: user_id
      type: string
      required: true
      pattern: "^[a-zA-Z0-9_-]{3,64}$"
    - name: event_type
      type: string
      required: true
      enum: ["click", "view", "purchase"]
    - name: timestamp
      type: integer
      required: true
      min: 0
    - name: amount
      type: number
      required: false
      min: 0

glossary:
  - field: "user_id"
    description: "Unique user identifier"
  - field: "amount"
    description: "Monetary value in USD"
    constraints: "must be non-negative"

metrics:
  - name: "total_revenue"
    formula: "sum(amount) where event_type = 'purchase'"
`;

const DEFAULT_EVENT = `{
  "user_id": "alice_01",
  "event_type": "click",
  "timestamp": 1712000000
}`;

// ---------------------------------------------------------------------------
// Types from YAML parse
// ---------------------------------------------------------------------------

interface ParsedEntity {
  name: string;
  type?: string;
  required?: boolean;
  pattern?: string;
  enum?: string[];
  min?: number;
  max?: number;
  min_length?: number;
  max_length?: number;
}

interface ParsedGlossaryEntry {
  field: string;
  description?: string;
  constraints?: string;
}

interface ParsedMetric {
  name: string;
  formula?: string;
}

interface ParsedContract {
  name?: string;
  version?: string;
  description?: string;
  entities: ParsedEntity[];
  glossary: ParsedGlossaryEntry[];
  metrics: ParsedMetric[];
  error: string | null;
}

function parseContractRules(yamlStr: string): ParsedContract {
  try {
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const parsed = yaml.load(yamlStr) as any;
    return {
      name: parsed?.name,
      version: parsed?.version,
      description: parsed?.description,
      entities: parsed?.ontology?.entities ?? [],
      glossary: parsed?.glossary ?? [],
      metrics: parsed?.metrics ?? [],
      error: null,
    };
  } catch (e) {
    return { entities: [], glossary: [], metrics: [], error: String(e) };
  }
}

// ---------------------------------------------------------------------------
// Contract Rules panel
// ---------------------------------------------------------------------------

function EntityConstraints({ entity }: { entity: ParsedEntity }) {
  const constraints: string[] = [];
  if (entity.required) constraints.push("required");
  if (entity.pattern) constraints.push(`pattern: ${entity.pattern}`);
  if (entity.enum) constraints.push(`enum: [${entity.enum.join(", ")}]`);
  if (entity.min !== undefined) constraints.push(`min: ${entity.min}`);
  if (entity.max !== undefined) constraints.push(`max: ${entity.max}`);
  if (entity.min_length !== undefined) constraints.push(`min_length: ${entity.min_length}`);
  if (entity.max_length !== undefined) constraints.push(`max_length: ${entity.max_length}`);
  return (
    <span className="text-xs text-slate-500">
      {constraints.length > 0 ? constraints.join(" · ") : "no constraints"}
    </span>
  );
}

function ContractRulesPanel({
  contract,
  violations,
  validated,
}: {
  contract: ParsedContract;
  violations: Violation[];
  validated: boolean;
}) {
  const [open, setOpen] = useState(true);

  if (contract.error || (contract.entities.length === 0 && contract.glossary.length === 0 && contract.metrics.length === 0)) {
    return null;
  }

  const violatedFields = new Set(violations.map((v) => v.field));

  return (
    <div className="bg-[#0a0d12] border border-[#1f2937] rounded-xl overflow-hidden">
      <button
        onClick={() => setOpen((x) => !x)}
        className="w-full flex items-center justify-between px-4 py-3 text-left hover:bg-[#1f2937]/20 transition-colors"
      >
        <span className="text-xs text-slate-400 uppercase tracking-wider font-semibold">
          Contract Rules
          {contract.name && (
            <span className="ml-2 text-slate-600 normal-case font-normal">
              {contract.name} {contract.version ? `v${contract.version}` : ""}
            </span>
          )}
        </span>
        <span className="text-slate-600 text-xs">{open ? "▲" : "▼"}</span>
      </button>

      {open && (
        <div className="px-4 pb-4 space-y-4">
          {/* Entities */}
          {contract.entities.length > 0 && (
            <div>
              <p className="text-xs text-slate-600 uppercase tracking-wider mb-2">
                Ontology · {contract.entities.length} field{contract.entities.length !== 1 ? "s" : ""}
              </p>
              <ul className="space-y-1.5">
                {contract.entities.map((e) => {
                  const failed = validated && violatedFields.has(e.name);
                  const passed = validated && !violatedFields.has(e.name);
                  return (
                    <li
                      key={e.name}
                      className={clsx(
                        "flex items-start gap-2 rounded-lg px-3 py-2 text-xs",
                        failed
                          ? "bg-red-900/20 border border-red-800/30"
                          : passed
                          ? "bg-green-900/10 border border-green-900/20"
                          : "bg-[#1f2937]/40"
                      )}
                    >
                      <span className="text-base leading-none mt-0.5">
                        {failed ? "❌" : passed ? "✅" : "·"}
                      </span>
                      <div className="flex-1 min-w-0">
                        <span className="font-mono text-slate-200">{e.name}</span>
                        {e.type && (
                          <span className="ml-2 text-indigo-400">{e.type}</span>
                        )}
                        <div className="mt-0.5">
                          <EntityConstraints entity={e} />
                        </div>
                      </div>
                    </li>
                  );
                })}
              </ul>
            </div>
          )}

          {/* Glossary */}
          {contract.glossary.length > 0 && (
            <div>
              <p className="text-xs text-slate-600 uppercase tracking-wider mb-2">
                Glossary · {contract.glossary.length} definition{contract.glossary.length !== 1 ? "s" : ""}
              </p>
              <ul className="space-y-1">
                {contract.glossary.map((g) => (
                  <li key={g.field} className="text-xs text-slate-500 flex gap-2">
                    <span className="font-mono text-slate-400">{g.field}</span>
                    {g.description && <span>— {g.description}</span>}
                    {g.constraints && (
                      <span className="text-amber-600/80">({g.constraints})</span>
                    )}
                  </li>
                ))}
              </ul>
            </div>
          )}

          {/* Metrics */}
          {contract.metrics.length > 0 && (
            <div>
              <p className="text-xs text-slate-600 uppercase tracking-wider mb-2">
                Metrics · {contract.metrics.length} formula{contract.metrics.length !== 1 ? "s" : ""}
              </p>
              <ul className="space-y-1">
                {contract.metrics.map((m) => (
                  <li key={m.name} className="text-xs flex gap-2 font-mono">
                    <span className="text-violet-400">{m.name}</span>
                    {m.formula && (
                      <span className="text-slate-500">= {m.formula}</span>
                    )}
                  </li>
                ))}
              </ul>
            </div>
          )}
        </div>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Transformed-payload preview panel (RFC-004 "what we stored")
// ---------------------------------------------------------------------------

/**
 * Top-level-key diff between the submitted event and the post-transform
 * payload.  We intentionally stop at the top level: RFC-004 v1 only
 * declares transforms on top-level string fields, so a deeper diff
 * would highlight keys the transform engine never touched.
 *
 * Returns the set of key names that differ (either by JSON-stringified
 * value, or by presence — `drop` transforms remove the key entirely).
 */
function diffTopLevelKeys(before: unknown, after: unknown): Set<string> {
  const changed = new Set<string>();
  const b = (before && typeof before === "object" ? before : {}) as Record<string, unknown>;
  const a = (after && typeof after === "object" ? after : {}) as Record<string, unknown>;
  const keys = new Set([...Object.keys(b), ...Object.keys(a)]);
  for (const k of keys) {
    const bv = JSON.stringify(b[k]);
    const av = JSON.stringify(a[k]);
    if (bv !== av) changed.add(k);
  }
  return changed;
}

/**
 * Render a JSON object with specific top-level keys highlighted.
 * Falls back to a plain `<pre>` dump for non-object roots.
 */
function HighlightedJson({
  value,
  changedKeys,
  highlightColor,
}: {
  value: unknown;
  changedKeys: Set<string>;
  /** Tailwind class applied to the value span when its key is in `changedKeys` */
  highlightColor: string;
}) {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    return (
      <pre className="text-xs text-slate-300 font-mono whitespace-pre-wrap break-all">
        {JSON.stringify(value, null, 2)}
      </pre>
    );
  }
  const entries = Object.entries(value as Record<string, unknown>);
  return (
    <pre className="text-xs font-mono whitespace-pre-wrap break-all leading-relaxed">
      <span className="text-slate-500">{"{"}</span>
      {entries.map(([k, v], i) => {
        const isChanged = changedKeys.has(k);
        return (
          <div key={k} className="pl-4">
            <span className={clsx("text-slate-300", isChanged && "font-semibold")}>
              &quot;{k}&quot;
            </span>
            <span className="text-slate-500">: </span>
            <span
              className={clsx(
                isChanged ? highlightColor : "text-slate-300",
                isChanged && "bg-opacity-10 rounded px-1"
              )}
            >
              {v === undefined ? (
                <em className="text-slate-600">(removed)</em>
              ) : (
                JSON.stringify(v)
              )}
            </span>
            {i < entries.length - 1 && <span className="text-slate-500">,</span>}
          </div>
        );
      })}
      <span className="text-slate-500">{"}"}</span>
    </pre>
  );
}

function TransformPreviewPanel({
  requestBody,
  transformed,
}: {
  requestBody: unknown;
  transformed: unknown;
}) {
  // Defensive: if the backend didn't return a post-transform payload (e.g.
  // the Rust deploy is behind the dashboard deploy and doesn't have the
  // RFC-004 Playground handler yet), hide the panel entirely rather than
  // rendering a misleading "every field was scrubbed" diff.  Only render
  // when `transformed` is a real non-null object or array.
  const hasTransformedPayload =
    transformed !== null &&
    transformed !== undefined &&
    typeof transformed === "object";
  const changedKeys = useMemo(
    () => (hasTransformedPayload ? diffTopLevelKeys(requestBody, transformed) : new Set<string>()),
    [requestBody, transformed, hasTransformedPayload]
  );
  const diverged = changedKeys.size > 0;

  // For the "removed by drop" story, overlay the missing keys into the
  // transformed column as `undefined` so the reader sees them greyed out
  // in place.  We do NOT mutate the transformed object — we render a
  // display copy that carries the sentinel.
  const transformedDisplay = useMemo(() => {
    if (!transformed || typeof transformed !== "object" || Array.isArray(transformed)) {
      return transformed;
    }
    const before = (requestBody && typeof requestBody === "object" && !Array.isArray(requestBody)
      ? (requestBody as Record<string, unknown>)
      : {});
    const after = { ...(transformed as Record<string, unknown>) };
    for (const k of Object.keys(before)) {
      if (!(k in after)) after[k] = undefined;
    }
    return after;
  }, [requestBody, transformed]);

  // Field missing from the response — the server we're talking to doesn't
  // know about RFC-004 yet.  Show a muted "preview unavailable" card
  // instead of lying about what got scrubbed.
  if (!hasTransformedPayload) {
    return (
      <div className="bg-[#111827] border border-[#1f2937] rounded-xl p-6">
        <h2 className="text-sm font-semibold text-slate-400 uppercase tracking-wider mb-2">
          Transformed Payload · what we&apos;d store
        </h2>
        <p className="text-xs text-slate-500 leading-relaxed">
          The backend didn&apos;t return a post-transform payload for this
          request. This usually means the server is on a build that predates
          the RFC-004 Playground handler — redeploy the Rust service to surface
          the diff here.
        </p>
      </div>
    );
  }

  return (
    <div className="bg-[#111827] border border-[#1f2937] rounded-xl p-6">
      <div className="flex items-center justify-between mb-4">
        <h2 className="text-sm font-semibold text-slate-400 uppercase tracking-wider">
          Transformed Payload · what we&apos;d store
        </h2>
        {diverged ? (
          <span className="inline-flex items-center gap-1.5 text-xs bg-indigo-900/40 text-indigo-300 border border-indigo-700/40 rounded-full px-2.5 py-1 font-medium">
            <span>🔒</span>
            <span>PII scrubbed · {changedKeys.size} field{changedKeys.size !== 1 ? "s" : ""}</span>
          </span>
        ) : (
          <span className="inline-flex items-center gap-1.5 text-xs bg-slate-800/60 text-slate-400 border border-slate-700/40 rounded-full px-2.5 py-1">
            <span>=</span>
            <span>Identical — no transforms triggered</span>
          </span>
        )}
      </div>

      <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
        <div>
          <p className="text-[10px] text-slate-500 uppercase tracking-wider mb-2">
            Request body
          </p>
          <div className="bg-[#0a0d12] border border-[#1f2937] rounded-lg p-3 overflow-x-auto">
            <HighlightedJson
              value={requestBody}
              changedKeys={changedKeys}
              highlightColor="text-rose-300 bg-rose-900"
            />
          </div>
        </div>
        <div>
          <p className="text-[10px] text-slate-500 uppercase tracking-wider mb-2">
            After transforms
          </p>
          <div className="bg-[#0a0d12] border border-[#1f2937] rounded-lg p-3 overflow-x-auto">
            <HighlightedJson
              value={transformedDisplay}
              changedKeys={changedKeys}
              highlightColor="text-emerald-300 bg-emerald-900"
            />
          </div>
        </div>
      </div>

      {diverged && (
        <p className="text-[11px] text-slate-500 mt-3 leading-relaxed">
          Preview only — the Playground uses an empty salt, so <code>hash</code> and
          format-preserving <code>mask</code> outputs shown here will not match
          production ingest (which keys on each contract&apos;s <code>pii_salt</code>).
          Shape, drops, and redactions are exact.
        </p>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Page
// ---------------------------------------------------------------------------

function PlaygroundContent() {
  const [yaml_, setYaml] = useState(DEFAULT_YAML);
  const [eventJson, setEventJson] = useState(DEFAULT_EVENT);
  const [atomic, setAtomic] = useState(false);
  const [result, setResult] = useState<PlaygroundResponse | null>(null);
  /**
   * Snapshot of the event body that produced `result`, captured at submit
   * time.  Rendering the "Transformed Payload" diff against this (rather
   * than against live `eventJson`) avoids misleading diffs when the user
   * keeps typing in the textarea after clicking Validate.
   */
  const [submittedEvent, setSubmittedEvent] = useState<unknown>(null);
  const [loading, setLoading] = useState(false);
  const [loadingContract, setLoadingContract] = useState(false);
  const [parseError, setParseError] = useState<string | null>(null);
  const [prefilledFrom, setPrefilledFrom] = useState<string | null>(null);

  // Parse contract rules from the YAML editor (live, debounced by React render)
  const parsedContract = useMemo(() => parseContractRules(yaml_), [yaml_]);

  // Load stored contracts for the "Load from store" dropdown
  const { data: contracts } = useSWR<ContractSummary[]>("contracts", listContracts);

  // On mount: check if we were sent here from the Contracts page via "Test in Playground"
  useEffect(() => {
    const storedYaml = sessionStorage.getItem("playground_yaml");
    const storedId   = sessionStorage.getItem("playground_contract_id");
    if (storedYaml) {
      setYaml(storedYaml);
      setResult(null);
      setSubmittedEvent(null);
      if (storedId) setPrefilledFrom(storedId);
      sessionStorage.removeItem("playground_yaml");
      sessionStorage.removeItem("playground_contract_id");
    }
  }, []);

  const handleLoadContract = async (id: string) => {
    if (!id) return;
    setLoadingContract(true);
    setParseError(null);
    try {
      // Prefer the latest stable version — that's what ingest would use for
      // unpinned traffic.  If the contract has no stable yet (everything is
      // still draft), fall back to the newest draft so the dropdown still
      // loads something sensible.
      let yaml_content: string | null = null;
      try {
        const v = await getLatestStableVersion(id);
        yaml_content = v.yaml_content;
      } catch {
        const versions = await listVersions(id);
        if (versions.length === 0) {
          throw new Error("Contract has no versions");
        }
        const newest = [...versions].sort((a, b) =>
          b.created_at.localeCompare(a.created_at)
        )[0];
        const v = await getVersion(id, newest.version);
        yaml_content = v.yaml_content;
      }
      setYaml(yaml_content);
      setResult(null);
      setSubmittedEvent(null);
    } catch (e: unknown) {
      setParseError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoadingContract(false);
    }
  };

  const handleValidate = async () => {
    setParseError(null);
    let parsed: unknown;
    try {
      parsed = JSON.parse(eventJson);
    } catch {
      setParseError("Invalid JSON in event field");
      return;
    }

    setLoading(true);
    try {
      const r = await playgroundValidate(yaml_, parsed, { atomic });
      setResult(r);
      setSubmittedEvent(parsed);
    } catch (e: unknown) {
      setParseError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  };

  return (
    <div>
      <div className="mb-8">
        <h1 className="text-2xl font-bold">Playground</h1>
        <p className="text-sm text-slate-500 mt-1">
          Test a contract YAML against a sample JSON event — no ingestion, no storage
        </p>
      </div>

      {/* Pre-fill banner — shown when arriving from Contracts → "Test in Playground" */}
      {prefilledFrom && (
        <div className="mb-6 flex items-center gap-3 bg-indigo-900/20 border border-indigo-700/40 rounded-xl px-4 py-3">
          <span className="text-indigo-400 text-lg">🔗</span>
          <p className="text-sm text-indigo-300 flex-1">
            Contract loaded from your{" "}
            <a href="/contracts" className="underline underline-offset-2 hover:text-indigo-200">
              Contracts page
            </a>
            . Edit the YAML below or validate straight away.
          </p>
          <button
            onClick={() => setPrefilledFrom(null)}
            className="text-indigo-500 hover:text-indigo-300 text-sm transition-colors"
            aria-label="Dismiss"
          >
            ✕
          </button>
        </div>
      )}

      {/* Load from stored contracts */}
      {contracts && contracts.length > 0 && (
        <div className="mb-6 flex items-center gap-3">
          <span className="text-xs text-slate-500 uppercase tracking-wider whitespace-nowrap">
            Load contract:
          </span>
          <select
            onChange={(e) => handleLoadContract(e.target.value)}
            defaultValue=""
            disabled={loadingContract}
            className="bg-[#111827] border border-[#1f2937] text-slate-300 text-sm rounded-lg px-3 py-1.5 outline-none focus:border-green-700 disabled:opacity-50"
          >
            <option value="">— select stored contract —</option>
            {contracts.map((c) => (
              <option key={c.id} value={c.id}>
                {c.name}
                {c.latest_stable_version
                  ? ` · stable v${c.latest_stable_version}`
                  : " · no stable yet"}
              </option>
            ))}
          </select>
          {loadingContract && (
            <span className="text-xs text-slate-500 animate-pulse">Loading…</span>
          )}
        </div>
      )}

      <div className="grid grid-cols-1 xl:grid-cols-2 gap-6">
        {/* Left: inputs */}
        <div className="space-y-4">
          <div>
            <label className="block text-xs text-slate-500 uppercase tracking-wider mb-2">
              Contract YAML
            </label>
            <textarea
              value={yaml_}
              onChange={(e) => { setYaml(e.target.value); setResult(null); setSubmittedEvent(null); }}
              className="w-full h-72 bg-[#0a0d12] text-green-300 font-mono text-xs p-4 rounded-lg border border-[#1f2937] outline-none focus:border-green-700 resize-y"
              spellCheck={false}
            />
          </div>

          <div>
            <label className="block text-xs text-slate-500 uppercase tracking-wider mb-2">
              Event JSON
            </label>
            <textarea
              value={eventJson}
              onChange={(e) => setEventJson(e.target.value)}
              className="w-full h-40 bg-[#0a0d12] text-blue-300 font-mono text-xs p-4 rounded-lg border border-[#1f2937] outline-none focus:border-blue-700 resize-y"
              spellCheck={false}
            />
          </div>

          {parseError && (
            <p className="text-sm text-red-400 bg-red-900/20 border border-red-800/40 rounded p-2">
              {parseError}
            </p>
          )}

          {/* Atomic toggle */}
          <label className="flex items-center gap-3 cursor-pointer select-none group w-fit">
            <div className="relative">
              <input
                type="checkbox"
                className="sr-only"
                checked={atomic}
                onChange={(e) => setAtomic(e.target.checked)}
              />
              <div
                className={clsx(
                  "w-9 h-5 rounded-full transition-colors",
                  atomic ? "bg-green-600" : "bg-[#374151]"
                )}
              />
              <div
                className={clsx(
                  "absolute top-0.5 left-0.5 w-4 h-4 rounded-full bg-white shadow transition-transform",
                  atomic && "translate-x-4"
                )}
              />
            </div>
            <div>
              <span className="text-sm font-medium text-slate-300">Atomic</span>
              <span className="ml-2 text-xs text-slate-600">
                fail entire batch on any single violation
              </span>
            </div>
          </label>

          <button
            onClick={handleValidate}
            disabled={loading}
            className="w-full py-3 bg-green-600 hover:bg-green-500 disabled:opacity-50 text-white font-semibold rounded-lg transition-colors"
          >
            {loading ? "Validating…" : "▶  Validate Event"}
          </button>
        </div>

        {/* Right: result + rules panel */}
        <div className="space-y-4">
          {/* Result card */}
          <div className="bg-[#111827] border border-[#1f2937] rounded-xl p-6">
            <h2 className="text-sm font-semibold text-slate-400 uppercase tracking-wider mb-4">
              Result
            </h2>

            {result ? (
              <div>
                {/* Pass / fail banner */}
                <div
                  className={clsx(
                    "flex items-center gap-3 rounded-lg p-4 mb-6",
                    result.passed
                      ? "bg-green-900/30 border border-green-700/50"
                      : "bg-red-900/30 border border-red-700/50"
                  )}
                >
                  <span className="text-2xl">{result.passed ? "✅" : "❌"}</span>
                  <div>
                    <p
                      className={clsx(
                        "text-lg font-bold",
                        result.passed ? "text-green-400" : "text-red-400"
                      )}
                    >
                      {result.passed ? "PASSED" : "FAILED"}
                    </p>
                    <p className="text-xs text-slate-500">
                      Validated in {result.validation_us}µs
                      {result.violations.length > 0 &&
                        ` — ${result.violations.length} violation${result.violations.length > 1 ? "s" : ""}`}
                    </p>
                  </div>
                </div>

                {/* Violations list */}
                {result.violations.length > 0 && (
                  <div>
                    <h3 className="text-xs text-slate-500 uppercase tracking-wider mb-3">
                      Violations
                    </h3>
                    <ul className="space-y-2">
                      {result.violations.map((v, i) => (
                        <li
                          key={i}
                          className="bg-red-900/20 border border-red-800/30 rounded-lg p-3"
                        >
                          <div className="flex items-center gap-2 mb-1">
                            <span className="text-xs bg-red-900/50 text-red-400 px-2 py-0.5 rounded font-mono">
                              {v.kind}
                            </span>
                            <span className="text-xs text-slate-400 font-mono">
                              {v.field}
                            </span>
                          </div>
                          <p className="text-sm text-slate-300">{v.message}</p>
                        </li>
                      ))}
                    </ul>
                  </div>
                )}
              </div>
            ) : (
              <div className="flex flex-col items-center justify-center h-32 text-slate-600">
                <p className="text-4xl mb-3">🧪</p>
                <p className="text-sm text-center">
                  Enter a contract and event, then click Validate
                </p>
              </div>
            )}
          </div>

          {/* Transformed-payload preview — only after a validate call */}
          {result && submittedEvent !== null && (
            <TransformPreviewPanel
              requestBody={submittedEvent}
              transformed={result.transformed_event}
            />
          )}

          {/* Contract Rules panel — parses YAML live */}
          <ContractRulesPanel
            contract={parsedContract}
            violations={result?.violations ?? []}
            validated={result !== null}
          />
        </div>
      </div>
    </div>
  );
}

export default function PlaygroundPage() {
  return (
    <AuthGate page="playground">
      <PlaygroundContent />
    </AuthGate>
  );
}
