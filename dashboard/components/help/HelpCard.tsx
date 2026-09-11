"use client";

import { useEffect, useRef } from "react";
import { getHelp } from "@/lib/help/catalog";

export function HelpCard({
  id,
  rect,
  onClose,
}: {
  id: string;
  rect: DOMRect;
  onClose: () => void;
}) {
  const entry = getHelp(id);
  const closeRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    closeRef.current?.focus();
  }, [id]);

  const width = 340;
  const margin = 12;
  const left = Math.max(
    margin,
    Math.min(rect.left, window.innerWidth - width - margin)
  );
  const below = rect.bottom + 8;
  const estimatedHeight = 220;
  const top =
    below + estimatedHeight > window.innerHeight - margin
      ? Math.max(margin, rect.top - estimatedHeight - 8)
      : below;

  return (
    <div
      data-help-ui
      role="dialog"
      aria-modal="false"
      aria-labelledby="help-card-title"
      className="fixed z-[210] w-[340px] max-w-[calc(100vw-24px)] bg-[#111827] text-slate-200 rounded-xl shadow-2xl border border-[#374151] p-4"
      style={{ top, left }}
    >
      <div className="flex items-start justify-between gap-3 mb-2">
        <h2 id="help-card-title" className="text-sm font-semibold text-slate-100 leading-snug">
          {entry?.title ?? "Not documented yet"}
        </h2>
        <button
          ref={closeRef}
          type="button"
          onClick={onClose}
          className="text-slate-500 hover:text-slate-200 text-lg leading-none shrink-0"
          aria-label="Close help"
        >
          ×
        </button>
      </div>
      {entry ? (
        <div className="space-y-2 text-xs leading-relaxed">
          <p className="text-slate-300">{entry.what}</p>
          <p className="text-slate-400">{entry.does}</p>
          {entry.when && (
            <p className="text-slate-500">
              <span className="uppercase tracking-wider text-[10px] text-slate-600 mr-1">
                When
              </span>
              {entry.when}
            </p>
          )}
          {entry.href && (
            <a
              href={entry.href}
              className="inline-block text-green-400 hover:text-green-300 mt-1"
              target={entry.href.startsWith("http") ? "_blank" : undefined}
              rel={entry.href.startsWith("http") ? "noopener noreferrer" : undefined}
            >
              Learn more →
            </a>
          )}
        </div>
      ) : (
        <p className="text-xs text-slate-400 leading-relaxed">
          This control is tagged for help but has no catalog entry yet (
          <code className="text-slate-500">{id}</code>).
        </p>
      )}
    </div>
  );
}
