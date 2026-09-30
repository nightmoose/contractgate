"use client";

import { useState } from "react";
import { egressValidate } from "@/lib/api";
import type { EgressResponse, EgressDisposition } from "@/lib/api";
import clsx from "clsx";

export function OutboundTab({
  contractId,
  hasStableVersion,
}: {
  contractId: string;
  hasStableVersion: boolean;
}) {
  const [disposition, setDisposition] = useState<EgressDisposition>("block");
  const [payloadText, setPayloadText] = useState(
    JSON.stringify(
      [{ user_id: "alice", event_type: "purchase", timestamp: 1716000000, amount: 42.5 }],
      null,
      2,
    ),
  );
  const [dryRun, setDryRun] = useState(true);
  const [result, setResult] = useState<EgressResponse | null>(null);
  const [running, setRunning] = useState(false);
  const [parseErr, setParseErr] = useState<string | null>(null);
  const [apiErr, setApiErr] = useState<string | null>(null);

  if (!hasStableVersion) {
    return (
      <div className="flex flex-col items-center justify-center h-48 text-slate-600 text-center gap-2">
        <p className="text-3xl">↗</p>
        <p className="text-sm">No stable version yet.</p>
        <p className="text-xs text-slate-700">
          Promote a draft to stable first, then come back to test outbound payloads.
          <br />
          To infer a contract from known-good outbound samples, use Generate from Sample.
        </p>
      </div>
    );
  }

  const handleValidate = async () => {
    setParseErr(null);
    setApiErr(null);
    setResult(null);
    let payload: unknown;
    try {
      payload = JSON.parse(payloadText);
    } catch (e) {
      setParseErr(`Invalid JSON: ${e instanceof Error ? e.message : String(e)}`);
      return;
    }
    if (!Array.isArray(payload)) payload = [payload];
    setRunning(true);
    try {
      const res = await egressValidate(contractId, payload, { disposition, dryRun });
      setResult(res);
    } catch (e) {
      setApiErr(e instanceof Error ? e.message : String(e));
    } finally {
      setRunning(false);
    }
  };

  const actionColor = (action: string) => {
    if (action === "included") return "text-green-400 bg-green-900/30";
    if (action === "blocked") return "text-red-400 bg-red-900/30";
    if (action === "rejected") return "text-red-400 bg-red-900/30";
    if (action === "tagged") return "text-amber-400 bg-amber-900/30";
    return "text-slate-400";
  };

  return (
    <div className="space-y-4 py-2">
      <p className="text-sm text-slate-500">
        Validate an outbound payload against this contract. Forward only the returned{" "}
        <code className="text-xs text-green-400 font-mono">payload</code> — never the original
        body after a block or fail.{" "}
        <a
          href="https://github.com/nightmoose/contractgate/blob/main/docs/egress-validation-reference.md"
          target="_blank"
          rel="noreferrer"
          className="text-teal-500 hover:text-teal-400 underline"
        >
          Reference
        </a>
      </p>

      {/* Controls */}
      <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
        <div>
          <label className="text-xs font-medium text-slate-400 uppercase tracking-wider mb-1.5 block">
            Disposition
          </label>
          <select
            value={disposition}
            onChange={(e) => setDisposition(e.target.value as EgressDisposition)}
            className="w-full bg-[#0a0d12] border border-[#1f2937] rounded-lg px-3 py-2 text-sm text-slate-200 outline-none"
          >
            <option value="block">block — drop failing records</option>
            <option value="fail">fail — reject entire batch</option>
            <option value="tag">tag — pass through with flags</option>
          </select>
        </div>
        <div className="flex flex-col justify-end">
          <label className="flex items-center gap-2 cursor-pointer mb-1.5">
            <input
              type="checkbox"
              checked={dryRun}
              onChange={(e) => setDryRun(e.target.checked)}
              className="accent-indigo-500"
            />
            <span className="text-sm text-slate-300">Dry run</span>
          </label>
          <p className="text-xs text-slate-600">
            Dry run validates without writing to the audit log.
          </p>
        </div>
      </div>

      {/* Payload editor */}
      <div>
        <label className="text-xs font-medium text-slate-400 uppercase tracking-wider mb-1.5 block">
          Outbound Payload (JSON array or object)
        </label>
        <textarea
          value={payloadText}
          onChange={(e) => {
            setPayloadText(e.target.value);
            setParseErr(null);
          }}
          rows={8}
          className="w-full bg-[#0a0d12] text-blue-300 font-mono text-sm p-4 rounded-lg border border-[#1f2937] outline-none focus:border-indigo-600 resize-y transition-colors"
          spellCheck={false}
        />
        {parseErr && <p className="mt-1 text-xs text-red-400">{parseErr}</p>}
      </div>

      <button
        onClick={handleValidate}
        disabled={running}
        className="px-5 py-2 bg-indigo-700 hover:bg-indigo-600 disabled:opacity-40 text-white text-sm font-medium rounded-lg transition-colors"
      >
        {running ? "Validating…" : "▶ Validate Outbound"}
      </button>

      {apiErr && (
        <p className="text-sm text-red-400 bg-red-900/20 border border-red-800/40 rounded p-3">
          {apiErr}
        </p>
      )}

      {result && (
        <div className="space-y-4">
          <div className="grid grid-cols-2 sm:grid-cols-4 gap-3">
            {[
              { label: "Total", value: result.total, color: "text-white" },
              { label: "Passed", value: result.passed, color: "text-green-400" },
              { label: "Failed", value: result.failed, color: "text-red-400" },
              {
                label: "Dry run",
                value: result.dry_run ? "yes" : "no",
                color: result.dry_run ? "text-amber-400" : "text-slate-400",
              },
            ].map((s) => (
              <div
                key={s.label}
                className="bg-[#0d1117] border border-[#1f2937] rounded-lg px-3 py-3"
              >
                <p className="text-xs text-slate-500 mb-1">{s.label}</p>
                <p className={clsx("text-xl font-bold", s.color)}>{s.value}</p>
              </div>
            ))}
          </div>

          <div className="bg-[#0d1117] border border-[#1f2937] rounded-xl overflow-hidden">
            <div className="px-4 py-3 border-b border-[#1f2937]">
              <p className="text-xs font-medium text-slate-400 uppercase tracking-wider">
                Per-Record Outcomes · disposition:{" "}
                <span className="text-slate-200">{result.disposition}</span>
                {" · "}v{result.resolved_version}
              </p>
            </div>
            <div className="divide-y divide-[#1f2937]/50">
              {result.outcomes.map((o) => (
                <div key={o.index} className="px-4 py-3 flex items-start gap-3">
                  <span className="text-xs text-slate-600 font-mono w-6 shrink-0">
                    [{o.index}]
                  </span>
                  <span
                    className={clsx(
                      "text-[10px] uppercase tracking-wider border rounded px-2 py-0.5 shrink-0 font-medium border-transparent",
                      actionColor(o.action),
                    )}
                  >
                    {o.action}
                  </span>
                  <div className="flex-1 min-w-0">
                    {o.violations.length > 0 ? (
                      <ul className="space-y-0.5">
                        {o.violations.map((v, vi) => (
                          <li key={vi} className="text-xs text-slate-400">
                            <span className="text-red-400 font-mono">{v.field}</span>
                            {" · "}
                            <span className="text-slate-500">{v.message}</span>
                          </li>
                        ))}
                      </ul>
                    ) : (
                      <p className="text-xs text-slate-600">no violations</p>
                    )}
                  </div>
                  <span className="text-[10px] text-slate-600 font-mono shrink-0">
                    {o.validation_us}µs
                  </span>
                </div>
              ))}
            </div>
          </div>

          {result.payload.length > 0 && (
            <details>
              <summary className="text-xs text-slate-500 cursor-pointer hover:text-slate-300 select-none">
                Cleaned payload ({result.payload.length} record
                {result.payload.length !== 1 ? "s" : ""}) ▾
              </summary>
              <pre className="mt-2 text-[10px] text-green-300 font-mono bg-[#0a0d12] rounded-lg p-4 max-h-64 overflow-auto whitespace-pre-wrap leading-relaxed">
                {JSON.stringify(result.payload, null, 2)}
              </pre>
            </details>
          )}
        </div>
      )}
    </div>
  );
}
