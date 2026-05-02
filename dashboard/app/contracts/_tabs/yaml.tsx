"use client";

/**
 * YAML tab body for EditContractModal.
 * RFC-020: extracted from page.tsx §split.
 */

import clsx from "clsx";
import { TooltipWrap } from "../_lib";
import type { VersionSummary, VersionResponse } from "@/lib/api";

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
                <TooltipWrap
                  content={
                    v.state === "stable"
                      ? "A frozen, immutable version eligible to receive inbound traffic."
                      : v.state === "draft"
                      ? "A work-in-progress version. YAML is freely editable."
                      : "A retired version. No new unpinned traffic routes to it."
                  }
                  rfc="RFC-002"
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
                </TooltipWrap>
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
                  <TooltipWrap
                    key={section}
                    content={
                      section === "ontology"
                        ? "The named entities and field rules your contract enforces — every inbound event is validated against these definitions."
                        : section === "glossary"
                        ? "Human-readable descriptions of fields, including any compliance constraints attached to each one."
                        : "Named aggregate formulas (e.g. sum, count) computed over events that pass this contract."
                    }
                  >
                    <span className="text-[10px] uppercase tracking-wider text-slate-600 border border-[#1f2937] rounded px-2 py-0.5 cursor-default hover:text-slate-400 transition-colors">
                      {section}
                    </span>
                  </TooltipWrap>
                ) : null
              )}
              {currentVersion.compliance_mode && (
                <TooltipWrap content="When enabled, any inbound field not declared in the contract ontology is rejected. Nothing undeclared can enter the audit log." rfc="RFC-004">
                  <span className="text-[10px] uppercase tracking-wider text-amber-600 border border-amber-800/40 rounded px-2 py-0.5 cursor-default">
                    compliance mode
                  </span>
                </TooltipWrap>
              )}
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
