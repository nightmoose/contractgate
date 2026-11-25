/**
 * CSV for POST /contracts/synth responses (RFC-118). Same layout as the
 * Rust and Python CLIs: nested objects become dot-path columns, arrays are
 * JSON text, and invalid rows get `_broken_field` and `_broken_rule`.
 */

import type { SynthInvalidRow, SynthResponse } from "./api";

/** Nested objects become dot-path columns; arrays are JSON text (matches the CLI). */
function flatten(value: unknown, prefix: string, out: [string, string][]): void {
  if (value !== null && typeof value === "object" && !Array.isArray(value)) {
    const entries = Object.entries(value as Record<string, unknown>);
    if (entries.length > 0 || prefix === "") {
      for (const [k, v] of entries) flatten(v, prefix ? `${prefix}.${k}` : k, out);
      return;
    }
  }
  let cell: string;
  if (value === null || value === undefined) cell = "";
  else if (typeof value === "string") cell = value;
  else cell = JSON.stringify(value);
  out.push([prefix, cell]);
}

function csvCell(s: string): string {
  return /[",\n\r]/.test(s) ? `"${s.replace(/"/g, '""')}"` : s;
}

export function synthToCsv(res: SynthResponse): string {
  const rows: [string, string][][] = [];
  for (const r of res.rows) {
    const cells: [string, string][] = [];
    if (res.mode === "invalid") {
      const inv = r as SynthInvalidRow;
      flatten(inv.record, "", cells);
      cells.push(["_broken_field", inv.broken_rule.field]);
      cells.push(["_broken_rule", inv.broken_rule.rule]);
    } else {
      flatten(r, "", cells);
    }
    rows.push(cells);
  }
  const header: string[] = [];
  for (const cells of rows) for (const [k] of cells) if (!header.includes(k)) header.push(k);
  const lines = [header.map(csvCell).join(",")];
  for (const cells of rows) {
    const m = new Map(cells);
    lines.push(header.map((h) => csvCell(m.get(h) ?? "")).join(","));
  }
  return lines.join("\n") + "\n";
}
