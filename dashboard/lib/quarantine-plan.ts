/** Copy for the quarantine plan split. Numbers match src/plan.rs. */

export type CapState = "ok" | "near" | "reached";

export function capNotice(
  state: CapState | string | undefined,
  pending: number,
  limit: number | null | undefined,
): { tone: "info" | "full"; text: string } | null {
  if (state !== "near" && state !== "reached") return null;
  if (limit == null || limit <= 0) return null;
  const held = `${pending.toLocaleString()} of ${limit.toLocaleString()}`;
  if (state === "reached") {
    return {
      tone: "full",
      text: `Quarantine is full for now (${held}). Failures are counted under Drops with their reasons. Storing resumes automatically as space clears.`,
    };
  }
  const pct = Math.round((pending / limit) * 100);
  return {
    tone: "info",
    text: `Quarantine is at ${pct}% (${held}). Older records clear daily after 7 days. New failures will be counted under Drops if it fills.`,
  };
}

export function retentionSentence(days: number | undefined): string {
  if (days === 90) return "Records are kept 90 days on Pro.";
  if (days === 365) return "Records are kept 365 days on Enterprise.";
  return "Records are kept 7 days on Free and 90 days on Pro.";
}

export const FREE_REPLAY_TOOLTIP =
  "Replay uses the stored record body, included with Pro. Fix the producer and resend. New records validate live.";

export const FREE_UPGRADE =
  "On Free you keep what failed and why. Pro adds the record body, replay, and 90-day history.";
