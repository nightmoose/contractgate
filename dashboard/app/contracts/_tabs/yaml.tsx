"use client";

/**
 * YAML tab body for EditContractModal.
 * RFC-020: extracted from page.tsx §split.
 */

import clsx from "clsx";
import { HelpTarget } from "@/components/help/HelpTarget";
import type { VersionSummary, VersionResponse, EgressLeakageMode } from "@/lib/api";

interface YamlTabProps {
  versions: VersionSummary[];
  selectedVersion: string | null;
  setSelectedVersion: (v: string | null) => void;
  currentVersion: VersionResponse | null;
  loadingVersion: boolean;
  yamlDraft: string;
  setYamlDraft: (s: string) => void;
  isDraft: boolean;
  error: string | null;
  setError: (e: string | null) => void;
  /** RFC-030: current leakage mode (from VersionResponse) */
  egressLeakageMode?: EgressLeakageMode;
  /** RFC-030: called when the user changes the leakage mode on a draft */
  onChangeLeakageMode?: (mode: EgressLeakageMode) => void;
  leakageSaving?: boolean;
}

export function YamlTab({
  versions,
  selectedVersion,
  setSelectedVersion,
  currentVersion,
  loadingVersion,
  yamlDraft,
  setYamlDraft,
  isDraft,
  error,
  setError,
  egressLeakageMode,
  onChangeLeakageMode,
  leakageSaving,
}: YamlTabProps) {
  return (
    <>
      {/* Version picker */}
      <div>
        <div className="flex items-center justify-between mb-2">
          <label className="text-xs font-medium text-slate-400 uppercase tracking-wider">
            Versions ({versions.length})
          </label>
          <span className="text-xs text-slate-600">Click a version to load its YAML</span>
        </div>
        {versions.length === 0 ? (
          <p className="text-xs text-slate-500 italic">No versions yet.</p>
        ) : (
          <div className="flex flex-wrap gap-2">
            {versions.map((v) => (
              <button
                key={v.version}
                onClick={() => setSelectedVersion(v.version)}
                className={clsx(
                  "px-3 py-1.5 text-xs font-mono rounded-lg border transition-colors inline-flex items-center gap-2",
                  v.version === selectedVersion
                    ? "bg-indigo-900/50 border-indigo-700 text-indigo-200"
                    : "bg-[#111827] border-[#1f2937] text-slate-400 hover:text-slate-200"
                )}
              >
                <span>v{v.version}</span>
                <HelpTarget
                  id={
                    v.state === "stable"
                      ? "term.stable"
                      : v.state === "draft"
                      ? "term.draft"
                      : "term.deprecated"
                  }
                >
                  <span
                    className={clsx(
                      "px-1.5 py-0.5 rounded text-[10px] uppercase tracking-wider font-sans cursor-default",
                      v.state === "stable" && "bg-green-900/40 text-green-400",
                      v.state === "draft" && "bg-amber-900/40 text-amber-400",
                      v.state === "deprecated" && "bg-slate-800 text-slate-500"
                    )}
                  >
                    {v.state}
                  </span>
                </HelpTarget>
              </button>
            ))}
          </div>
        )}
      </div>

      {/* YAML editor */}
      {currentVersion ? (
        <div>
          <div className="flex items-center justify-between mb-2">
            <label className="text-xs font-medium text-slate-400 uppercase tracking-wider">
              v{currentVersion.version} YAML ·{" "}
              {isDraft ? (
                <span className="text-amber-400 normal-case">draft — editable</span>
              ) : (
                <span className="text-slate-500 normal-case">
                  {currentVersion.state} — read-only (fork to edit)
                </span>
              )}
            </label>
            <span className="text-xs text-slate-600">
              Created {new Date(currentVersion.created_at).toLocaleString()}
              {currentVersion.promoted_at &&
                ` · promoted ${new Date(currentVersion.promoted_at).toLocaleString()}`}
              {currentVersion.deprecated_at &&
                ` · deprecated ${new Date(currentVersion.deprecated_at).toLocaleString()}`}
            </span>
          </div>

          {/* Inline YAML section tooltips */}
          {currentVersion.yaml_content && (
            <div className="flex gap-3 mb-2 flex-wrap">
              {["ontology", "glossary", "metrics"].map((section) =>
                currentVersion.yaml_content.includes(`${section}:`) ? (
                  <HelpTarget key={section} id={`term.${section}`}>
                    <span className="text-[10px] uppercase tracking-wider text-slate-600 border border-[#1f2937] rounded px-2 py-0.5 cursor-default hover:text-slate-400 transition-colors">
                      {section}
                    </span>
                  </HelpTarget>
                ) : null
              )}
              {currentVersion.compliance_mode && (
                <HelpTarget id="term.compliance-mode">
                  <span className="text-[10px] uppercase tracking-wider text-amber-600 border border-amber-800/40 rounded px-2 py-0.5 cursor-default">
                    compliance mode
                  </span>
                </HelpTarget>
              )}
              {isDraft && onChangeLeakageMode ? (
                <HelpTarget id="term.leakage">
                  <select
                    value={egressLeakageMode ?? "off"}
                    onChange={(e) => onChangeLeakageMode(e.target.value as EgressLeakageMode)}
                    disabled={leakageSaving}
                    className={clsx(
                      "text-[10px] uppercase tracking-wider rounded px-2 py-0.5 border cursor-pointer outline-none transition-colors",
                      egressLeakageMode && egressLeakageMode !== "off"
                        ? "bg-violet-900/30 border-violet-700/50 text-violet-300"
                        : "bg-[#1f2937] border-[#374151] text-slate-400 hover:text-slate-200"
                    )}
                  >
                    <option value="off">leakage: off</option>
                    <option value="strip">leakage: strip</option>
                    <option value="fail">leakage: fail</option>
                  </select>
                </HelpTarget>
              ) : egressLeakageMode && egressLeakageMode !== "off" ? (
                <HelpTarget id="term.leakage">
                  <span className="text-[10px] uppercase tracking-wider text-violet-400 border border-violet-800/40 rounded px-2 py-0.5 cursor-default">
                    leakage: {egressLeakageMode}
                  </span>
                </HelpTarget>
              ) : null}
            </div>
          )}

          <textarea
            className={clsx(
              "w-full h-80 font-mono text-sm p-4 rounded-lg border outline-none resize-y transition-colors",
              isDraft
                ? "bg-[#0a0d12] text-green-300 border-[#1f2937] focus:border-green-700"
                : "bg-[#0d1117] text-slate-300 border-[#1f2937] cursor-not-allowed"
            )}
            value={yamlDraft}
            onChange={(e) => {
              if (!isDraft) return;
              setYamlDraft(e.target.value);
              setError(null);
            }}
            spellCheck={false}
            readOnly={!isDraft}
          />
          {error && (
            <p className="mt-2 text-sm text-red-400 bg-red-900/20 border border-red-800/40 rounded p-2">
              {error}
            </p>
          )}
        </div>
      ) : loadingVersion ? (
        <div className="flex items-center justify-center h-48 text-slate-500 text-sm">
          Loading version…
        </div>
      ) : null}
    </>
  );
}
