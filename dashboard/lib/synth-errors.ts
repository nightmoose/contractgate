/**
 * Readable messages for POST /contracts/synth error bodies (RFC-117/118).
 * Pure, so the dashboard test runner covers it without a browser.
 */

export const SYNTH_MAX_ROWS = 10000;

const SYNTH_ERROR_CODES = new Set([
  "unsatisfiable_rule",
  "invalid_contract",
  "invalid_rows",
  "no_envelope",
]);

function sentence(s: string): string {
  const t = s.trim();
  if (!t) return t;
  const cap = t[0].toUpperCase() + t.slice(1);
  return /[.!?]$/.test(cap) ? cap : `${cap}.`;
}

/** Message for a synth error body, or null when `body` is not one. */
export function synthErrorMessage(body: unknown): string | null {
  if (!body || typeof body !== "object") return null;
  const b = body as Record<string, unknown>;
  if (typeof b.error !== "string" || !SYNTH_ERROR_CODES.has(b.error)) return null;
  const message = typeof b.message === "string" ? b.message : "";
  switch (b.error) {
    case "invalid_rows": {
      // Server text: "rows must be between 1 and 10000, got 10001".
      const got = /got\s+(-?\d+)/.exec(message)?.[1];
      return got
        ? `Rows must be a whole number from 1 to ${SYNTH_MAX_ROWS} (you asked for ${got}).`
        : `Rows must be a whole number from 1 to ${SYNTH_MAX_ROWS}.`;
    }
    case "no_envelope":
      return "This contract declares no envelope, so rows cannot be wrapped. Turn off the envelope option or add an envelope block to the contract.";
    case "invalid_contract":
      return message ? `The contract could not be compiled: ${message}` : "The contract could not be compiled.";
    default: {
      const rule = typeof b.rule === "string" ? b.rule : "rule";
      const field = typeof b.field === "string" ? ` on '${b.field}'` : "";
      return `No rows can satisfy the ${rule} rule${field}. ${sentence(message)}`.trim();
    }
  }
}
