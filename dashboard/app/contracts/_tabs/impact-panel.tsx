"use client";

/**
 * Downstream impact for one contract version, plus lineage import.
 * The API is the gate: a Free org gets an upgrade message and no asset list.
 */

import { useCallback, useEffect, useState } from "react";
import Link from "next/link";
import {
  getVersionImpact,
  importLineage,
  recomputeVersionImpact,
  type ImpactReport,
  type VersionSummary,
} from "@/lib/api";

interface ImpactPanelProps {
  contractId: string;
  versions: VersionSummary[];
}

export function ImpactPanel({ contractId, versions }: ImpactPanelProps) {
  const newest = [...versions].sort((a, b) => b.created_at.localeCompare(a.created_at))[0];
  const [version, setVersion] = useState(newest?.version ?? "");
  const [report, setReport] = useState<ImpactReport | null>(null);
  const [upgrade, setUpgrade] = useState<string | null>(null);
  const [upgradeUrl, setUpgradeUrl] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [importing, setImporting] = useState(false);

  useEffect(() => {
    if (!version && newest) setVersion(newest.version);
  }, [newest, version]);

  const load = useCallback(async (selected: string) => {
    if (!selected) return;
    setLoading(true);
    setMessage(null);
    try {
      const result = await getVersionImpact(contractId, selected);
      if (result.status === "upgrade") {
        setReport(null);
        setUpgrade(result.message);
        setUpgradeUrl(result.upgrade_url ?? "/pricing");
        return;
      }
      setUpgrade(null);
      if (result.status === "ok") {
        setReport(result.report);
        return;
      }
      setReport(null);
      setMessage(result.message);
    } catch (err) {
      setReport(null);
      setUpgrade(null);
      setMessage(err instanceof Error ? err.message : "Impact request failed");
    } finally {
      setLoading(false);
    }
  }, [contractId]);

  useEffect(() => {
    if (version) void load(version);
  }, [version, load]);

  async function onFile(file: File) {
    setImporting(true);
    setMessage(null);
    try {
      const text = await file.text();
      const imported = await importLineage(text, "auto", contractId);
      if (imported.status === "upgrade") {
        setReport(null);
        setUpgrade(imported.message);
        setUpgradeUrl(imported.upgrade_url ?? "/pricing");
        return;
      }
      if (imported.status !== "ok") {
        setMessage(imported.message);
        return;
      }
      const note = `Imported ${imported.imported.assets} assets and ${imported.imported.edges} edges.`;
      setMessage(note);
      if (version) {
        const refreshed = await recomputeVersionImpact(contractId, version);
        if (refreshed.status === "upgrade") {
          setReport(null);
          setUpgrade(refreshed.message);
          setUpgradeUrl(refreshed.upgrade_url ?? "/pricing");
        } else if (refreshed.status === "ok") {
          setUpgrade(null);
          setReport(refreshed.report);
        } else {
          setMessage(`${note} ${refreshed.message}`);
        }
      }
    } catch (err) {
      setMessage(err instanceof Error ? err.message : "Lineage import failed");
    } finally {
      setImporting(false);
    }
  }

  return (
    <section
      data-testid="impact-panel"
      className="bg-[#0a0d12] border border-[#1f2937] rounded-xl p-4 space-y-4"
    >
      <div className="flex flex-wrap items-center gap-3">
        <h3 className="text-xs font-semibold text-slate-400 uppercase tracking-wider">
          Downstream impact
        </h3>
        {versions.length > 0 && (
          <label className="text-xs text-slate-500 flex items-center gap-2">
            Version
            <select
              data-testid="impact-version"
              value={version}
              onChange={(e) => setVersion(e.target.value)}
              className="bg-[#111827] border border-[#374151] text-slate-200 text-xs rounded-lg px-2 py-1"
            >
              {versions.map((v) => (
                <option key={v.version} value={v.version}>
                  v{v.version} ({v.state})
                </option>
              ))}
            </select>
          </label>
        )}
        <label className="ml-auto text-xs px-3 py-1.5 bg-[#1f2937] hover:bg-[#374151] text-slate-200 rounded-lg cursor-pointer">
          {importing ? "Importing…" : "Import lineage"}
          <input
            data-testid="lineage-file"
            type="file"
            accept=".json,.yaml,.yml,application/json,text/yaml"
            className="hidden"
            disabled={importing}
            onChange={(e) => {
              const file = e.target.files?.[0];
              if (file) void onFile(file);
              e.target.value = "";
            }}
          />
        </label>
      </div>
      <p className="text-xs text-slate-500">
        DataContractGate reads a dbt manifest.json or an edge list and, when a version is proposed or published, names the downstream assets and which field changes are breaking.
      </p>

      {upgrade && (
        <div
          data-testid="impact-upgrade"
          className="rounded-lg border border-amber-700/40 bg-amber-900/20 px-4 py-3 space-y-2"
        >
          <p className="text-sm text-amber-200">{upgrade}</p>
          <Link href={upgradeUrl ?? "/pricing"} className="text-sm text-amber-300 underline">
            Upgrade to Pro + Impact
          </Link>
        </div>
      )}

      {message && !upgrade && (
        <p data-testid="impact-message" className="text-xs text-slate-300">{message}</p>
      )}

      {loading && !upgrade && (
        <p className="text-xs text-slate-500">Loading impact…</p>
      )}

      {report && !upgrade && (
        <div className="space-y-3" data-testid="impact-report">
          <p className="text-sm text-slate-200">{report.summary}</p>
          <p className="text-[11px] text-slate-500">
            {report.breaking_count} breaking, {report.safe_count} safe
            {report.base_version ? ` compared with v${report.base_version}` : ""}.
          </p>
          {report.changes.length > 0 && (
            <ul className="space-y-1">
              {report.changes.map((change) => (
                <li key={`${change.field}:${change.kind}:${change.detail}`} className="text-xs text-slate-300 flex gap-2">
                  <span className={change.severity === "breaking" ? "text-rose-300" : "text-emerald-300"}>
                    {change.severity === "breaking" ? "Breaking" : "Safe"}
                  </span>
                  <span>{change.explanation}</span>
                </li>
              ))}
            </ul>
          )}
          {report.affected.length === 0 ? (
            <p className="text-xs text-slate-500">No downstream assets are registered for this contract.</p>
          ) : (
            <ul className="space-y-2">
              {report.affected.map((asset) => (
                <li
                  key={asset.key}
                  data-testid="impact-asset"
                  className="border border-[#1f2937] rounded-lg px-3 py-2"
                >
                  <p className="text-sm text-slate-100">
                    {asset.name}{" "}
                    <span className="text-slate-500">({asset.kind}, {asset.hops} {asset.hops === 1 ? "hop" : "hops"})</span>
                  </p>
                  <p className="text-xs text-slate-400 mt-1">{asset.explanation}</p>
                </li>
              ))}
            </ul>
          )}
        </div>
      )}
    </section>
  );
}
