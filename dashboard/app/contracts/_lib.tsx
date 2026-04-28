"use client";

/**
 * Shared helpers, types, and primitives for the Contracts section.
 * RFC-020: extracted from page.tsx to support the _tabs/ split.
 */

import { useEffect } from "react";
import * as Tooltip from "@radix-ui/react-tooltip";
import clsx from "clsx";
import type { VersionSummary } from "@/lib/api";

// ---------------------------------------------------------------------------
// Version-picker helpers
// ---------------------------------------------------------------------------

/** Prefer latest stable by promoted_at; fall back to latest draft; then newest. */
export function pickDefaultVersion(vs: VersionSummary[]): string | null {
  if (vs.length === 0) return null;
  const stables = vs.filter((v) => v.state === "stable");
  if (stables.length > 0) {
    return [...stables].sort((a, b) =>
      (b.promoted_at ?? "").localeCompare(a.promoted_at ?? "")
    )[0].version;
  }
  const drafts = vs.filter((v) => v.state === "draft");
  if (drafts.length > 0) {
    return [...drafts].sort((a, b) =>
      b.created_at.localeCompare(a.created_at)
    )[0].version;
  }
  return [...vs].sort((a, b) => b.created_at.localeCompare(a.created_at))[0].version;
}

/** Newest version string by created_at — seed for suggestNextVersion. */
export function newestVersionString(vs: VersionSummary[]): string | null {
  if (vs.length === 0) return null;
  return [...vs].sort((a, b) => b.created_at.localeCompare(a.created_at))[0].version;
}

// ---------------------------------------------------------------------------
// Contract generator helpers (for GeneratorTab)
// ---------------------------------------------------------------------------

export interface InferredField {
  name: string;
  type: "string" | "integer" | "number" | "boolean";
  required: boolean;
  pattern?: string;
  enum?: string[];
  min?: number;
}

/** Well-known regex patterns we auto-detect. */
export const PATTERNS: [RegExp, string][] = [
  [/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i, "^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$"],
  [/^[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}$/, "^[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\\.[a-zA-Z]{2,}$"],
  [/^\d{4}-\d{2}-\d{2}(T\d{2}:\d{2}:\d{2}(\.\d+)?(Z|[+-]\d{2}:\d{2})?)?$/, "^\\d{4}-\\d{2}-\\d{2}"],
  [/^https?:\/\//, "^https?:\\/\\/"],
  [/^[a-zA-Z0-9_-]{3,64}$/, "^[a-zA-Z0-9_-]{3,64}$"],
];

export function sniffPattern(values: string[]): string | undefined {
  for (const [regex, pattern] of PATTERNS) {
    if (values.every((v) => regex.test(v))) return pattern;
  }
  return undefined;
}

export function inferFields(records: Record<string, unknown>[]): InferredField[] {
  const allKeys = Array.from(new Set(records.flatMap((r) => Object.keys(r))));
  const totalRecords = records.length;

  return allKeys.map((key) => {
    const presentIn = records.filter((r) => key in r);
    const values = presentIn.map((r) => r[key]);
    const required = presentIn.length === totalRecords;

    let type: InferredField["type"] = "string";
    const nonNull = values.filter((v) => v !== null && v !== undefined);

    if (nonNull.every((v) => typeof v === "boolean")) {
      type = "boolean";
    } else if (nonNull.every((v) => typeof v === "number")) {
      type = nonNull.every((v) => Number.isInteger(v)) ? "integer" : "number";
    }

    const field: InferredField = { name: key, type, required };

    if (type === "string") {
      const strValues = nonNull.map((v) => String(v));
      const unique = Array.from(new Set(strValues));
      if (unique.length <= 6 && unique.length < totalRecords) {
        field.enum = unique.sort();
      } else {
        const pattern = sniffPattern(strValues);
        if (pattern) field.pattern = pattern;
      }
    }

    if (type === "integer" || type === "number") {
      const nums = nonNull.map((v) => v as number);
      const min = Math.min(...nums);
      if (min >= 0) field.min = 0;
    }

    return field;
  });
}

export function buildYaml(name: string, fields: InferredField[]): string {
  const safeName = name.trim().replace(/\s+/g, "_").toLowerCase() || "generated_contract";
  const lines: string[] = [
    `version: "1.0"`,
    `name: "${safeName}"`,
    `description: "Auto-generated contract from sample data"`,
    ``,
    `ontology:`,
    `  entities:`,
  ];

  for (const f of fields) {
    lines.push(`    - name: ${f.name}`);
    lines.push(`      type: ${f.type}`);
    lines.push(`      required: ${f.required}`);
    if (f.pattern) lines.push(`      pattern: "${f.pattern}"`);
    if (f.enum) {
      lines.push(`      enum:`);
      for (const v of f.enum) lines.push(`        - "${v}"`);
    }
    if (f.min !== undefined) lines.push(`      min: ${f.min}`);
  }

  lines.push(``, `glossary:`);
  for (const f of fields) {
    lines.push(`  - field: "${f.name}"`);
    lines.push(`    description: "Description of ${f.name}"`);
  }

  lines.push(``, `metrics: []`, ``);
  return lines.join("\n");
}

// ---------------------------------------------------------------------------
// Tooltip primitive (RFC-020 §Design E)
// ---------------------------------------------------------------------------

export function TooltipWrap({
  children,
  content,
  rfc,
}: {
  children: React.ReactNode;
  content: string;
  rfc?: string;
}) {
  return (
    <Tooltip.Provider delayDuration={300}>
      <Tooltip.Root>
        <Tooltip.Trigger asChild>{children}</Tooltip.Trigger>
        <Tooltip.Portal>
          <Tooltip.Content
            className="max-w-xs text-xs bg-[#1f2937] text-slate-200 rounded-lg px-3 py-2 shadow-xl border border-[#374151] z-[200] leading-relaxed"
            sideOffset={4}
          >
            {content}
            {rfc && (
              <span className="ml-1 text-indigo-400 underline underline-offset-2">
                {rfc}
              </span>
            )}
            <Tooltip.Arrow className="fill-[#1f2937]" />
          </Tooltip.Content>
        </Tooltip.Portal>
      </Tooltip.Root>
    </Tooltip.Provider>
  );
}

// ---------------------------------------------------------------------------
// ConfirmActionModal — replaces window.confirm for promote/deprecate
// ---------------------------------------------------------------------------

export function ConfirmActionModal({
  title,
  body,
  confirmLabel,
  destructive,
  onConfirm,
  onCancel,
}: {
  title: string;
  body: string;
  confirmLabel: string;
  destructive: boolean;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  // Escape key cancels
  useEffect(() => {
    const handler = (e: KeyboardEvent) => { if (e.key === "Escape") onCancel(); };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [onCancel]);

  return (
    <div
      className="fixed inset-0 z-[100] flex items-center justify-center bg-black/70 backdrop-blur-sm p-4"
      onClick={(e) => { if (e.target === e.currentTarget) onCancel(); }}
    >
      <div className="bg-[#0f1623] border border-[#1f2937] rounded-2xl w-full max-w-md shadow-2xl p-6 space-y-4">
        <h3 className="text-base font-semibold text-slate-100">{title}</h3>
        <p className="text-sm text-slate-400 leading-relaxed whitespace-pre-wrap">{body}</p>
        <div className="flex gap-3 justify-end pt-2">
          <button
            onClick={onCancel}
            className="px-4 py-2 bg-[#1f2937] hover:bg-[#374151] text-slate-300 text-sm font-medium rounded-lg transition-colors"
          >
            Cancel
          </button>
          <button
            onClick={onConfirm}
            className={clsx(
              "px-4 py-2 text-sm font-medium rounded-lg transition-colors text-white",
              destructive
                ? "bg-amber-700 hover:bg-amber-600"
                : "bg-indigo-600 hover:bg-indigo-500"
            )}
          >
            {confirmLabel}
          </button>
        </div>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// ConfirmReplayModal — used by both per-row and bulk replay actions
// ---------------------------------------------------------------------------

export function ConfirmReplayModal({
  count,
  version,
  onConfirm,
  onCancel,
}: {
  count: number;
  version: string;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  useEffect(() => {
    const handler = (e: KeyboardEvent) => { if (e.key === "Escape") onCancel(); };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [onCancel]);

  return (
    <div
      className="fixed inset-0 z-[100] flex items-center justify-center bg-black/70 backdrop-blur-sm p-4"
      onClick={(e) => { if (e.target === e.currentTarget) onCancel(); }}
    >
      <div className="bg-[#0f1623] border border-[#1f2937] rounded-2xl w-full max-w-md shadow-2xl p-6 space-y-4">
        <h3 className="text-base font-semibold text-slate-100">Confirm Replay</h3>
        <p className="text-sm text-slate-400 leading-relaxed">
          Replay{" "}
          <span className="text-slate-200 font-medium">{count} event{count !== 1 ? "s" : ""}</span>{" "}
          against{" "}
          <span className="text-indigo-300 font-mono font-medium">v{version}</span>.
        </p>
        <p className="text-xs text-slate-500 leading-relaxed">
          Events that pass will be written to audit_log and forwarded downstream.
          Original quarantine rows are preserved regardless of outcome.
        </p>
        <div className="flex gap-3 justify-end pt-2">
          <button
            onClick={onCancel}
            className="px-4 py-2 bg-[#1f2937] hover:bg-[#374151] text-slate-300 text-sm font-medium rounded-lg transition-colors"
          >
            Cancel
          </button>
          <button
            onClick={onConfirm}
            className="px-4 py-2 bg-indigo-600 hover:bg-indigo-500 text-white text-sm font-medium rounded-lg transition-colors"
          >
            ▶ Replay
          </button>
        </div>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// ReplaySummaryModal — shown after bulk or per-row replay completes
// ---------------------------------------------------------------------------

export function ReplaySummaryModal({
  total,
  replayed,
  stillQuarantined,
  alreadyReplayed,
  skipped,
  targetVersion,
  contractId,
  onClose,
}: {
  total: number;
  replayed: number;
  stillQuarantined: number;
  alreadyReplayed: number;
  skipped: number;
  targetVersion: string;
  contractId?: string;
  onClose: () => void;
}) {
  useEffect(() => {
    const handler = (e: KeyboardEvent) => { if (e.key === "Escape") onClose(); };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [onClose]);

  return (
    <div
      className="fixed inset-0 z-[100] flex items-center justify-center bg-black/70 backdrop-blur-sm p-4"
      onClick={(e) => { if (e.target === e.currentTarget) onClose(); }}
    >
      <div className="bg-[#0f1623] border border-[#1f2937] rounded-2xl w-full max-w-md shadow-2xl p-6 space-y-4">
        <div className="flex items-center gap-3">
          <span className="text-2xl">{replayed === total ? "✅" : "⚠️"}</span>
          <h3 className="text-base font-semibold text-slate-100">Replay Complete</h3>
        </div>

        <div className="grid grid-cols-2 gap-2 text-sm">
          <Stat label="Total attempted" value={total} color="slate" />
          <Stat label="Passed" value={replayed} color="green" />
          <Stat label="Still quarantined" value={stillQuarantined} color="red" />
          <Stat label="Already replayed" value={alreadyReplayed} color="indigo" />
          <Stat label="Skipped / purged" value={skipped} color="slate" />
          <div className="col-span-2 border-t border-[#1f2937] pt-2 text-xs text-slate-500">
            Target version:{" "}
            <span className="font-mono text-indigo-300">v{targetVersion}</span>
          </div>
        </div>

        <div className="flex gap-3 justify-end pt-2">
          {contractId && (
            <a
              href={`/audit?contract_id=${contractId}`}
              className="px-4 py-2 bg-[#1f2937] hover:bg-[#374151] text-slate-300 text-sm font-medium rounded-lg transition-colors"
            >
              View in Audit Log →
            </a>
          )}
          <button
            onClick={onClose}
            className="px-4 py-2 bg-indigo-600 hover:bg-indigo-500 text-white text-sm font-medium rounded-lg transition-colors"
          >
            Done
          </button>
        </div>
      </div>
    </div>
  );
}

function Stat({
  label,
  value,
  color,
}: {
  label: string;
  value: number;
  color: "green" | "red" | "indigo" | "amber" | "slate";
}) {
  const colorMap = {
    green: "text-green-400",
    red: "text-red-400",
    indigo: "text-indigo-400",
    amber: "text-amber-400",
    slate: "text-slate-400",
  };
  return (
    <div className="bg-[#111827] border border-[#1f2937] rounded-lg px-3 py-2">
      <p className="text-xs text-slate-500">{label}</p>
      <p className={clsx("text-lg font-semibold", colorMap[color])}>{value}</p>
    </div>
  );
}
