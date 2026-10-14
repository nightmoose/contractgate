"use client";

// The caller org's live publications with Unpublish (S-U1). Unpublish is
// only offered for publications nobody has imported yet; the API enforces
// the same rule (409).

import { useCallback, useEffect, useState } from "react";
import { listMyPublications, revokePublication, type MyPublication } from "@/lib/api";

export default function MyPublications({
  contractId,
  refreshKey = 0,
  title = "Your publications",
}: {
  contractId?: string;
  refreshKey?: number;
  title?: string;
}) {
  const [rows, setRows] = useState<MyPublication[] | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      const all = await listMyPublications();
      setRows(contractId ? all.filter((p) => p.contract_id === contractId) : all);
      setErr(null);
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    }
  }, [contractId]);

  useEffect(() => {
    load();
  }, [load, refreshKey]);

  const unpublish = async (p: MyPublication) => {
    const ok = window.confirm(
      `Unpublish ${p.contract_name} v${p.contract_version} (ref ${p.publication_ref})?\n\n` +
        "The ref and any link token stop working for previews and imports. This cannot be undone.",
    );
    if (!ok) return;
    setBusy(p.publication_ref);
    try {
      await revokePublication(p.publication_ref);
      await load();
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
      await load();
    } finally {
      setBusy(null);
    }
  };

  if (!err && (!rows || rows.length === 0)) return null;

  return (
    <div className="mt-4">
      <p className="text-xs font-medium text-slate-400 uppercase tracking-wider mb-2">{title}</p>
      {err && <p className="text-xs text-red-400 mb-2">✕ {err}</p>}
      <div className="divide-y divide-[#1f2937] border border-[#1f2937] rounded-lg overflow-hidden">
        {(rows ?? []).map((p) => (
          <div key={p.publication_ref} className="flex items-center justify-between gap-3 px-3 py-2">
            <div className="min-w-0 text-xs">
              <p className="text-slate-300 truncate">
                {p.contract_name} <span className="text-slate-500">v{p.contract_version}</span>
              </p>
              <p className="text-slate-600 font-mono truncate">
                {p.publication_ref} · {p.visibility} · {new Date(p.published_at).toLocaleDateString()}
              </p>
              {p.import_count > 0 && (
                <p className="text-amber-400/80 mt-0.5">
                  Imported {p.import_count} time{p.import_count === 1 ? "" : "s"} — can&apos;t be unpublished.
                  Promote a new version and publish it instead.
                </p>
              )}
            </div>
            {p.import_count === 0 && (
              <button
                type="button"
                onClick={() => unpublish(p)}
                disabled={busy === p.publication_ref}
                className="shrink-0 text-xs px-2.5 py-1 rounded border border-red-800/50 text-red-400 hover:bg-red-900/20 disabled:opacity-50"
              >
                {busy === p.publication_ref ? "Unpublishing…" : "Unpublish"}
              </button>
            )}
          </div>
        ))}
      </div>
    </div>
  );
}
