"use client";

import clsx from "clsx";

export function HelpToggle({
  compact = false,
  mode,
  onToggle,
}: {
  compact?: boolean;
  mode: boolean;
  onToggle: () => void;
}) {
  return (
    <button
      type="button"
      data-help-ui
      data-help="help.whats-this"
      aria-pressed={mode}
      aria-label="What’s this?"
      onClick={onToggle}
      className={clsx(
        "flex items-center gap-2 rounded-lg text-sm font-medium transition-colors",
        compact
          ? "h-10 w-10 justify-center bg-[#111827] border border-[#374151] shadow-xl"
          : "w-full px-3 py-2",
        mode
          ? "bg-green-900/40 text-green-300 border border-green-700/50"
          : compact
          ? "text-slate-300 hover:text-white"
          : "text-slate-400 hover:text-slate-200 hover:bg-[#1f2937] border border-transparent"
      )}
    >
      <span aria-hidden="true">?</span>
      {!compact && <span>What’s this?</span>}
    </button>
  );
}
