"use client";

/**
 * RFC-118: "Generate synthetic data" on the contract page.
 *
 * Calls POST /contracts/synth with the YAML of the version on screen, so
 * drafts work too. Rows come only from the contract's rules; nothing is
 * stored. Downloads are built in the browser as JSON or CSV.
 */

import { useEffect, useRef, useState } from "react";
import clsx from "clsx";
import { synthRows, warmApi } from "@/lib/api";
import type { SynthInvalidRow, SynthResponse } from "@/lib/api";
import { synthToCsv } from "@/lib/synth-csv";
import { SYNTH_MAX_ROWS as MAX_ROWS } from "@/lib/synth-errors";

/** After this long, explain that the API may be starting up. */
const SLOW_MS = 3000;

function download(name: string, text: string, type: string) {
  const url = URL.createObjectURL(new Blob([text], { type }));
  const link = document.createElement("a");
  link.href = url;
  link.download = name;
  link.click();
  URL.revokeObjectURL(url);
}

function fileStem(res: SynthResponse): string {
  const safe = (s: string) => s.replace(/[^A-Za-z0-9._-]+/g, "_");
  return `${safe(res.contract.name)}-v${safe(res.contract.version)}-${res.mode}-seed${res.seed}`;
}

export function SynthTab({
  yamlContent,
  versionLabel,
  hasEnvelope,
}: {
  yamlContent: string;
  versionLabel: string | null;
  hasEnvelope: boolean;
}) {
  const [rows, setRows] = useState("100");
  const [mode, setMode] = useState<"valid" | "invalid">("valid");
  const [seed, setSeed] = useState("");
  const [anchor, setAnchor] = useState("");
  const [envelope, setEnvelope] = useState(false);
  const [result, setResult] = useState<SynthResponse | null>(null);
  const [running, setRunning] = useState(false);
  const [inputErr, setInputErr] = useState<string | null>(null);
  const [apiErr, setApiErr] = useState<string | null>(null);
  const [elapsedMs, setElapsedMs] = useState(0);
  const [totalMs, setTotalMs] = useState<number | null>(null);
  const timer = useRef<ReturnType<typeof setInterval> | null>(null);

  // Wake the API when the tab opens, so Generate does not pay a cold start.
  useEffect(() => {
    warmApi();
    return () => {
      if (timer.current) clearInterval(timer.current);
    };
  }, []);

  if (!yamlContent.trim()) {
    return (
      <div className="flex flex-col items-center justify-center h-48 text-slate-600 text-center gap-2">
        <p className="text-sm">No contract YAML loaded.</p>
        <p className="text-xs text-slate-700">Pick a version on the YAML tab first.</p>
      </div>
    );
  }

  const handleGenerate = async () => {
    setInputErr(null);
    setApiErr(null);
    setResult(null);
    const n = Number(rows);
    if (!Number.isInteger(n) || n < 1 || n > MAX_ROWS) {
      setInputErr(`Rows must be a whole number from 1 to ${MAX_ROWS}.`);
      return;
    }
    let seedNum: number | undefined;
    if (seed.trim() !== "") {
      seedNum = Number(seed);
      if (!Number.isSafeInteger(seedNum) || seedNum < 0) {
        setInputErr("Seed must be a whole number of 0 or more.");
        return;
      }
    }
    let anchorIso: string | undefined;
    if (anchor.trim() !== "") {
      const d = new Date(anchor);
      if (Number.isNaN(d.getTime())) {
        setInputErr("Anchor time is not a valid date and time.");
        return;
      }
      anchorIso = d.toISOString().replace(/\.\d{3}Z$/, "Z");
    }
    setRunning(true);
    setTotalMs(null);
    setElapsedMs(0);
    const t0 = performance.now();
    timer.current = setInterval(() => setElapsedMs(performance.now() - t0), 250);
    try {
      const res = await synthRows({
        yaml_content: yamlContent,
        rows: n,
        mode,
        ...(seedNum !== undefined ? { seed: seedNum } : {}),
        ...(anchorIso ? { anchor_time: anchorIso } : {}),
        ...(envelope ? { envelope: true } : {}),
      });
      setResult(res);
    } catch (e) {
      setApiErr(e instanceof Error ? e.message : String(e));
    } finally {
      if (timer.current) clearInterval(timer.current);
      timer.current = null;
      setTotalMs(Math.round(performance.now() - t0));
      setRunning(false);
    }
  };

  const downloadJson = () => {
    if (!result) return;
    let body: unknown = result.rows;
    if (envelope && result.envelope_payload !== undefined) {
      body =
        result.mode === "invalid"
          ? {
              payload: result.envelope_payload,
              labels: (result.rows as SynthInvalidRow[]).map((r) => r.broken_rule),
            }
          : result.envelope_payload;
    }
    download(`${fileStem(result)}.json`, JSON.stringify(body, null, 2) + "\n", "application/json");
  };

  const downloadCsv = () => {
    if (!result) return;
    download(`${fileStem(result)}.csv`, synthToCsv(result), "text/csv");
  };

  const inputCls =
    "w-full bg-[#0a0d12] border border-[#1f2937] rounded-lg px-3 py-2 text-sm text-slate-200 outline-none focus:border-indigo-600";
  const labelCls = "text-xs font-medium text-slate-400 uppercase tracking-wider mb-1.5 block";

  return (
    <div className="space-y-4 py-2">
      <p className="text-sm text-slate-500">
        Generate synthetic data from this contract{versionLabel ? ` (v${versionLabel})` : ""}.
        Rows come only from the contract&apos;s rules. No stored events are read and nothing is
        saved. Invalid mode returns rows that each break exactly one rule, labeled with it.{" "}
        <a
          href="https://github.com/nightmoose/contractgate/blob/main/docs/synthetic-data-reference.md"
          target="_blank"
          rel="noreferrer"
          className="text-teal-500 hover:text-teal-400 underline"
        >
          Reference
        </a>
      </p>

      <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-4 gap-3">
        <div>
          <label htmlFor="synth-rows" className={labelCls}>Rows</label>
          <input
            id="synth-rows"
            type="number"
            min={1}
            max={MAX_ROWS}
            value={rows}
            onChange={(e) => setRows(e.target.value)}
            className={inputCls}
          />
        </div>
        <div>
          <label htmlFor="synth-mode" className={labelCls}>Mode</label>
          <select
            id="synth-mode"
            value={mode}
            onChange={(e) => setMode(e.target.value as "valid" | "invalid")}
            className={inputCls}
          >
            <option value="valid">valid: every row passes</option>
            <option value="invalid">invalid: each row breaks one rule</option>
          </select>
        </div>
        <div>
          <label htmlFor="synth-seed" className={labelCls}>Seed (optional)</label>
          <input
            id="synth-seed"
            type="number"
            min={0}
            placeholder="random"
            value={seed}
            onChange={(e) => setSeed(e.target.value)}
            className={inputCls}
          />
        </div>
        <div>
          <label htmlFor="synth-anchor" className={labelCls}>Anchor time (optional)</label>
          <input
            id="synth-anchor"
            type="datetime-local"
            step={1}
            value={anchor}
            onChange={(e) => setAnchor(e.target.value)}
            className={inputCls}
          />
        </div>
      </div>
      <div className="flex flex-col gap-1">
        <label
          className={clsx(
            "flex items-center gap-2",
            hasEnvelope ? "cursor-pointer" : "opacity-50 cursor-not-allowed",
          )}
          title={hasEnvelope ? undefined : "This contract declares no envelope"}
        >
          <input
            type="checkbox"
            checked={envelope && hasEnvelope}
            disabled={!hasEnvelope}
            onChange={(e) => setEnvelope(e.target.checked)}
            className="accent-indigo-500"
          />
          <span className="text-sm text-slate-300">Wrap rows in the contract&apos;s envelope (JSON)</span>
        </label>
        {!hasEnvelope && (
          <p className="text-xs text-slate-500 pl-6">
            This contract has no <code className="font-mono">envelope:</code> block, so there is
            nothing to wrap rows in. Add one to the YAML to turn this on.
          </p>
        )}
        <p className="text-xs text-slate-600">
          Same seed and anchor time give the same rows. Without an anchor, freshness rules use the
          current time.
        </p>
      </div>
      {inputErr && <p className="text-xs text-red-400">{inputErr}</p>}

      <div className="flex items-center gap-3 flex-wrap">
        <button
          onClick={handleGenerate}
          disabled={running}
          aria-busy={running}
          className="inline-flex items-center gap-2 px-5 py-2 bg-indigo-700 hover:bg-indigo-600 disabled:opacity-60 disabled:cursor-wait text-white text-sm font-medium rounded-lg transition-colors"
        >
          {running && (
            <span
              aria-hidden
              className="h-4 w-4 rounded-full border-2 border-white/30 border-t-white animate-spin"
            />
          )}
          {running ? "Generating…" : "Generate synthetic data"}
        </button>
        {running && (
          <span role="status" className="text-xs text-slate-400">
            {(elapsedMs / 1000).toFixed(1)}s
            {elapsedMs > SLOW_MS &&
              ". Still working: the first request after a quiet period can take a few seconds while the API starts."}
          </span>
        )}
      </div>

      {apiErr && (
        <p className="text-sm text-red-400 bg-red-900/20 border border-red-800/40 rounded p-3">
          {apiErr}
        </p>
      )}

      {result && (
        <div className="space-y-4">
          <div className="grid grid-cols-2 sm:grid-cols-4 gap-3">
            {[
              { label: "Rows", value: result.row_count, color: "text-white" },
              {
                label: "Mode",
                value: result.mode,
                color: result.mode === "invalid" ? "text-amber-400" : "text-green-400",
              },
              { label: "Seed", value: result.seed, color: "text-slate-200" },
              { label: "Skipped rules", value: result.skipped_rules.length, color: "text-slate-400" },
            ].map((s) => (
              <div key={s.label} className="bg-[#0d1117] border border-[#1f2937] rounded-lg px-3 py-3">
                <p className="text-xs text-slate-500 mb-1">{s.label}</p>
                <p className={clsx("text-xl font-bold truncate", s.color)}>{s.value}</p>
              </div>
            ))}
          </div>
          <p className="text-xs text-slate-600">
            Anchor time: {result.anchor_time}
            {totalMs !== null && (
              <>
                {" "}· Done in {(totalMs / 1000).toFixed(2)}s
                {typeof result.generation_ms === "number" && ` (generation ${result.generation_ms} ms on the server)`}
              </>
            )}
          </p>

          <div className="flex gap-2 flex-wrap">
            <button
              onClick={downloadJson}
              className="px-4 py-2 bg-green-600 hover:bg-green-500 text-white text-sm font-medium rounded-lg transition-colors"
            >
              Download JSON
            </button>
            <button
              onClick={downloadCsv}
              className="px-4 py-2 bg-slate-700 hover:bg-slate-600 text-white text-sm font-medium rounded-lg transition-colors"
            >
              Download CSV
            </button>
          </div>

          {result.skipped_rules.length > 0 && (
            <details>
              <summary className="text-xs text-slate-500 cursor-pointer hover:text-slate-300 select-none">
                Rules no row breaks on its own ({result.skipped_rules.length}) ▾
              </summary>
              <ul className="mt-2 space-y-0.5">
                {result.skipped_rules.map((s, i) => (
                  <li key={i} className="text-xs text-slate-400">
                    <span className="text-amber-400 font-mono">{s.field}</span> · {s.rule} ·{" "}
                    <span className="text-slate-500">{s.reason}</span>
                  </li>
                ))}
              </ul>
            </details>
          )}

          <div>
            <p className="text-xs font-medium text-slate-400 uppercase tracking-wider mb-1.5">
              Preview (first {Math.min(5, result.rows.length)} of {result.row_count})
            </p>
            <pre className="text-[10px] text-green-300 font-mono bg-[#0a0d12] rounded-lg p-4 max-h-64 overflow-auto whitespace-pre-wrap leading-relaxed">
              {JSON.stringify(result.rows.slice(0, 5), null, 2)}
            </pre>
          </div>
        </div>
      )}
    </div>
  );
}
