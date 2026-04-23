"use client";

import { useState, useEffect, useRef, useCallback } from "react";
import clsx from "clsx";

// ---------------------------------------------------------------------------
// Backend base URL (same env var the rest of the dashboard uses)
// ---------------------------------------------------------------------------
const BASE = process.env.NEXT_PUBLIC_API_URL ?? "http://localhost:3001";

// ---------------------------------------------------------------------------
// Types — identical shape to the Rust StatsSnapshot
// ---------------------------------------------------------------------------

interface LaneSnapshot {
  consumed: number;
  produced_downstream: number;
  passed: number;
  failed: number;
  bytes_in: number;
  rate_per_sec: number;
  p50_us: number;
  p95_us: number;
  p99_us: number;
  max_us: number;
}

interface Snapshot {
  running: boolean;
  elapsed_ms: number;
  scenario: string;
  fail_ratio: number;
  producer: { sent: number; rate_per_sec: number };
  validator: LaneSnapshot;
  copy: LaneSnapshot;
}

interface HistoryPoint {
  elapsed_s: number;
  producer_rate: number;
  validator_rate: number;
  copy_rate: number;
}

interface EventViolation {
  field: string;
  message: string;
  kind: string;
}

interface EventRecord {
  seq: number;
  elapsed_ms: number;
  passed: boolean;
  violations: EventViolation[];
  event: Record<string, unknown>;
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const HISTORY_LEN = 120; // 24 s at 200 ms snapshots
const CHART_H = 200;
const PAD = { top: 14, right: 12, bottom: 28, left: 56 };

// ---------------------------------------------------------------------------
// Formatters
// ---------------------------------------------------------------------------

function fmtNum(n: number): string {
  return Math.round(n ?? 0).toLocaleString();
}

function fmtRate(n: number): string {
  if (!Number.isFinite(n) || n === 0) return "0";
  if (n >= 1_000_000) return (n / 1_000_000).toFixed(2) + "M";
  if (n >= 1_000) return (n / 1_000).toFixed(1) + "k";
  return Math.round(n).toString();
}

// ---------------------------------------------------------------------------
// SVG Throughput chart — with clickable legend toggles + auto Y rescale
// ---------------------------------------------------------------------------

type SeriesKey = "producer" | "validator" | "copy";

const SERIES: { key: SeriesKey; color: string; label: string; fill: string; strokeWidth: number; fn: (h: HistoryPoint) => number }[] = [
  { key: "producer",  color: "#8a97ad", label: "Producer",      fill: "rgba(138,151,173,0.08)", strokeWidth: 1.5, fn: (h) => h.producer_rate  },
  { key: "validator", color: "#5ea1ff", label: "ContractGate",  fill: "rgba(94,161,255,0.15)",  strokeWidth: 2,   fn: (h) => h.validator_rate },
  { key: "copy",      color: "#ffc857", label: "Straight-copy", fill: "rgba(255,200,87,0.12)",  strokeWidth: 2,   fn: (h) => h.copy_rate      },
];

function ThroughputChart({ history }: { history: HistoryPoint[] }) {
  const wrapRef = useRef<HTMLDivElement>(null);
  const [width, setWidth] = useState(700);
  const [visible, setVisible] = useState<Record<SeriesKey, boolean>>({
    producer: true, validator: true, copy: true,
  });

  const toggle = (key: SeriesKey) => setVisible((v) => ({ ...v, [key]: !v[key] }));

  useEffect(() => {
    if (!wrapRef.current) return;
    const ro = new ResizeObserver(([e]) => setWidth(e.contentRect.width || 700));
    ro.observe(wrapRef.current);
    setWidth(wrapRef.current.offsetWidth);
    return () => ro.disconnect();
  }, []);

  const innerW = width - PAD.left - PAD.right;
  const innerH = CHART_H - PAD.top - PAD.bottom;

  // Y scale only considers visible series so toggling Producer reveals the gap
  const visibleRates = history.flatMap((h) =>
    SERIES.filter((s) => visible[s.key]).map((s) => s.fn(h))
  );
  const maxRate = Math.max(...(visibleRates.length ? visibleRates : [0]), 1000);

  function toX(i: number) {
    return PAD.left + ((i + HISTORY_LEN - history.length) / (HISTORY_LEN - 1)) * innerW;
  }
  function toY(rate: number) {
    return PAD.top + innerH - (rate / maxRate) * innerH;
  }
  function pts(fn: (h: HistoryPoint) => number) {
    return history.map((h, i) => `${toX(i).toFixed(1)},${toY(fn(h)).toFixed(1)}`).join(" ");
  }
  function area(fn: (h: HistoryPoint) => number) {
    if (!history.length) return "";
    const base = `${toX(history.length - 1).toFixed(1)},${(PAD.top + innerH).toFixed(1)} ${toX(0).toFixed(1)},${(PAD.top + innerH).toFixed(1)}`;
    return `${pts(fn)} ${base}`;
  }

  const yTicks = [0, 0.25, 0.5, 0.75, 1].map((p) => ({
    y: PAD.top + innerH * (1 - p),
    label: fmtRate(maxRate * p),
  }));
  const xTicks: { x: number; label: string }[] = [];
  if (history.length > 1) {
    const step = Math.max(1, Math.floor(history.length / 6));
    for (let i = 0; i < history.length; i += step)
      xTicks.push({ x: toX(i), label: history[i].elapsed_s.toFixed(0) + "s" });
  }

  return (
    <div ref={wrapRef} style={{ width: "100%" }}>
      {history.length < 2 ? (
        <div className="flex items-center justify-center text-slate-600 text-sm" style={{ height: CHART_H }}>
          Waiting for data…
        </div>
      ) : (
        <>
          {/* Clickable legend lives outside the SVG so hover/cursor works reliably */}
          <div className="flex items-center gap-5 mb-2 ml-14 flex-wrap">
            {SERIES.map((s) => (
              <button
                key={s.key}
                onClick={() => toggle(s.key)}
                className="flex items-center gap-1.5 text-xs transition-opacity select-none"
                style={{ opacity: visible[s.key] ? 1 : 0.35 }}
                title={visible[s.key] ? `Hide ${s.label}` : `Show ${s.label}`}
              >
                <svg width={18} height={10} className="shrink-0">
                  <line
                    x1={0} y1={5} x2={18} y2={5}
                    stroke={s.color}
                    strokeWidth={s.strokeWidth}
                    strokeDasharray={visible[s.key] ? undefined : "3,2"}
                  />
                </svg>
                <span style={{ color: visible[s.key] ? "#e6edf7" : "#6b7280", textDecoration: visible[s.key] ? "none" : "line-through" }}>
                  {s.label}
                </span>
              </button>
            ))}
            <span className="text-[10px] text-slate-600 ml-1">click to toggle</span>
          </div>

          <svg width={width} height={CHART_H}>
            {yTicks.map((t, i) => (
              <g key={i}>
                <line x1={PAD.left} y1={t.y} x2={width - PAD.right} y2={t.y} stroke="rgba(255,255,255,0.04)" strokeWidth={1} />
                <text x={PAD.left - 6} y={t.y + 4} textAnchor="end" fill="#8a97ad" fontSize={10} fontFamily="ui-monospace,monospace">{t.label}</text>
              </g>
            ))}
            {xTicks.map((t, i) => (
              <text key={i} x={t.x} y={CHART_H - 6} textAnchor="middle" fill="#8a97ad" fontSize={10} fontFamily="ui-monospace,monospace">{t.label}</text>
            ))}
            {/* Render back-to-front: areas first, then lines on top */}
            {SERIES.map((s) => visible[s.key] && (
              <polygon key={`area-${s.key}`} points={area(s.fn)} fill={s.fill} />
            ))}
            {SERIES.map((s) => visible[s.key] && (
              <polyline key={`line-${s.key}`} points={pts(s.fn)} fill="none" stroke={s.color} strokeWidth={s.strokeWidth} strokeLinejoin="round" />
            ))}
          </svg>
        </>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Sub-components
// ---------------------------------------------------------------------------

function MetricBox({ label, value, unit }: { label: string; value: string; unit?: string }) {
  return (
    <div className="bg-[#0a0d12] rounded-lg p-3">
      <p className="text-[10px] text-slate-500 uppercase tracking-wider">{label}</p>
      <p className="text-xl font-semibold font-mono tabular-nums mt-1">
        {value}{unit && <span className="text-[11px] text-slate-500 font-sans ml-1">{unit}</span>}
      </p>
    </div>
  );
}

function LatBox({ label, value }: { label: string; value: string }) {
  return (
    <div className="bg-[#0a0d12] rounded-lg px-3 py-2">
      <p className="text-[10px] text-slate-500 uppercase tracking-wider">{label}</p>
      <p className="text-base font-semibold font-mono tabular-nums mt-0.5">
        {value} <span className="text-[10px] text-slate-500">µs</span>
      </p>
    </div>
  );
}

function PassFailBar({ passed, failed, passthrough }: { passed: number; failed: number; passthrough?: boolean }) {
  if (passthrough) {
    return <div className="h-7 rounded-lg bg-green-900/20 flex items-center justify-center text-xs font-semibold text-green-400">passthrough — no checks</div>;
  }
  const total = (passed + failed) || 1;
  return (
    <div className="h-7 rounded-lg overflow-hidden flex text-xs font-semibold">
      <div className="flex items-center justify-center bg-green-900/25 text-green-400 transition-all duration-300" style={{ flex: passed / total || 1 }}>
        {passed > 0 ? `pass ${fmtNum(passed)}` : ""}
      </div>
      <div className="flex items-center justify-center bg-red-900/25 text-red-400 transition-all duration-300" style={{ flex: failed / total }}>
        {failed > 0 ? `fail ${fmtNum(failed)}` : ""}
      </div>
    </div>
  );
}

function LanePanel({ title, sub, color, stats, passthrough }: {
  title: string; sub: string; color: "blue" | "amber"; stats: LaneSnapshot; passthrough?: boolean;
}) {
  const dotClass = color === "blue"
    ? "bg-[#5ea1ff] shadow-[0_0_8px_#5ea1ff]"
    : "bg-[#ffc857] shadow-[0_0_8px_#ffc857]";
  return (
    <div className="bg-[#111827] border border-[#1f2937] rounded-xl p-5 flex flex-col gap-4">
      <h2 className="flex items-center gap-2 text-sm font-semibold">
        <span className={clsx("w-2.5 h-2.5 rounded-full shrink-0", dotClass)} />
        {title}
        <span className="text-slate-500 font-normal text-xs">{sub}</span>
      </h2>
      <div className="grid grid-cols-3 gap-3">
        <MetricBox label="Rate" value={fmtRate(stats.rate_per_sec)} unit="ev/s" />
        <MetricBox label="Consumed" value={fmtNum(stats.consumed)} />
        <MetricBox label="Forwarded" value={fmtNum(stats.produced_downstream)} />
      </div>
      <div className="grid grid-cols-4 gap-2">
        <LatBox label="p50" value={fmtNum(stats.p50_us)} />
        <LatBox label="p95" value={fmtNum(stats.p95_us)} />
        <LatBox label="p99" value={fmtNum(stats.p99_us)} />
        <LatBox label="max" value={fmtNum(stats.max_us)} />
      </div>
      <PassFailBar passed={stats.passed} failed={stats.failed} passthrough={passthrough} />
    </div>
  );
}

// ---------------------------------------------------------------------------
// Records tab — live feed of sampled validation results
// ---------------------------------------------------------------------------

const EVENT_FEED_MAX = 50;

function kindBadge(kind: string) {
  const map: Record<string, string> = {
    missing_required_field: "missing",
    type_mismatch:          "type",
    pattern_mismatch:       "pattern",
    enum_violation:         "enum",
    range_violation:        "range",
    length_violation:       "length",
    metric_range_violation: "metric",
    undeclared_field:       "undeclared",
  };
  return map[kind] ?? kind;
}

function EventRow({ rec }: { rec: EventRecord }) {
  const [open, setOpen] = useState(false);
  const elapsedS = (rec.elapsed_ms / 1000).toFixed(2);

  // Build a compact one-line preview of the event: top-level scalar fields only
  const preview = Object.entries(rec.event)
    .filter(([, v]) => typeof v !== "object" || v === null)
    .slice(0, 4)
    .map(([k, v]) => `${k}: ${JSON.stringify(v)}`)
    .join("  ·  ");

  return (
    <div className={clsx(
      "border-b border-[#1f2937]/60 last:border-0 px-4 py-2.5",
      open && "bg-[#0a0d12]/60"
    )}>
      <div
        className="flex items-start gap-3 cursor-pointer"
        onClick={() => setOpen((o) => !o)}
      >
        {/* pass/fail badge */}
        <span className={clsx(
          "shrink-0 mt-0.5 text-[10px] font-bold px-1.5 py-0.5 rounded font-mono",
          rec.passed
            ? "bg-green-900/40 text-green-400 border border-green-700/40"
            : "bg-red-900/40 text-red-400 border border-red-700/40"
        )}>
          {rec.passed ? "PASS" : "FAIL"}
        </span>

        {/* time + seq */}
        <span className="shrink-0 text-[10px] text-slate-600 font-mono mt-0.5 w-16">{elapsedS}s</span>

        {/* preview */}
        <span className="flex-1 min-w-0 text-xs text-slate-400 truncate font-mono">
          {preview}
        </span>

        {/* violation pills */}
        {!rec.passed && (
          <div className="shrink-0 flex gap-1 flex-wrap justify-end max-w-[40%]">
            {rec.violations.slice(0, 3).map((v, i) => (
              <span key={i} className="text-[9px] bg-red-900/20 text-red-400 border border-red-800/30 rounded px-1.5 py-0.5 font-mono">
                {v.field}: {kindBadge(v.kind)}
              </span>
            ))}
            {rec.violations.length > 3 && (
              <span className="text-[9px] text-slate-600">+{rec.violations.length - 3}</span>
            )}
          </div>
        )}

        {/* expand chevron */}
        <span className="shrink-0 text-slate-600 text-xs mt-0.5">{open ? "▲" : "▼"}</span>
      </div>

      {/* Expanded detail */}
      {open && (
        <div className="mt-2 ml-[76px] space-y-2">
          {/* Full violation list */}
          {rec.violations.length > 0 && (
            <div className="space-y-1">
              {rec.violations.map((v, i) => (
                <div key={i} className="flex items-start gap-2 text-xs">
                  <span className="text-red-500 font-mono shrink-0">{v.field}</span>
                  <span className="text-slate-500">{v.message}</span>
                </div>
              ))}
            </div>
          )}
          {/* Raw event JSON */}
          <pre className="text-[10px] text-slate-500 bg-[#0a0d12] border border-[#1f2937] rounded p-2 overflow-x-auto">
            {JSON.stringify(rec.event, null, 2)}
          </pre>
        </div>
      )}
    </div>
  );
}

function RecordsTab({ records }: { records: EventRecord[] }) {
  if (records.length === 0) {
    return (
      <div className="flex flex-col items-center justify-center py-16 text-slate-600">
        <p className="text-sm">No records yet — start a run to see the live feed.</p>
        <p className="text-xs mt-1">Sampled at 1-in-500 (passes) · 1-in-50 (failures)</p>
      </div>
    );
  }

  const passes = records.filter((r) => r.passed).length;
  const fails = records.length - passes;

  return (
    <div>
      <div className="flex items-center gap-4 px-4 py-2 border-b border-[#1f2937] text-xs text-slate-500">
        <span>Last <strong className="text-slate-300">{records.length}</strong> sampled records</span>
        <span className="text-green-500">{passes} pass</span>
        <span className="text-red-500">{fails} fail</span>
        <span className="ml-auto">click row to expand · newest first</span>
      </div>
      {records.map((rec) => (
        <EventRow key={`${rec.seq}-${rec.elapsed_ms}`} rec={rec} />
      ))}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Main page
// ---------------------------------------------------------------------------

const ZERO_LANE: LaneSnapshot = {
  consumed: 0, produced_downstream: 0, passed: 0, failed: 0,
  bytes_in: 0, rate_per_sec: 0, p50_us: 0, p95_us: 0, p99_us: 0, max_us: 0,
};

type ConnState = "idle" | "connecting" | "connected" | "error";

export default function StreamDemoPage() {
  // ── Form state ────────────────────────────────────────────────────────────
  const [scenario, setScenario] = useState<"simple" | "nested">("simple");
  const [failPct, setFailPct] = useState(10);
  const [targetRate, setTargetRate] = useState(0);
  const [durationSecs, setDurationSecs] = useState(30);

  // ── Live data from backend ────────────────────────────────────────────────
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null);
  const [history, setHistory] = useState<HistoryPoint[]>([]);
  const [running, setRunning] = useState(false);
  const [connState, setConnState] = useState<ConnState>("idle");
  const [error, setError] = useState<string | null>(null);

  // ── Records tab ───────────────────────────────────────────────────────────
  const [tab, setTab] = useState<"stats" | "records">("stats");
  const [eventRecords, setEventRecords] = useState<EventRecord[]>([]);
  const eventsEsRef = useRef<EventSource | null>(null);

  const esRef = useRef<EventSource | null>(null);
  const lastElapsedRef = useRef(0);

  // ── SSE connection ────────────────────────────────────────────────────────

  const connect = useCallback(() => {
    if (esRef.current) {
      esRef.current.close();
    }
    setConnState("connecting");
    setError(null);

    const es = new EventSource(`${BASE}/demo/stream`);
    esRef.current = es;

    es.onopen = () => setConnState("connected");

    es.onmessage = (ev) => {
      let s: Snapshot;
      try { s = JSON.parse(ev.data); } catch { return; }

      setConnState("connected");
      setSnapshot(s);
      setRunning(s.running);

      if (s.elapsed_ms !== lastElapsedRef.current) {
        lastElapsedRef.current = s.elapsed_ms;
        setHistory((prev) => {
          const next = [
            ...prev,
            {
              elapsed_s: s.elapsed_ms / 1000,
              producer_rate: s.producer.rate_per_sec,
              validator_rate: s.validator.rate_per_sec,
              copy_rate: s.copy.rate_per_sec,
            },
          ];
          return next.length > HISTORY_LEN ? next.slice(next.length - HISTORY_LEN) : next;
        });
      }
    };

    es.onerror = () => {
      setConnState("error");
      setError("Cannot reach backend — is the Rust server running on " + BASE + "?");
    };
  }, []);

  // Connect stats SSE on mount
  useEffect(() => {
    connect();
    return () => { esRef.current?.close(); };
  }, [connect]);

  // Connect events SSE on mount — stays open in background regardless of active tab
  useEffect(() => {
    const es = new EventSource(`${BASE}/demo/events`);
    eventsEsRef.current = es;
    es.onmessage = (ev) => {
      let rec: EventRecord;
      try { rec = JSON.parse(ev.data); } catch { return; }
      setEventRecords((prev) => {
        const next = [rec, ...prev];
        return next.length > EVENT_FEED_MAX ? next.slice(0, EVENT_FEED_MAX) : next;
      });
    };
    return () => { es.close(); };
  }, []);

  // ── Start / Stop ──────────────────────────────────────────────────────────

  async function handleStart() {
    setError(null);
    setHistory([]);
    setEventRecords([]);
    lastElapsedRef.current = 0;
    try {
      const res = await fetch(`${BASE}/demo/start`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          scenario,
          fail_ratio: failPct / 100,
          target_rate: targetRate > 0 ? targetRate : null,
          duration_secs: durationSecs,
        }),
      });
      if (!res.ok) throw new Error(await res.text());
    } catch (e) {
      setError("Start failed: " + (e instanceof Error ? e.message : String(e)));
    }
  }

  async function handleStop() {
    try {
      await fetch(`${BASE}/demo/stop`, { method: "POST" });
    } catch { /* idempotent */ }
  }

  // ── Derived display values ────────────────────────────────────────────────
  const snap = snapshot;
  const vRate = snap?.validator.rate_per_sec ?? 0;
  const cRate = snap?.copy.rate_per_sec ?? 0;
  const ratio = cRate > 0 ? Math.round((vRate / cRate) * 100) : null;
  const overhead = ratio !== null ? Math.max(0, 100 - ratio) : null;
  const haveLatency = (snap?.validator.p99_us ?? 0) > 0 && (snap?.copy.p99_us ?? 0) > 0;
  const p99Delta = haveLatency ? Math.max(0, snap!.validator.p99_us - snap!.copy.p99_us) : null;

  const overheadBorderClass =
    overhead === null ? "" :
    overhead <= 5  ? "!border-green-700/40" :
    overhead <= 20 ? "!border-amber-600/40" : "!border-red-700/40";
  const overheadTextClass =
    overhead === null ? "text-slate-300" :
    overhead <= 5  ? "text-green-400" :
    overhead <= 20 ? "text-amber-400" : "text-red-400";

  // ── Connection badge ──────────────────────────────────────────────────────
  const connBadge = {
    idle:       { cls: "bg-slate-800/60 text-slate-500 border-slate-700/40", label: "not connected" },
    connecting: { cls: "bg-slate-800/60 text-slate-400 border-slate-700/40 animate-pulse", label: "connecting…" },
    connected:  { cls: "bg-green-900/40 text-green-400 border-green-700/50", label: running ? `running — ${snap?.scenario ?? ""} · ${((snap?.fail_ratio ?? 0) * 100).toFixed(0)}% fail` : "connected · idle" },
    error:      { cls: "bg-red-900/40 text-red-400 border-red-700/50", label: "backend offline" },
  }[connState];

  return (
    <div className="space-y-5">
      {/* Page header */}
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-bold">Stream Demo</h1>
          <p className="text-sm text-slate-500 mt-1">
            Real validation engine · no Kafka · no database writes
          </p>
        </div>
        <span className={clsx("inline-flex items-center px-3 py-1 rounded-full text-xs font-medium border", connBadge.cls)}>
          {connBadge.label}
        </span>
      </div>

      {/* Error banner */}
      {error && (
        <div className="flex items-start gap-3 bg-red-900/20 border border-red-800/40 rounded-xl px-4 py-3">
          <span className="text-red-400 text-lg shrink-0">⚠</span>
          <div className="flex-1 min-w-0">
            <p className="text-sm text-red-300">{error}</p>
            <button onClick={connect} className="mt-1 text-xs text-red-400 underline underline-offset-2 hover:text-red-300">
              Retry connection
            </button>
          </div>
        </div>
      )}

      {/* Controls */}
      <div className="bg-[#111827] border border-[#1f2937] rounded-xl p-4">
        <div className="grid grid-cols-2 md:grid-cols-4 gap-4 items-end">
          <div>
            <label className="block text-[10px] text-slate-500 uppercase tracking-wider mb-1.5">Scenario</label>
            <select value={scenario} onChange={(e) => setScenario(e.target.value as "simple" | "nested")} disabled={running}
              className="w-full bg-[#0a0d12] border border-[#1f2937] text-slate-300 text-sm rounded-lg px-3 py-2 outline-none focus:border-green-700 disabled:opacity-50">
              <option value="simple">simple — flat click-stream</option>
              <option value="nested">nested — e-commerce order</option>
            </select>
          </div>
          <div>
            <label className="block text-[10px] text-slate-500 uppercase tracking-wider mb-1.5">Fail ratio (% violating)</label>
            <input type="number" min={0} max={100} step={1} value={failPct}
              onChange={(e) => setFailPct(Math.max(0, Math.min(100, Number(e.target.value) || 0)))}
              disabled={running}
              className="w-full bg-[#0a0d12] border border-[#1f2937] text-slate-300 text-sm rounded-lg px-3 py-2 outline-none focus:border-green-700 disabled:opacity-50" />
          </div>
          <div>
            <label className="block text-[10px] text-slate-500 uppercase tracking-wider mb-1.5">Target rate (ev/s, 0 = max)</label>
            <input type="number" min={0} step={1000} value={targetRate}
              onChange={(e) => setTargetRate(Math.max(0, Number(e.target.value) || 0))}
              disabled={running}
              className="w-full bg-[#0a0d12] border border-[#1f2937] text-slate-300 text-sm rounded-lg px-3 py-2 outline-none focus:border-green-700 disabled:opacity-50" />
          </div>
          <div>
            <label className="block text-[10px] text-slate-500 uppercase tracking-wider mb-1.5">Duration (s, 0 = until stop)</label>
            <input type="number" min={0} step={5} value={durationSecs}
              onChange={(e) => setDurationSecs(Math.max(0, Number(e.target.value) || 0))}
              disabled={running}
              className="w-full bg-[#0a0d12] border border-[#1f2937] text-slate-300 text-sm rounded-lg px-3 py-2 outline-none focus:border-green-700 disabled:opacity-50" />
          </div>
        </div>
        <div className="flex gap-3 mt-4">
          <button onClick={handleStart} disabled={running || connState === "error"}
            className="px-5 py-2 rounded-lg text-sm font-semibold bg-green-600 hover:bg-green-500 text-white disabled:opacity-40 disabled:cursor-not-allowed transition-colors">
            ▶ Start
          </button>
          <button onClick={handleStop} disabled={!running}
            className="px-5 py-2 rounded-lg text-sm font-semibold bg-red-600 hover:bg-red-500 text-white disabled:opacity-40 disabled:cursor-not-allowed transition-colors">
            ■ Stop
          </button>
        </div>
      </div>

      {/* Tab bar */}
      <div className="flex gap-1 border-b border-[#1f2937]">
        {(["stats", "records"] as const).map((t) => (
          <button
            key={t}
            onClick={() => setTab(t)}
            className={clsx(
              "px-4 py-2 text-sm font-medium transition-colors border-b-2 -mb-px",
              tab === t
                ? "border-green-500 text-green-400"
                : "border-transparent text-slate-500 hover:text-slate-300"
            )}
          >
            {t === "stats" ? "📊 Stats" : "📋 Records"}
            {t === "records" && eventRecords.length > 0 && (
              <span className="ml-1.5 text-[10px] bg-slate-700 text-slate-400 rounded-full px-1.5 py-0.5">
                {eventRecords.length}
              </span>
            )}
          </button>
        ))}
      </div>

      {/* Stats tab */}
      {tab === "stats" && <>

      {/* Summary strip */}
      <div className="grid grid-cols-2 lg:grid-cols-4 gap-4">
        <div className="bg-[#111827] border border-[#1f2937] rounded-xl p-4">
          <p className="text-[10px] text-slate-500 uppercase tracking-wider">ContractGate throughput</p>
          <p className="text-2xl font-semibold font-mono tabular-nums mt-1">{fmtRate(vRate)}<span className="text-sm text-slate-500 font-sans ml-1">ev/s</span></p>
          <p className="text-[11px] text-slate-500 mt-1 font-mono tabular-nums">{fmtNum(snap?.validator.produced_downstream ?? 0)} forwarded</p>
        </div>
        <div className={clsx("bg-[#111827] border border-[#1f2937] rounded-xl p-4 transition-colors", overheadBorderClass)}>
          <p className="text-[10px] text-slate-500 uppercase tracking-wider">Overhead vs baseline</p>
          <p className={clsx("text-2xl font-semibold font-mono tabular-nums mt-1", overheadTextClass)}>
            {overhead !== null ? overhead : "—"}<span className="text-sm text-slate-500 font-sans ml-1">%</span>
          </p>
          <p className="text-[11px] text-slate-500 mt-1 font-mono tabular-nums">
            {p99Delta !== null ? fmtNum(p99Delta) : "—"} µs · validation cost (p99)
          </p>
        </div>
        <div className="bg-[#111827] border border-[#1f2937] rounded-xl p-4">
          <p className="text-[10px] text-slate-500 uppercase tracking-wider">Producer</p>
          <p className="text-2xl font-semibold font-mono tabular-nums mt-1">{fmtRate(snap?.producer.rate_per_sec ?? 0)}<span className="text-sm text-slate-500 font-sans ml-1">ev/s</span></p>
          <p className="text-[11px] text-slate-500 mt-1 font-mono tabular-nums">{fmtNum(snap?.producer.sent ?? 0)} sent</p>
        </div>
        <div className="bg-[#111827] border border-[#1f2937] rounded-xl p-4">
          <p className="text-[10px] text-slate-500 uppercase tracking-wider">Elapsed</p>
          <p className="text-2xl font-semibold font-mono tabular-nums mt-1">{((snap?.elapsed_ms ?? 0) / 1000).toFixed(1)}<span className="text-sm text-slate-500 font-sans ml-1">s</span></p>
          <p className="text-[11px] text-slate-500 mt-1 font-mono tabular-nums">{ratio !== null ? `${ratio}% of baseline throughput` : "—"}</p>
        </div>
      </div>

      {/* Throughput chart */}
      <div className="bg-[#111827] border border-[#1f2937] rounded-xl p-5">
        <p className="text-xs text-slate-500 uppercase tracking-wider font-semibold mb-3">Throughput — events / sec</p>
        <ThroughputChart history={history} />
      </div>

      {/* Per-lane detail */}
      <div className="grid grid-cols-1 lg:grid-cols-2 gap-4">
        <LanePanel title="ContractGate validator" sub="generate → validate → forward" color="blue" stats={snap?.validator ?? ZERO_LANE} />
        <LanePanel title="Straight-copy baseline" sub="generate → serialize only (no validation)" color="amber" stats={snap?.copy ?? ZERO_LANE} passthrough />
      </div>

      {/* Explainer */}
      <div className="bg-[#0a0d12] border border-[#1f2937] rounded-xl p-4 text-xs text-slate-500 leading-relaxed">
        <strong className="text-slate-400">How it works:</strong> Both lanes run in-process inside the Rust server — no Kafka, no database.
        The <span className="text-[#5ea1ff]">ContractGate lane</span> calls the real semantic validation engine on every event.
        The <span className="text-[#ffc857]">copy lane</span> does a serde round-trip only (no validation).
        The latency delta between them is the <em>exact cost</em> of semantic validation.
        The full Kafka-backed demo (<code className="text-slate-400">cargo demo</code>) adds real queue depth and network overhead on top.
      </div>

      </> /* end stats tab */}

      {/* Records tab */}
      {tab === "records" && (
        <div className="bg-[#111827] border border-[#1f2937] rounded-xl overflow-hidden">
          <RecordsTab records={eventRecords} />
        </div>
      )}
    </div>
  );
}
