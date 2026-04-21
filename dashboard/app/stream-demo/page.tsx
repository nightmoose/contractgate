"use client";

import { useState, useEffect, useRef, useCallback } from "react";
import clsx from "clsx";

// ---------------------------------------------------------------------------
// Simulation constants
// ---------------------------------------------------------------------------

const TICK_MS = 200;
const HISTORY_LEN = 120; // ~24 s of history at 200 ms snapshots

// Nominal throughput when target_rate = 0 (unbounded), events/sec
const BASE_RATE = 52_000;

// Copy-lane latency baselines (µs) — models Kafka passthrough cost
const C_P50  =  190;
const C_P95  =  430;
const C_P99  =  810;
const C_MAX  = 2400;

// Validation overhead added on top of copy-lane latency (µs)
const V_ADD_P50  =  42;
const V_ADD_P95  =  88;
const V_ADD_P99  = 165;
const V_ADD_MAX  = 750;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

interface RunConfig {
  scenario: "simple" | "nested";
  fail_ratio: number;   // 0…1
  target_rate: number;  // events/sec; 0 = unbounded
  duration_secs: number; // 0 = until stop
}

interface LaneStats {
  consumed: number;
  produced_downstream: number;
  passed: number;
  failed: number;
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
  validator: LaneStats;
  copy: LaneStats;
}

interface HistoryPoint {
  elapsed_s: number;
  producer_rate: number;
  validator_rate: number;
  copy_rate: number;
}

// ---------------------------------------------------------------------------
// Simulation helpers
// ---------------------------------------------------------------------------

function jitter(val: number, pct = 0.08): number {
  return Math.max(0, val * (1 + (Math.random() - 0.5) * 2 * pct));
}

function warmup(elapsed_ms: number): number {
  // Ramp from 0 to 1 over the first 2 s
  return Math.min(1, elapsed_ms / 2000);
}

function buildZeroLane(): LaneStats {
  return {
    consumed: 0,
    produced_downstream: 0,
    passed: 0,
    failed: 0,
    rate_per_sec: 0,
    p50_us: 0,
    p95_us: 0,
    p99_us: 0,
    max_us: 0,
  };
}

function buildZeroSnapshot(config: RunConfig): Snapshot {
  return {
    running: false,
    elapsed_ms: 0,
    scenario: config.scenario,
    fail_ratio: config.fail_ratio,
    producer: { sent: 0, rate_per_sec: 0 },
    validator: buildZeroLane(),
    copy: buildZeroLane(),
  };
}

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
// SVG Throughput chart
// ---------------------------------------------------------------------------

const CHART_H = 200;
const PAD = { top: 14, right: 12, bottom: 28, left: 56 };

function ThroughputChart({ history }: { history: HistoryPoint[] }) {
  const wrapRef = useRef<HTMLDivElement>(null);
  const [width, setWidth] = useState(700);

  useEffect(() => {
    if (!wrapRef.current) return;
    const ro = new ResizeObserver(([entry]) => {
      setWidth(entry.contentRect.width || 700);
    });
    ro.observe(wrapRef.current);
    setWidth(wrapRef.current.offsetWidth);
    return () => ro.disconnect();
  }, []);

  const innerW = width - PAD.left - PAD.right;
  const innerH = CHART_H - PAD.top - PAD.bottom;

  const maxRate = Math.max(
    ...history.map((h) =>
      Math.max(h.producer_rate, h.validator_rate, h.copy_rate)
    ),
    1000
  );

  function toX(i: number): number {
    const offset = HISTORY_LEN - history.length;
    return PAD.left + ((i + offset) / (HISTORY_LEN - 1)) * innerW;
  }
  function toY(rate: number): number {
    return PAD.top + innerH - (rate / maxRate) * innerH;
  }

  function buildPolyline(getter: (h: HistoryPoint) => number): string {
    return history.map((h, i) => `${toX(i).toFixed(1)},${toY(getter(h)).toFixed(1)}`).join(" ");
  }

  function buildArea(getter: (h: HistoryPoint) => number): string {
    if (history.length === 0) return "";
    const top = history.map((h, i) => `${toX(i).toFixed(1)},${toY(getter(h)).toFixed(1)}`).join(" ");
    const base = `${toX(history.length - 1).toFixed(1)},${(PAD.top + innerH).toFixed(1)} ${toX(0).toFixed(1)},${(PAD.top + innerH).toFixed(1)}`;
    return `${top} ${base}`;
  }

  const yTicks = [0, 0.25, 0.5, 0.75, 1].map((pct) => ({
    y: PAD.top + innerH * (1 - pct),
    label: fmtRate(maxRate * pct),
  }));

  const xTicks: { x: number; label: string }[] = [];
  if (history.length > 1) {
    const step = Math.max(1, Math.floor(history.length / 6));
    for (let i = 0; i < history.length; i += step) {
      xTicks.push({ x: toX(i), label: history[i].elapsed_s.toFixed(0) + "s" });
    }
  }

  const legend = [
    { color: "#8a97ad", label: "Producer" },
    { color: "#5ea1ff", label: "ContractGate" },
    { color: "#ffc857", label: "Straight-copy" },
  ];

  return (
    <div ref={wrapRef} style={{ width: "100%" }}>
      {history.length < 2 ? (
        <div
          className="flex items-center justify-center text-slate-600 text-sm"
          style={{ height: CHART_H }}
        >
          Waiting for data…
        </div>
      ) : (
        <svg width={width} height={CHART_H}>
          {/* Grid lines */}
          {yTicks.map((t, i) => (
            <g key={i}>
              <line
                x1={PAD.left} y1={t.y}
                x2={width - PAD.right} y2={t.y}
                stroke="rgba(255,255,255,0.04)"
                strokeWidth={1}
              />
              <text
                x={PAD.left - 6} y={t.y + 4}
                textAnchor="end"
                fill="#8a97ad"
                fontSize={10}
                fontFamily="ui-monospace, monospace"
              >
                {t.label}
              </text>
            </g>
          ))}

          {/* X-axis labels */}
          {xTicks.map((t, i) => (
            <text
              key={i}
              x={t.x} y={CHART_H - 6}
              textAnchor="middle"
              fill="#8a97ad"
              fontSize={10}
              fontFamily="ui-monospace, monospace"
            >
              {t.label}
            </text>
          ))}

          {/* Filled areas */}
          <polygon
            points={buildArea((h) => h.producer_rate)}
            fill="rgba(138,151,173,0.08)"
          />
          <polygon
            points={buildArea((h) => h.copy_rate)}
            fill="rgba(255,200,87,0.12)"
          />
          <polygon
            points={buildArea((h) => h.validator_rate)}
            fill="rgba(94,161,255,0.15)"
          />

          {/* Lines */}
          <polyline
            points={buildPolyline((h) => h.producer_rate)}
            fill="none"
            stroke="#8a97ad"
            strokeWidth={1.5}
            strokeLinejoin="round"
          />
          <polyline
            points={buildPolyline((h) => h.copy_rate)}
            fill="none"
            stroke="#ffc857"
            strokeWidth={2}
            strokeLinejoin="round"
          />
          <polyline
            points={buildPolyline((h) => h.validator_rate)}
            fill="none"
            stroke="#5ea1ff"
            strokeWidth={2}
            strokeLinejoin="round"
          />

          {/* Legend */}
          {legend.map((item, i) => (
            <g key={i} transform={`translate(${PAD.left + 8 + i * 120}, ${PAD.top + 6})`}>
              <line x1={0} y1={5} x2={16} y2={5} stroke={item.color} strokeWidth={2} />
              <text x={20} y={9} fill="#e6edf7" fontSize={11} fontFamily="ui-sans-serif, system-ui">
                {item.label}
              </text>
            </g>
          ))}
        </svg>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Sub-components
// ---------------------------------------------------------------------------

function StatusPill({ running, scenario, fail_ratio }: { running: boolean; scenario: string; fail_ratio: number }) {
  return (
    <span
      className={clsx(
        "inline-flex items-center px-3 py-1 rounded-full text-xs font-medium",
        running
          ? "bg-green-900/40 text-green-400 border border-green-700/50"
          : "bg-slate-800/60 text-slate-500 border border-slate-700/40"
      )}
    >
      {running
        ? `running — ${scenario} · ${(fail_ratio * 100).toFixed(0)}% fail`
        : "idle"}
    </span>
  );
}

function SummaryCard({
  label,
  value,
  unit,
  sub,
  overheadClass,
}: {
  label: string;
  value: string;
  unit?: string;
  sub?: React.ReactNode;
  overheadClass?: string;
}) {
  return (
    <div
      className={clsx(
        "bg-[#111827] border border-[#1f2937] rounded-xl p-4",
        overheadClass
      )}
    >
      <p className="text-[10px] text-slate-500 uppercase tracking-wider">{label}</p>
      <p className="text-2xl font-semibold font-mono tabular-nums mt-1">
        {value}
        {unit && <span className="text-sm text-slate-500 font-sans ml-1">{unit}</span>}
      </p>
      {sub && <p className="text-[11px] text-slate-500 mt-1 font-mono tabular-nums">{sub}</p>}
    </div>
  );
}

function MetricBox({ label, value, unit }: { label: string; value: string; unit?: string }) {
  return (
    <div className="bg-[#0a0d12] rounded-lg p-3">
      <p className="text-[10px] text-slate-500 uppercase tracking-wider">{label}</p>
      <p className="text-xl font-semibold font-mono tabular-nums mt-1">
        {value}
        {unit && <span className="text-[11px] text-slate-500 font-sans ml-1">{unit}</span>}
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
    return (
      <div className="h-7 rounded-lg bg-green-900/20 flex items-center justify-center text-xs font-semibold text-green-400">
        passthrough — no checks
      </div>
    );
  }
  const total = (passed + failed) || 1;
  const pPct = (passed / total) * 100;
  const fPct = (failed / total) * 100;
  return (
    <div className="h-7 rounded-lg overflow-hidden flex text-xs font-semibold">
      <div
        className="flex items-center justify-center bg-green-900/25 text-green-400 transition-all duration-300"
        style={{ flex: pPct || 1 }}
      >
        {passed > 0 ? `pass ${fmtNum(passed)}` : ""}
      </div>
      <div
        className="flex items-center justify-center bg-red-900/25 text-red-400 transition-all duration-300"
        style={{ flex: fPct }}
      >
        {failed > 0 ? `fail ${fmtNum(failed)}` : ""}
      </div>
    </div>
  );
}

function LanePanel({
  title,
  sub,
  color,
  stats,
  passthrough,
}: {
  title: string;
  sub: string;
  color: "blue" | "amber";
  stats: LaneStats;
  passthrough?: boolean;
}) {
  const dotClass =
    color === "blue"
      ? "bg-[#5ea1ff] shadow-[0_0_8px_#5ea1ff]"
      : "bg-[#ffc857] shadow-[0_0_8px_#ffc857]";

  return (
    <div className="bg-[#111827] border border-[#1f2937] rounded-xl p-5 flex flex-col gap-4">
      <div>
        <h2 className="flex items-center gap-2 text-sm font-semibold">
          <span className={clsx("w-2.5 h-2.5 rounded-full shrink-0", dotClass)} />
          {title}
          <span className="text-slate-500 font-normal text-xs">{sub}</span>
        </h2>
      </div>

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
// Main page
// ---------------------------------------------------------------------------

export default function StreamDemoPage() {
  // ── Form state ────────────────────────────────────────────────────────────
  const [scenario, setScenario] = useState<"simple" | "nested">("simple");
  const [failPct, setFailPct] = useState(10);
  const [targetRate, setTargetRate] = useState(0);
  const [durationSecs, setDurationSecs] = useState(30);

  // ── Simulation output ─────────────────────────────────────────────────────
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null);
  const [history, setHistory] = useState<HistoryPoint[]>([]);
  const [running, setRunning] = useState(false);

  // ── Internal mutable sim state (avoids stale closure in setInterval) ──────
  const simRef = useRef<{
    config: RunConfig;
    elapsed_ms: number;
    producer: { sent: number };
    validator: LaneStats;
    copy: LaneStats;
  } | null>(null);

  const intervalRef = useRef<ReturnType<typeof setInterval> | null>(null);

  // ── Tick function ─────────────────────────────────────────────────────────
  const tick = useCallback(() => {
    const s = simRef.current;
    if (!s) return;

    s.elapsed_ms += TICK_MS;

    const cfg = s.config;

    // Check duration limit
    if (cfg.duration_secs > 0 && s.elapsed_ms >= cfg.duration_secs * 1000) {
      stopSim();
      return;
    }

    const w = warmup(s.elapsed_ms);
    const nominalRate = cfg.target_rate > 0 ? cfg.target_rate : BASE_RATE;
    const producerRate = jitter(nominalRate * w, 0.07);
    const thisTickEvents = Math.round((producerRate * TICK_MS) / 1000);

    s.producer.sent += thisTickEvents;

    // Validator lane: consumes ~96% of producer rate
    const vFactor = jitter(0.96, 0.03);
    const vEvents = Math.round(thisTickEvents * vFactor);
    const vRate = jitter(producerRate * vFactor, 0.05);
    const vPassed = Math.round(vEvents * (1 - cfg.fail_ratio));
    const vFailed = vEvents - vPassed;

    s.validator.consumed += vEvents;
    s.validator.produced_downstream += vEvents;
    s.validator.passed += vPassed;
    s.validator.failed += vFailed;
    s.validator.rate_per_sec = vRate;
    s.validator.p50_us  = Math.round(jitter(C_P50 + V_ADD_P50, 0.12) * w);
    s.validator.p95_us  = Math.round(jitter(C_P95 + V_ADD_P95, 0.12) * w);
    s.validator.p99_us  = Math.round(jitter(C_P99 + V_ADD_P99, 0.10) * w);
    s.validator.max_us  = Math.round(jitter(C_MAX + V_ADD_MAX, 0.18) * w);

    // Copy lane: consumes ~99% of producer rate, no validation overhead
    const cFactor = jitter(0.99, 0.02);
    const cEvents = Math.round(thisTickEvents * cFactor);
    const cRate = jitter(producerRate * cFactor, 0.04);

    s.copy.consumed += cEvents;
    s.copy.produced_downstream += cEvents;
    s.copy.passed += cEvents;
    s.copy.rate_per_sec = cRate;
    s.copy.p50_us  = Math.round(jitter(C_P50, 0.12) * w);
    s.copy.p95_us  = Math.round(jitter(C_P95, 0.12) * w);
    s.copy.p99_us  = Math.round(jitter(C_P99, 0.10) * w);
    s.copy.max_us  = Math.round(jitter(C_MAX, 0.18) * w);

    const snap: Snapshot = {
      running: true,
      elapsed_ms: s.elapsed_ms,
      scenario: cfg.scenario,
      fail_ratio: cfg.fail_ratio,
      producer: { sent: s.producer.sent, rate_per_sec: producerRate },
      validator: { ...s.validator },
      copy: { ...s.copy },
    };

    setSnapshot(snap);
    setHistory((prev) => {
      const next = [
        ...prev,
        {
          elapsed_s: s.elapsed_ms / 1000,
          producer_rate: producerRate,
          validator_rate: vRate,
          copy_rate: cRate,
        },
      ];
      return next.length > HISTORY_LEN ? next.slice(next.length - HISTORY_LEN) : next;
    });
  }, []);

  // ── Start / Stop ──────────────────────────────────────────────────────────

  function stopSim() {
    if (intervalRef.current) {
      clearInterval(intervalRef.current);
      intervalRef.current = null;
    }
    setRunning(false);
    if (simRef.current) {
      setSnapshot((prev) => prev ? { ...prev, running: false } : null);
    }
  }

  function startSim() {
    const config: RunConfig = {
      scenario,
      fail_ratio: failPct / 100,
      target_rate: targetRate,
      duration_secs: durationSecs,
    };

    // Reset sim state
    simRef.current = {
      config,
      elapsed_ms: 0,
      producer: { sent: 0 },
      validator: buildZeroLane(),
      copy: buildZeroLane(),
    };
    setHistory([]);
    setSnapshot(buildZeroSnapshot(config));
    setRunning(true);

    intervalRef.current = setInterval(tick, TICK_MS);
  }

  // Clean up on unmount
  useEffect(() => {
    return () => {
      if (intervalRef.current) clearInterval(intervalRef.current);
    };
  }, []);

  // ── Derived display values ────────────────────────────────────────────────
  const snap = snapshot;
  const vRate = snap?.validator.rate_per_sec ?? 0;
  const cRate = snap?.copy.rate_per_sec ?? 0;
  const ratio = cRate > 0 ? Math.round((vRate / cRate) * 100) : null;
  const overhead = ratio !== null ? Math.max(0, 100 - ratio) : null;
  const haveLatency = (snap?.validator.p99_us ?? 0) > 0 && (snap?.copy.p99_us ?? 0) > 0;
  const p99Delta = haveLatency
    ? Math.max(0, (snap!.validator.p99_us - snap!.copy.p99_us))
    : null;

  const overheadCardClass =
    overhead === null ? "" :
    overhead <= 5  ? "!border-green-700/40 [&_.overhead-val]:text-green-400" :
    overhead <= 20 ? "!border-amber-600/40 [&_.overhead-val]:text-amber-400" :
                     "!border-red-700/40 [&_.overhead-val]:text-red-400";

  // ── Render ────────────────────────────────────────────────────────────────

  return (
    <div className="space-y-5">
      {/* Page header */}
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-bold">Stream Demo</h1>
          <p className="text-sm text-slate-500 mt-1">
            In-browser simulation of ContractGate&apos;s Kafka validation pipeline — no backend required
          </p>
        </div>
        <StatusPill
          running={running}
          scenario={snap?.scenario ?? scenario}
          fail_ratio={snap?.fail_ratio ?? failPct / 100}
        />
      </div>

      {/* Controls */}
      <div className="bg-[#111827] border border-[#1f2937] rounded-xl p-4">
        <div className="grid grid-cols-2 md:grid-cols-4 gap-4 items-end">
          <div>
            <label className="block text-[10px] text-slate-500 uppercase tracking-wider mb-1.5">
              Scenario
            </label>
            <select
              value={scenario}
              onChange={(e) => setScenario(e.target.value as "simple" | "nested")}
              disabled={running}
              className="w-full bg-[#0a0d12] border border-[#1f2937] text-slate-300 text-sm rounded-lg px-3 py-2 outline-none focus:border-green-700 disabled:opacity-50"
            >
              <option value="simple">simple — flat click-stream</option>
              <option value="nested">nested — e-commerce order</option>
            </select>
          </div>

          <div>
            <label className="block text-[10px] text-slate-500 uppercase tracking-wider mb-1.5">
              Fail ratio (% violating)
            </label>
            <input
              type="number"
              min={0} max={100} step={1}
              value={failPct}
              onChange={(e) => setFailPct(Math.max(0, Math.min(100, Number(e.target.value) || 0)))}
              disabled={running}
              className="w-full bg-[#0a0d12] border border-[#1f2937] text-slate-300 text-sm rounded-lg px-3 py-2 outline-none focus:border-green-700 disabled:opacity-50"
            />
          </div>

          <div>
            <label className="block text-[10px] text-slate-500 uppercase tracking-wider mb-1.5">
              Target rate (ev/s, 0 = max)
            </label>
            <input
              type="number"
              min={0} step={1000}
              value={targetRate}
              onChange={(e) => setTargetRate(Math.max(0, Number(e.target.value) || 0))}
              disabled={running}
              className="w-full bg-[#0a0d12] border border-[#1f2937] text-slate-300 text-sm rounded-lg px-3 py-2 outline-none focus:border-green-700 disabled:opacity-50"
            />
          </div>

          <div>
            <label className="block text-[10px] text-slate-500 uppercase tracking-wider mb-1.5">
              Duration (s, 0 = until stop)
            </label>
            <input
              type="number"
              min={0} step={5}
              value={durationSecs}
              onChange={(e) => setDurationSecs(Math.max(0, Number(e.target.value) || 0))}
              disabled={running}
              className="w-full bg-[#0a0d12] border border-[#1f2937] text-slate-300 text-sm rounded-lg px-3 py-2 outline-none focus:border-green-700 disabled:opacity-50"
            />
          </div>
        </div>

        <div className="flex gap-3 mt-4">
          <button
            onClick={startSim}
            disabled={running}
            className="px-5 py-2 rounded-lg text-sm font-semibold bg-green-600 hover:bg-green-500 text-white disabled:opacity-40 disabled:cursor-not-allowed transition-colors"
          >
            ▶ Start
          </button>
          <button
            onClick={stopSim}
            disabled={!running}
            className="px-5 py-2 rounded-lg text-sm font-semibold bg-red-600 hover:bg-red-500 text-white disabled:opacity-40 disabled:cursor-not-allowed transition-colors"
          >
            ■ Stop
          </button>
        </div>
      </div>

      {/* Summary strip */}
      <div className="grid grid-cols-2 lg:grid-cols-4 gap-4">
        <SummaryCard
          label="ContractGate throughput"
          value={fmtRate(vRate)}
          unit="ev/s"
          sub={<>{fmtNum(snap?.validator.produced_downstream ?? 0)} forwarded</>}
        />
        <div
          className={clsx(
            "bg-[#111827] border border-[#1f2937] rounded-xl p-4 transition-colors",
            overheadCardClass
          )}
        >
          <p className="text-[10px] text-slate-500 uppercase tracking-wider">
            Overhead vs baseline
          </p>
          <p className="text-2xl font-semibold font-mono tabular-nums mt-1 overhead-val">
            {overhead !== null ? overhead : "—"}
            <span className="text-sm text-slate-500 font-sans ml-1">%</span>
          </p>
          <p className="text-[11px] text-slate-500 mt-1 font-mono tabular-nums">
            {p99Delta !== null ? fmtNum(p99Delta) : "—"} µs · validation cost (p99)
          </p>
        </div>
        <SummaryCard
          label="Producer"
          value={fmtRate(snap?.producer.rate_per_sec ?? 0)}
          unit="ev/s"
          sub={<>{fmtNum(snap?.producer.sent ?? 0)} sent</>}
        />
        <SummaryCard
          label="Elapsed"
          value={((snap?.elapsed_ms ?? 0) / 1000).toFixed(1)}
          unit="s"
          sub={
            ratio !== null
              ? <>{ratio}% of baseline throughput</>
              : <>—</>
          }
        />
      </div>

      {/* Throughput chart */}
      <div className="bg-[#111827] border border-[#1f2937] rounded-xl p-5">
        <p className="text-xs text-slate-500 uppercase tracking-wider font-semibold mb-3">
          Throughput — events / sec
        </p>
        <ThroughputChart history={history} />
      </div>

      {/* Per-lane detail */}
      <div className="grid grid-cols-1 lg:grid-cols-2 gap-4">
        <LanePanel
          title="ContractGate validator"
          sub="consume → validate → forward to valid / quarantine"
          color="blue"
          stats={snap?.validator ?? buildZeroLane()}
        />
        <LanePanel
          title="Straight-copy baseline"
          sub="consume → forward unchanged (no validation)"
          color="amber"
          stats={snap?.copy ?? buildZeroLane()}
          passthrough
        />
      </div>

      {/* Explainer footer */}
      <div className="bg-[#0a0d12] border border-[#1f2937] rounded-xl p-4 text-xs text-slate-500 leading-relaxed">
        <strong className="text-slate-400">How this works:</strong> The simulation models two concurrent
        consumer groups reading from the same input stream. The <span className="text-[#5ea1ff]">ContractGate validator</span> lane
        validates every event against the selected contract schema and routes passing events to{" "}
        <code className="text-slate-400">demo.events.valid</code> and violators to{" "}
        <code className="text-slate-400">demo.events.quarantine</code>. The{" "}
        <span className="text-[#ffc857]">straight-copy baseline</span> forwards events unchanged —
        zero validation, Kafka overhead only. The overhead card shows how much slower ContractGate
        is relative to that baseline; production runs typically land at{" "}
        <strong className="text-slate-400">&lt;5%</strong>. The real binary (
        <code className="text-slate-400">cargo demo</code>) drives actual Kafka/Redpanda for live numbers.
      </div>
    </div>
  );
}
