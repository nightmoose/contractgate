"use client";

/**
 * Scaffold page — RFC-024: Brownfield Contract Scaffolder.
 *
 * Lets users paste JSON samples, NDJSON, an Avro schema (.avsc), or a
 * Protobuf definition (.proto) and get a draft ContractGate YAML back,
 * with embedded profiler stats and PII TODO annotations highlighted.
 */

import { useState, useRef, useCallback } from "react";
import clsx from "clsx";
import AuthGate from "@/components/AuthGate";
import {
  scaffoldFromSamples,
  scaffoldFromContent,
  createContract,
  type ScaffoldResponse,
  type PiiCandidate,
} from "@/lib/api";

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

type InputMode = "json" | "ndjson" | "avro_schema" | "proto";

const MODE_META: Record<InputMode, { label: string; placeholder: string; ext: string }> = {
  json: {
    label: "JSON samples",
    ext: ".json",
    placeholder: `Paste a JSON array of sample objects:\n\n[\n  { "user_id": "u1", "email": "alice@example.com", "amount": 42.5 },\n  { "user_id": "u2", "email": "bob@example.com", "amount": 9.99 }\n]`,
  },
  ndjson: {
    label: "NDJSON",
    ext: ".ndjson",
    placeholder: `Paste newline-delimited JSON (one object per line):\n\n{"user_id":"u1","event":"click","ts":1700000000}\n{"user_id":"u2","event":"purchase","ts":1700000001,"amount":49.99}`,
  },
  avro_schema: {
    label: "Avro schema (.avsc)",
    ext: ".avsc",
    placeholder: `Paste an Avro schema (JSON):\n\n{\n  "type": "record",\n  "name": "Order",\n  "fields": [\n    { "name": "order_id", "type": "string" },\n    { "name": "email",    "type": ["null","string"] },\n    { "name": "amount",   "type": "double" }\n  ]\n}`,
  },
  proto: {
    label: "Protobuf (.proto)",
    ext: ".proto",
    placeholder: `Paste a proto3 message definition:\n\nsyntax = "proto3";\nmessage UserEvent {\n  string user_id  = 1;\n  string email    = 2;\n  int64  timestamp = 3;\n  double amount   = 4;\n}`,
  },
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function confidenceBadge(c: number) {
  const pct = Math.round(c * 100);
  const color =
    c >= 0.8
      ? "bg-red-900/40 text-red-300 border-red-700/50"
      : c >= 0.6
      ? "bg-orange-900/40 text-orange-300 border-orange-700/50"
      : "bg-yellow-900/40 text-yellow-300 border-yellow-700/50";
  return (
    <span
      className={clsx(
        "inline-flex items-center px-2 py-0.5 rounded-full text-xs font-medium border",
        color
      )}
    >
      {pct}% confidence
    </span>
  );
}

/** Annotate YAML lines that contain scaffold comments for visual highlighting. */
function renderYamlLine(line: string, idx: number) {
  const trimmed = line.trim();
  if (trimmed.startsWith("# scaffold: pii-candidate")) {
    return (
      <span key={idx} className="block text-red-400 bg-red-950/30">
        {line}
      </span>
    );
  }
  if (trimmed.startsWith("# TODO:") || trimmed.startsWith("#   transform:") || trimmed.startsWith("#     kind:")) {
    return (
      <span key={idx} className="block text-orange-400">
        {line}
      </span>
    );
  }
  if (trimmed.startsWith("# scaffold:")) {
    return (
      <span key={idx} className="block text-slate-500">
        {line}
      </span>
    );
  }
  if (trimmed.startsWith("#")) {
    return (
      <span key={idx} className="block text-slate-500">
        {line}
      </span>
    );
  }
  if (trimmed.startsWith("- name:")) {
    return (
      <span key={idx} className="block text-green-300">
        {line}
      </span>
    );
  }
  if (trimmed.startsWith("name:") || trimmed.startsWith("version:") || trimmed.startsWith("description:")) {
    return (
      <span key={idx} className="block text-indigo-300">
        {line}
      </span>
    );
  }
  return (
    <span key={idx} className="block text-slate-200">
      {line}
    </span>
  );
}

// ---------------------------------------------------------------------------
// Main page
// ---------------------------------------------------------------------------

export default function ScaffoldPage() {
  const [mode, setMode] = useState<InputMode>("json");
  const [contractName, setContractName] = useState("");
  const [description, setDescription] = useState("");
  const [content, setContent] = useState("");
  const [fast, setFast] = useState(false);

  const [loading, setLoading] = useState(false);
  const [result, setResult] = useState<ScaffoldResponse | null>(null);
  const [error, setError] = useState<string | null>(null);

  const [pushing, setPushing] = useState(false);
  const [pushed, setPushed] = useState<string | null>(null); // contract id after push

  const fileInputRef = useRef<HTMLInputElement>(null);

  // ── File upload ────────────────────────────────────────────────────────────
  const handleFileUpload = useCallback(
    (e: React.ChangeEvent<HTMLInputElement>) => {
      const file = e.target.files?.[0];
      if (!file) return;

      // Auto-detect mode from extension
      const ext = file.name.split(".").pop()?.toLowerCase() ?? "";
      if (ext === "avsc") setMode("avro_schema");
      else if (ext === "proto") setMode("proto");
      else if (ext === "ndjson") setMode("ndjson");
      else setMode("json");

      // Auto-fill name from filename stem
      if (!contractName) {
        const stem = file.name.replace(/\.[^.]+$/, "").replace(/[-.\s]+/g, "_");
        setContractName(stem);
      }

      const reader = new FileReader();
      reader.onload = (ev) => setContent(ev.target?.result as string ?? "");
      reader.readAsText(file);
    },
    [contractName]
  );

  // ── Scaffold ───────────────────────────────────────────────────────────────
  const handleScaffold = async () => {
    const name = contractName.trim() || "unnamed";
    setLoading(true);
    setError(null);
    setResult(null);
    setPushed(null);

    try {
      let res: ScaffoldResponse;
      if (mode === "json") {
        // Try parsing as JSON array/object first to use the samples path
        try {
          const parsed = JSON.parse(content);
          const samples = Array.isArray(parsed) ? parsed : [parsed];
          res = await scaffoldFromSamples({
            name,
            description: description.trim() || undefined,
            samples,
            fast,
          });
        } catch {
          // Fall back to content path (server will re-parse)
          res = await scaffoldFromContent({
            name,
            description: description.trim() || undefined,
            content,
            format: "json",
            fast,
          });
        }
      } else {
        res = await scaffoldFromContent({
          name,
          description: description.trim() || undefined,
          content,
          format: mode,
          fast,
        });
      }
      setResult(res);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setLoading(false);
    }
  };

  // ── Push to gateway ────────────────────────────────────────────────────────
  const handlePush = async () => {
    if (!result) return;
    setPushing(true);
    try {
      const created = await createContract(result.yaml_content);
      setPushed(created.id);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setPushing(false);
    }
  };

  const canScaffold = content.trim().length > 0;

  return (
    <AuthGate page="scaffold">
      <div className="p-6 max-w-6xl mx-auto space-y-6">
        {/* Header */}
        <div>
          <h1 className="text-2xl font-bold text-white">Contract Scaffolder</h1>
          <p className="mt-1 text-sm text-slate-400">
            Generate a draft contract from JSON samples, NDJSON, an Avro schema, or a Protobuf
            definition. PII candidates are flagged with TODO annotations — never auto-applied.
          </p>
        </div>

        <div className="grid grid-cols-1 xl:grid-cols-2 gap-6">
          {/* ── Left panel: input ── */}
          <div className="space-y-4">
            {/* Contract metadata */}
            <div className="grid grid-cols-2 gap-3">
              <div>
                <label className="block text-xs font-medium text-slate-400 mb-1">
                  Contract name <span className="text-slate-600">(optional)</span>
                </label>
                <input
                  type="text"
                  value={contractName}
                  onChange={(e) => setContractName(e.target.value)}
                  placeholder="e.g. user_events"
                  className="w-full bg-[#0d1117] border border-[#1f2937] rounded-lg px-3 py-2 text-sm text-slate-200 placeholder-slate-600 focus:outline-none focus:border-indigo-500"
                />
              </div>
              <div>
                <label className="block text-xs font-medium text-slate-400 mb-1">
                  Description <span className="text-slate-600">(optional)</span>
                </label>
                <input
                  type="text"
                  value={description}
                  onChange={(e) => setDescription(e.target.value)}
                  placeholder="Short description"
                  className="w-full bg-[#0d1117] border border-[#1f2937] rounded-lg px-3 py-2 text-sm text-slate-200 placeholder-slate-600 focus:outline-none focus:border-indigo-500"
                />
              </div>
            </div>

            {/* Format selector */}
            <div>
              <label className="block text-xs font-medium text-slate-400 mb-2">Input format</label>
              <div className="flex gap-2 flex-wrap">
                {(Object.keys(MODE_META) as InputMode[]).map((m) => (
                  <button
                    key={m}
                    onClick={() => setMode(m)}
                    className={clsx(
                      "px-3 py-1.5 rounded-lg text-xs font-medium border transition-colors",
                      mode === m
                        ? "bg-indigo-900/40 text-indigo-300 border-indigo-700/60"
                        : "bg-[#111827] text-slate-400 border-[#1f2937] hover:border-slate-600"
                    )}
                  >
                    {MODE_META[m].label}
                  </button>
                ))}
              </div>
            </div>

            {/* File upload */}
            <div className="flex items-center gap-3">
              <button
                onClick={() => fileInputRef.current?.click()}
                className="px-3 py-1.5 text-xs font-medium rounded-lg border border-[#1f2937] bg-[#111827] text-slate-400 hover:text-slate-200 hover:border-slate-600 transition-colors"
              >
                Upload file
              </button>
              <span className="text-xs text-slate-600">
                or paste directly below ({MODE_META[mode].ext})
              </span>
              <input
                ref={fileInputRef}
                type="file"
                accept=".json,.ndjson,.avsc,.proto"
                onChange={handleFileUpload}
                className="hidden"
              />
            </div>

            {/* Content textarea */}
            <div>
              <textarea
                value={content}
                onChange={(e) => setContent(e.target.value)}
                placeholder={MODE_META[mode].placeholder}
                rows={18}
                className="w-full bg-[#0d1117] border border-[#1f2937] rounded-lg px-3 py-3 text-xs text-slate-300 font-mono placeholder-slate-700 focus:outline-none focus:border-indigo-500 resize-y"
              />
            </div>

            {/* Options + action */}
            <div className="flex items-center justify-between">
              <label className="flex items-center gap-2 text-xs text-slate-400 cursor-pointer select-none">
                <input
                  type="checkbox"
                  checked={fast}
                  onChange={(e) => setFast(e.target.checked)}
                  className="rounded border-[#1f2937] bg-[#0d1117] text-indigo-500 focus:ring-indigo-500"
                />
                Fast mode (skip value profiling)
              </label>

              <button
                onClick={handleScaffold}
                disabled={!canScaffold || loading}
                className={clsx(
                  "px-5 py-2 rounded-lg text-sm font-semibold transition-colors",
                  canScaffold && !loading
                    ? "bg-indigo-600 hover:bg-indigo-500 text-white"
                    : "bg-[#1f2937] text-slate-600 cursor-not-allowed"
                )}
              >
                {loading ? "Scaffolding…" : "Generate contract"}
              </button>
            </div>

            {/* Error */}
            {error && (
              <div className="rounded-lg border border-red-800/50 bg-red-950/30 px-4 py-3 text-sm text-red-300">
                {error}
              </div>
            )}
          </div>

          {/* ── Right panel: result ── */}
          <div className="space-y-4">
            {result ? (
              <>
                {/* Stats bar */}
                <div className="flex flex-wrap gap-3">
                  <Stat label="Fields" value={result.field_count} />
                  <Stat label="Samples" value={result.sample_count} />
                  <Stat label="Format" value={result.format} />
                  {result.pii_candidate_count > 0 && (
                    <span className="inline-flex items-center gap-1.5 px-3 py-1 rounded-lg bg-red-950/40 border border-red-800/50 text-xs text-red-300 font-medium">
                      ⚠️ {result.pii_candidate_count} PII candidate
                      {result.pii_candidate_count !== 1 ? "s" : ""}
                    </span>
                  )}
                  {result.sr_unavailable && (
                    <span className="inline-flex items-center px-3 py-1 rounded-lg bg-yellow-950/40 border border-yellow-800/50 text-xs text-yellow-300">
                      Schema Registry unavailable
                    </span>
                  )}
                </div>

                {/* PII candidates */}
                {result.pii_candidates.length > 0 && (
                  <div className="rounded-lg border border-red-800/40 bg-red-950/20 p-4 space-y-2">
                    <p className="text-xs font-semibold text-red-300 uppercase tracking-wider">
                      PII candidates — review before promoting
                    </p>
                    {result.pii_candidates.map((c: PiiCandidate) => (
                      <div
                        key={c.field_name}
                        className="flex items-start justify-between gap-4 text-xs"
                      >
                        <div>
                          <span className="font-mono text-red-200">{c.field_name}</span>
                          <span className="text-slate-500 ml-2">→ suggest: </span>
                          <span className="font-mono text-orange-300">{c.suggested_transform}</span>
                          <p className="text-slate-500 mt-0.5">{c.reason}</p>
                        </div>
                        {confidenceBadge(c.confidence)}
                      </div>
                    ))}
                  </div>
                )}

                {/* YAML output */}
                <div className="relative">
                  <div className="flex items-center justify-between mb-1.5">
                    <p className="text-xs font-medium text-slate-400">Generated YAML</p>
                    <div className="flex gap-2">
                      <button
                        onClick={() => navigator.clipboard?.writeText(result.yaml_content)}
                        className="text-xs text-slate-500 hover:text-slate-300 transition-colors"
                      >
                        Copy
                      </button>
                    </div>
                  </div>
                  <pre className="bg-[#0d1117] border border-[#1f2937] rounded-lg p-4 text-xs overflow-auto max-h-[480px] leading-5">
                    {result.yaml_content.split("\n").map((line, i) =>
                      renderYamlLine(line, i)
                    )}
                  </pre>
                </div>

                {/* Push to gateway */}
                {pushed ? (
                  <div className="rounded-lg border border-green-800/50 bg-green-950/30 px-4 py-3 text-sm text-green-300">
                    ✓ Contract created — ID: <span className="font-mono">{pushed}</span>. Go to{" "}
                    <a href="/contracts" className="underline hover:text-green-200">
                      Contracts
                    </a>{" "}
                    to promote it.
                  </div>
                ) : (
                  <button
                    onClick={handlePush}
                    disabled={pushing}
                    className={clsx(
                      "w-full py-2 rounded-lg text-sm font-semibold transition-colors",
                      !pushing
                        ? "bg-green-700 hover:bg-green-600 text-white"
                        : "bg-[#1f2937] text-slate-600 cursor-not-allowed"
                    )}
                  >
                    {pushing ? "Pushing…" : "Push contract to gateway →"}
                  </button>
                )}
              </>
            ) : (
              <div className="flex flex-col items-center justify-center h-full min-h-[480px] rounded-xl border border-dashed border-[#1f2937] text-slate-600">
                <span className="text-5xl mb-4">🏗️</span>
                <p className="text-sm">Paste content on the left and click</p>
                <p className="text-sm font-semibold mt-1">Generate contract</p>
              </div>
            )}
          </div>
        </div>
      </div>
    </AuthGate>
  );
}

// ---------------------------------------------------------------------------
// Small stat chip
// ---------------------------------------------------------------------------

function Stat({ label, value }: { label: string; value: string | number }) {
  return (
    <div className="px-3 py-1 rounded-lg bg-[#111827] border border-[#1f2937] text-xs">
      <span className="text-slate-500">{label}: </span>
      <span className="text-slate-200 font-medium">{value}</span>
    </div>
  );
}
