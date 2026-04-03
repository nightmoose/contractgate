"use client";

import { useState } from "react";
import { playgroundValidate } from "@/lib/api";
import type { ValidationResult } from "@/lib/api";
import clsx from "clsx";

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

glossary: []
metrics: []
`;

const DEFAULT_EVENT = `{
  "user_id": "alice_01",
  "event_type": "click",
  "timestamp": 1712000000
}`;

export default function PlaygroundPage() {
  const [yaml, setYaml] = useState(DEFAULT_YAML);
  const [eventJson, setEventJson] = useState(DEFAULT_EVENT);
  const [result, setResult] = useState<ValidationResult | null>(null);
  const [loading, setLoading] = useState(false);
  const [parseError, setParseError] = useState<string | null>(null);

  const handleValidate = async () => {
    setParseError(null);
    let parsed: unknown;
    try {
      parsed = JSON.parse(eventJson);
    } catch (e) {
      setParseError("Invalid JSON in event field");
      return;
    }

    setLoading(true);
    try {
      const r = await playgroundValidate(yaml, parsed);
      setResult(r);
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

      <div className="grid grid-cols-1 xl:grid-cols-2 gap-6">
        {/* Left: inputs */}
        <div className="space-y-4">
          <div>
            <label className="block text-xs text-slate-500 uppercase tracking-wider mb-2">
              Contract YAML
            </label>
            <textarea
              value={yaml}
              onChange={(e) => setYaml(e.target.value)}
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

          <button
            onClick={handleValidate}
            disabled={loading}
            className="w-full py-3 bg-green-600 hover:bg-green-500 disabled:opacity-50 text-white font-semibold rounded-lg transition-colors"
          >
            {loading ? "Validating…" : "▶  Validate Event"}
          </button>
        </div>

        {/* Right: result */}
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
            <div className="flex flex-col items-center justify-center h-64 text-slate-600">
              <p className="text-4xl mb-3">🧪</p>
              <p className="text-sm text-center">
                Enter a contract and event, then click Validate
              </p>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
