"use client";

// RFC-083 Phase 3 — per-org monthly usage widget.
// Reads GET /usage (Phase 1) and renders used/limit with a progress bar and an
// upgrade CTA when the org is near or over its plan limit.

import useSWR from "swr";
import Link from "next/link";
import { getUsage } from "@/lib/api";

function fmt(n: number): string {
  return n.toLocaleString();
}

export default function UsageWidget() {
  const { data, error, isLoading } = useSWR("/usage", () => getUsage(), {
    refreshInterval: 60_000,
  });

  // Non-fatal: if usage can't load, don't clutter the page.
  if (error) return null;

  if (isLoading || !data) {
    return (
      <div className="bg-[#111827] border border-[#1f2937] rounded-2xl p-6 mb-8">
        <div className="text-sm font-semibold text-slate-200 mb-1">Usage this month</div>
        <div className="text-xs text-slate-500">Loading…</div>
      </div>
    );
  }

  const { used, limit, pct, unlimited, period_start } = data;
  const periodLabel = new Date(period_start).toLocaleDateString(undefined, {
    month: "long",
    year: "numeric",
  });
  const hasLimit = !unlimited && limit != null;
  const clampedPct = pct == null ? 0 : Math.min(pct, 100);
  const over = hasLimit && used >= (limit as number);
  const warn = pct != null && pct >= 80;
  const barColor = over ? "bg-red-500" : warn ? "bg-amber-500" : "bg-green-500";

  return (
    <div className="bg-[#111827] border border-[#1f2937] rounded-2xl p-6 mb-8">
      <div className="flex items-center justify-between mb-3">
        <div>
          <div className="text-sm font-semibold text-slate-200">Usage this month</div>
          <div className="text-xs text-slate-500">
            {periodLabel} · validated events (UTC month)
          </div>
        </div>
        <div className="text-sm text-slate-300 font-medium tabular-nums">
          {fmt(used)}
          {hasLimit ? ` / ${fmt(limit as number)}` : ""} events
        </div>
      </div>

      {hasLimit ? (
        <>
          <div className="h-2 w-full rounded-full bg-[#1f2937] overflow-hidden">
            <div
              className={`h-full ${barColor} transition-all`}
              style={{ width: `${clampedPct}%` }}
            />
          </div>
          <div className="mt-2 flex items-center justify-between gap-2">
            <span className="text-[11px] text-slate-500">
              {clampedPct.toFixed(1)}% of plan limit
              {over ? " · ingest blocked until next month or upgrade" : warn ? " · approaching cap" : ""}
            </span>
            {(over || warn) && (
              <Link
                href="/pricing"
                className="text-[11px] px-2.5 py-1 rounded bg-green-600 hover:bg-green-500 text-white font-medium shrink-0"
              >
                {over ? "Upgrade plan" : "Upgrade"}
              </Link>
            )}
          </div>
          {over && (
            <p className="mt-2 text-[11px] text-slate-500">
              Free/Growth monthly caps return HTTP 429 on ingest when exceeded.
              Upgrade for more headroom, or wait until the next UTC month.
            </p>
          )}
        </>
      ) : (
        <p className="text-xs text-slate-500">
          Unlimited events on your plan (Enterprise).
        </p>
      )}
    </div>
  );
}
