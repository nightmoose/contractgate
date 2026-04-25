"use client";

import { useState, useEffect } from "react";
import { useRouter } from "next/navigation";
import useSWR, { mutate } from "swr";
import AuthGate from "@/components/AuthGate";
import { useOrg } from "@/lib/org";
import {
  listContracts,
  getContract,
  createContract,
  deleteContract,
  listVersions,
  getVersion,
  createVersion,
  patchVersionYaml,
  promoteVersion,
  deprecateVersion,
  deleteVersion,
  suggestNextVersion,
  listNameHistory,
  listQuarantinedEvents,
  replayEvents,
  getReplayHistory,
} from "@/lib/api";
import type {
  ContractSummary,
  ContractResponse,
  VersionSummary,
  VersionResponse,
  NameHistoryEntry,
  QuarantinedEvent,
  ReplayOutcome,
  ReplayResponse,
} from "@/lib/api";
import VisualBuilder from "./VisualBuilder";
import { EXAMPLE_YAML, EXAMPLE_SAMPLE } from "./examples";
import clsx from "clsx";

// ---------------------------------------------------------------------------
// Version-picker helpers — used by EditContractModal to pick a sensible
// default version on open and to find the newest version for suggestNextVersion.
// ---------------------------------------------------------------------------

/**
 * Prefer latest stable by promoted_at; fall back to latest draft by created_at;
 * final fallback is newest created_at of anything.  Matches the backend's
 * "latest stable" resolution order for the common case of opening a contract
 * fresh from the list.
 */
function pickDefaultVersion(vs: VersionSummary[]): string | null {
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
  return [...vs].sort((a, b) => b.created_at.localeCompare(a.created_at))[0]
    .version;
}

/** Newest version string by created_at — seed for `suggestNextVersion`. */
function newestVersionString(vs: VersionSummary[]): string | null {
  if (vs.length === 0) return null;
  return [...vs].sort((a, b) => b.created_at.localeCompare(a.created_at))[0]
    .version;
}

// EXAMPLE_YAML / EXAMPLE_SAMPLE moved to ./examples.ts to keep this file
// focused on the page component.  See that file for behavioural notes.

// ---------------------------------------------------------------------------
// Contract generator — client-side inference
// ---------------------------------------------------------------------------

interface InferredField {
  name: string;
  type: "string" | "integer" | "number" | "boolean";
  required: boolean;
  pattern?: string;
  enum?: string[];
  min?: number;
}

/** Well-known regex patterns we auto-detect. */
const PATTERNS: [RegExp, string][] = [
  [/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i, "^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$"],
  [/^[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}$/, "^[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\\.[a-zA-Z]{2,}$"],
  [/^\d{4}-\d{2}-\d{2}(T\d{2}:\d{2}:\d{2}(\.\d+)?(Z|[+-]\d{2}:\d{2})?)?$/, "^\\d{4}-\\d{2}-\\d{2}"],
  [/^https?:\/\//, "^https?:\\/\\/"],
  [/^[a-zA-Z0-9_-]{3,64}$/, "^[a-zA-Z0-9_-]{3,64}$"],
];

function sniffPattern(values: string[]): string | undefined {
  for (const [regex, pattern] of PATTERNS) {
    if (values.every((v) => regex.test(v))) return pattern;
  }
  return undefined;
}

function inferFields(records: Record<string, unknown>[]): InferredField[] {
  // Collect all keys across all records
  const allKeys = Array.from(new Set(records.flatMap((r) => Object.keys(r))));
  const totalRecords = records.length;

  return allKeys.map((key) => {
    const presentIn = records.filter((r) => key in r);
    const values = presentIn.map((r) => r[key]);
    const required = presentIn.length === totalRecords;

    // Determine type from values
    let type: InferredField["type"] = "string";
    const nonNull = values.filter((v) => v !== null && v !== undefined);

    if (nonNull.every((v) => typeof v === "boolean")) {
      type = "boolean";
    } else if (nonNull.every((v) => typeof v === "number")) {
      type = nonNull.every((v) => Number.isInteger(v)) ? "integer" : "number";
    } else {
      type = "string";
    }

    const field: InferredField = { name: key, type, required };

    // String-specific enrichment
    if (type === "string") {
      const strValues = nonNull.map((v) => String(v));
      const unique = Array.from(new Set(strValues));

      // Enum detection: ≤6 distinct values and all values seen more than once
      // (or total records is small)
      if (unique.length <= 6 && unique.length < totalRecords) {
        field.enum = unique.sort();
      } else {
        const pattern = sniffPattern(strValues);
        if (pattern) field.pattern = pattern;
      }
    }

    // Numeric range hint
    if (type === "integer" || type === "number") {
      const nums = nonNull.map((v) => v as number);
      const min = Math.min(...nums);
      if (min >= 0) field.min = 0; // suggest non-negative constraint
    }

    return field;
  });
}

function buildYaml(name: string, fields: InferredField[]): string {
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

  lines.push(``);
  lines.push(`glossary:`);
  for (const f of fields) {
    lines.push(`  - field: "${f.name}"`);
    lines.push(`    description: "Description of ${f.name}"`);
  }

  lines.push(``);
  lines.push(`metrics: []`);
  lines.push(``);

  return lines.join("\n");
}

// ---------------------------------------------------------------------------
// Edit Contract Modal
// ---------------------------------------------------------------------------

function EditContractModal({
  contractId,
  onClose,
  onSaved,
  onTestInPlayground,
}: {
  contractId: string;
  onClose: () => void;
  onSaved: () => void;
  onTestInPlayground: (yaml: string, contractId: string) => void;
}) {
  const [contract, setContract] = useState<ContractResponse | null>(null);
  const [versions, setVersions] = useState<VersionSummary[]>([]);
  const [selectedVersion, setSelectedVersion] = useState<string | null>(null);
  const [currentVersion, setCurrentVersion] = useState<VersionResponse | null>(
    null
  );
  const [yamlDraft, setYamlDraft] = useState<string>("");
  const [loading, setLoading] = useState(true);
  const [loadingVersion, setLoadingVersion] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);

  // GitHub sync state
  const [ghSyncing, setGhSyncing] = useState(false);
  const [ghSyncUrl, setGhSyncUrl] = useState<string | null>(null);
  const [ghSyncError, setGhSyncError] = useState<string | null>(null);

  // Modal-level tab state
  type ModalTab = "yaml" | "versions";
  const [modalTab, setModalTab] = useState<ModalTab>("yaml");
  const [nameHistory, setNameHistory] = useState<NameHistoryEntry[] | null>(null);
  const [loadingNameHistory, setLoadingNameHistory] = useState(false);

  const BASE = process.env.NEXT_PUBLIC_API_URL ?? "http://localhost:3001";
  const ingestUrl = `${BASE}/ingest/${contractId}`;

  const isDraft = currentVersion?.state === "draft";
  const dirty =
    currentVersion != null && yamlDraft !== currentVersion.yaml_content;

  // Load contract + version list on mount.
  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    Promise.all([getContract(contractId), listVersions(contractId)])
      .then(([c, vs]) => {
        if (cancelled) return;
        setContract(c);
        setVersions(vs);
        setSelectedVersion(pickDefaultVersion(vs));
      })
      .catch((e) => {
        if (!cancelled) setError(e instanceof Error ? e.message : String(e));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [contractId]);

  // When the selected version changes, fetch its YAML.  We always treat the
  // server's yaml_content as authoritative — local unsaved edits to a draft
  // are dropped if the user clicks onto another version (they chose to move).
  useEffect(() => {
    if (!selectedVersion) {
      setCurrentVersion(null);
      setYamlDraft("");
      return;
    }
    let cancelled = false;
    setLoadingVersion(true);
    setError(null);
    getVersion(contractId, selectedVersion)
      .then((v) => {
        if (cancelled) return;
        setCurrentVersion(v);
        setYamlDraft(v.yaml_content);
      })
      .catch((e) => {
        if (!cancelled) setError(e instanceof Error ? e.message : String(e));
      })
      .finally(() => {
        if (!cancelled) setLoadingVersion(false);
      });
    return () => {
      cancelled = true;
    };
  }, [contractId, selectedVersion]);

  const refreshVersions = async () => {
    const vs = await listVersions(contractId);
    setVersions(vs);
    return vs;
  };

  const handleSaveDraft = async () => {
    if (!currentVersion || currentVersion.state !== "draft") return;
    setSaving(true);
    setError(null);
    try {
      const v = await patchVersionYaml(
        contractId,
        currentVersion.version,
        yamlDraft
      );
      setCurrentVersion(v);
      setYamlDraft(v.yaml_content);
      await refreshVersions();
      await mutate("contracts");
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  };

  const handleForkAsDraft = async () => {
    // Create a new draft seeded from whatever YAML is in the editor right now.
    // If the user is viewing a stable/deprecated version, that's the stable's
    // YAML verbatim; if they were editing a draft, their in-flight edits are
    // carried forward into the new draft.
    setSaving(true);
    setError(null);
    try {
      const seed = newestVersionString(versions);
      const next = suggestNextVersion(seed);
      const v = await createVersion(contractId, {
        version: next,
        yaml_content: yamlDraft,
      });
      const vs = await refreshVersions();
      await mutate("contracts");
      // Find the freshly-created version and select it.
      const found = vs.find((row) => row.version === v.version);
      setSelectedVersion(found?.version ?? v.version);
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  };

  const handlePromote = async () => {
    if (!currentVersion || currentVersion.state !== "draft") return;
    if (dirty) {
      setError("Save draft changes before promoting.");
      return;
    }
    if (
      !confirm(
        `Promote v${currentVersion.version} to stable? This freezes the YAML forever.`
      )
    ) {
      return;
    }
    setSaving(true);
    setError(null);
    try {
      const v = await promoteVersion(contractId, currentVersion.version);
      setCurrentVersion(v);
      setYamlDraft(v.yaml_content);
      await refreshVersions();
      await mutate("contracts");
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  };

  const handleDeprecate = async () => {
    if (!currentVersion || currentVersion.state !== "stable") return;
    if (
      !confirm(
        `Deprecate v${currentVersion.version}?\n\n` +
          `Pinned traffic will still validate against this version.  New ` +
          `unpinned traffic routes to the next stable (or fails closed if ` +
          `none remains, depending on policy).`
      )
    ) {
      return;
    }
    setSaving(true);
    setError(null);
    try {
      const v = await deprecateVersion(contractId, currentVersion.version);
      setCurrentVersion(v);
      setYamlDraft(v.yaml_content);
      await refreshVersions();
      await mutate("contracts");
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  };

  const handleDeleteDraft = async () => {
    if (!currentVersion || currentVersion.state !== "draft") return;
    if (!confirm(`Delete draft v${currentVersion.version}? This cannot be undone.`)) {
      return;
    }
    setSaving(true);
    setError(null);
    try {
      await deleteVersion(contractId, currentVersion.version);
      const vs = await refreshVersions();
      await mutate("contracts");
      const pick = pickDefaultVersion(vs);
      setSelectedVersion(pick);
      if (!pick) {
        setCurrentVersion(null);
        setYamlDraft("");
      }
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  };

  const handleCopyEndpoint = () => {
    navigator.clipboard.writeText(ingestUrl);
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  };

  const handleGitHubSync = async () => {
    if (!currentVersion || !contract) return;
    setGhSyncing(true);
    setGhSyncError(null);
    setGhSyncUrl(null);
    try {
      const res = await fetch("/api/github/sync", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          contractId,
          contractName: contract.name,
          version: currentVersion.version,
          yamlContent: currentVersion.yaml_content,
        }),
      });
      const data = await res.json();
      if (!res.ok) {
        setGhSyncError(data.error ?? "GitHub sync failed");
      } else {
        setGhSyncUrl(data.url);
      }
    } catch {
      setGhSyncError("Network error — please try again");
    } finally {
      setGhSyncing(false);
    }
  };

  // Close on Escape key
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [onClose]);

  // Load name history the first time the Versions tab is opened
  useEffect(() => {
    if (modalTab !== "versions") return;
    if (nameHistory !== null) return;
    let cancelled = false;
    setLoadingNameHistory(true);
    listNameHistory(contractId)
      .then((h) => { if (!cancelled) setNameHistory(h); })
      .catch(() => { if (!cancelled) setNameHistory([]); })
      .finally(() => { if (!cancelled) setLoadingNameHistory(false); });
    return () => { cancelled = true; };
  }, [contractId, modalTab, nameHistory]);

  /** Promote a specific version by string — used by the Versions tab row buttons. */
  const handlePromoteVersion = async (version: string) => {
    if (!confirm(`Promote v${version} to stable? This freezes the YAML forever.`)) return;
    setSaving(true);
    setError(null);
    try {
      const v = await promoteVersion(contractId, version);
      if (currentVersion?.version === version) {
        setCurrentVersion(v);
        setYamlDraft(v.yaml_content);
      }
      await refreshVersions();
      await mutate("contracts");
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  };

  /** Deprecate a specific version by string — used by the Versions tab row buttons. */
  const handleDeprecateVersion = async (version: string) => {
    if (!confirm(
      `Deprecate v${version}?\n\n` +
      `Pinned traffic will still validate against this version. New ` +
      `unpinned traffic routes to the next stable (or fails closed if ` +
      `none remains, depending on policy).`
    )) return;
    setSaving(true);
    setError(null);
    try {
      const v = await deprecateVersion(contractId, version);
      if (currentVersion?.version === version) {
        setCurrentVersion(v);
        setYamlDraft(v.yaml_content);
      }
      await refreshVersions();
      await mutate("contracts");
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  };

  return (
    /* Backdrop */
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/70 backdrop-blur-sm p-4"
      onClick={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div className="bg-[#0f1623] border border-[#1f2937] rounded-2xl w-full max-w-4xl shadow-2xl flex flex-col max-h-[90vh]">
        {/* Header */}
        <div className="flex items-start justify-between p-6 border-b border-[#1f2937]">
          <div>
            <h2 className="text-lg font-semibold text-slate-100">
              {loading ? "Loading…" : contract?.name ?? "Contract"}
            </h2>
            {contract && (
              <p className="text-xs text-slate-500 font-mono mt-1">
                {contract.id}
                {contract.latest_stable_version && (
                  <span className="ml-2 text-slate-600">
                    · latest stable v{contract.latest_stable_version}
                  </span>
                )}
              </p>
            )}
          </div>
          <button
            onClick={onClose}
            className="text-slate-500 hover:text-slate-300 transition-colors text-xl leading-none ml-4"
            aria-label="Close"
          >
            ✕
          </button>
        </div>

        {/* Ingest endpoint strip */}
        {contract && (
          <div className="px-6 py-3 bg-[#111827] border-b border-[#1f2937] flex items-center gap-3">
            <span className="text-xs text-slate-500 uppercase tracking-wider font-medium shrink-0">
              Ingest URL
            </span>
            <code className="text-xs text-blue-400 font-mono truncate flex-1">
              {ingestUrl}
            </code>
            <button
              onClick={handleCopyEndpoint}
              className="shrink-0 px-3 py-1 text-xs bg-[#1f2937] hover:bg-[#374151] text-slate-300 rounded-lg transition-colors"
            >
              {copied ? "✔ Copied!" : "Copy"}
            </button>
            <button
              onClick={() => onTestInPlayground(yamlDraft, contractId)}
              disabled={!yamlDraft}
              className="shrink-0 px-3 py-1 text-xs bg-indigo-600 hover:bg-indigo-500 disabled:opacity-50 text-white rounded-lg transition-colors"
            >
              Test in Playground →
            </button>
          </div>
        )}

        {/* Modal tab bar — shown once contract metadata has loaded */}
        {!loading && contract && (
          <div className="flex gap-1 px-6 pt-4 border-b border-[#1f2937] bg-[#0f1623]">
            <button
              onClick={() => setModalTab("yaml")}
              className={clsx(
                "px-4 py-2 text-sm font-medium rounded-t-lg transition-colors border-b-2 -mb-px",
                modalTab === "yaml"
                  ? "text-slate-100 border-green-600"
                  : "text-slate-500 hover:text-slate-300 border-transparent"
              )}
            >
              YAML
            </button>
            <button
              onClick={() => setModalTab("versions")}
              className={clsx(
                "px-4 py-2 text-sm font-medium rounded-t-lg transition-colors border-b-2 -mb-px flex items-center gap-2",
                modalTab === "versions"
                  ? "text-slate-100 border-indigo-500"
                  : "text-slate-500 hover:text-slate-300 border-transparent"
              )}
            >
              Versions
              {versions.length > 0 && (
                <span className="text-[10px] bg-slate-700 text-slate-400 px-1.5 py-0.5 rounded-full">
                  {versions.length}
                </span>
              )}
            </button>
          </div>
        )}

        {/* Body */}
        <div className="flex-1 overflow-auto p-6 space-y-5">
          {loading ? (
            <div className="flex items-center justify-center h-48 text-slate-500 text-sm">
              Loading contract…
            </div>
          ) : modalTab === "yaml" ? (
            <>
              {/* Version picker */}
              <div>
                <div className="flex items-center justify-between mb-2">
                  <label className="text-xs font-medium text-slate-400 uppercase tracking-wider">
                    Versions ({versions.length})
                  </label>
                  <span className="text-xs text-slate-600">
                    Click a version to load its YAML
                  </span>
                </div>
                {versions.length === 0 ? (
                  <p className="text-xs text-slate-500 italic">
                    No versions yet.
                  </p>
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
                        <span
                          className={clsx(
                            "px-1.5 py-0.5 rounded text-[10px] uppercase tracking-wider font-sans",
                            v.state === "stable" &&
                              "bg-green-900/40 text-green-400",
                            v.state === "draft" &&
                              "bg-amber-900/40 text-amber-400",
                            v.state === "deprecated" &&
                              "bg-slate-800 text-slate-500"
                          )}
                        >
                          {v.state}
                        </span>
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
                        <span className="text-amber-400 normal-case">
                          draft — editable
                        </span>
                      ) : (
                        <span className="text-slate-500 normal-case">
                          {currentVersion.state} — read-only (fork to edit)
                        </span>
                      )}
                    </label>
                    <span className="text-xs text-slate-600">
                      Created{" "}
                      {new Date(currentVersion.created_at).toLocaleString()}
                      {currentVersion.promoted_at &&
                        ` · promoted ${new Date(
                          currentVersion.promoted_at
                        ).toLocaleString()}`}
                      {currentVersion.deprecated_at &&
                        ` · deprecated ${new Date(
                          currentVersion.deprecated_at
                        ).toLocaleString()}`}
                    </span>
                  </div>
                  <textarea
                    className={clsx(
                      "w-full h-80 font-mono text-sm p-4 rounded-lg border outline-none resize-y transition-colors",
                      isDraft
                        ? "bg-[#0a0d12] text-green-300 border-[#1f2937] focus:border-green-700"
                        : "bg-[#0a0d12]/70 text-slate-400 border-[#1f2937]/70 cursor-not-allowed"
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
          ) : (
            /* ── Versions tab ── */
            <div className="space-y-8">
              {/* All versions table */}
              <div>
                <h3 className="text-xs font-semibold text-slate-400 uppercase tracking-wider mb-3">
                  All Versions
                </h3>
                {versions.length === 0 ? (
                  <p className="text-xs text-slate-500 italic">No versions yet.</p>
                ) : (
                  <div className="space-y-2">
                    {[...versions]
                      .sort((a, b) => b.created_at.localeCompare(a.created_at))
                      .map((v) => (
                        <div
                          key={v.version}
                          className="flex items-center justify-between bg-[#0a0d12] border border-[#1f2937] rounded-xl px-4 py-3 gap-4"
                        >
                          {/* Left: version + state + timestamps */}
                          <div className="flex items-center gap-3 min-w-0">
                            <span className="font-mono text-sm text-slate-200 shrink-0">
                              v{v.version}
                            </span>
                            <span
                              className={clsx(
                                "shrink-0 px-1.5 py-0.5 rounded text-[10px] uppercase tracking-wider font-sans",
                                v.state === "stable" && "bg-green-900/40 text-green-400",
                                v.state === "draft" && "bg-amber-900/40 text-amber-400",
                                v.state === "deprecated" && "bg-slate-800 text-slate-500"
                              )}
                            >
                              {v.state}
                            </span>
                            <span className="text-xs text-slate-600 truncate">
                              Created {new Date(v.created_at).toLocaleString()}
                              {v.promoted_at &&
                                ` · promoted ${new Date(v.promoted_at).toLocaleString()}`}
                              {v.deprecated_at &&
                                ` · deprecated ${new Date(v.deprecated_at).toLocaleString()}`}
                            </span>
                          </div>

                          {/* Right: actions */}
                          <div className="flex items-center gap-2 shrink-0">
                            {v.state === "draft" && (
                              <button
                                onClick={() => handlePromoteVersion(v.version)}
                                disabled={saving}
                                className="px-3 py-1 text-xs bg-indigo-600 hover:bg-indigo-500 disabled:opacity-40 text-white rounded-lg transition-colors"
                              >
                                Promote
                              </button>
                            )}
                            {v.state === "stable" && (
                              <button
                                onClick={() => handleDeprecateVersion(v.version)}
                                disabled={saving}
                                className="px-3 py-1 text-xs bg-amber-900/30 hover:bg-amber-900/50 disabled:opacity-40 text-amber-300 rounded-lg transition-colors"
                              >
                                Deprecate
                              </button>
                            )}
                            <button
                              onClick={() => {
                                setSelectedVersion(v.version);
                                setModalTab("yaml");
                              }}
                              className="px-3 py-1 text-xs bg-[#1f2937] hover:bg-[#374151] text-slate-300 rounded-lg transition-colors"
                            >
                              View YAML →
                            </button>
                          </div>
                        </div>
                      ))}
                  </div>
                )}
              </div>

              {/* Name history */}
              <div>
                <h3 className="text-xs font-semibold text-slate-400 uppercase tracking-wider mb-3">
                  Name History
                </h3>
                {loadingNameHistory ? (
                  <p className="text-xs text-slate-500 animate-pulse">Loading…</p>
                ) : !nameHistory || nameHistory.length === 0 ? (
                  <p className="text-xs text-slate-500 italic">
                    No name changes recorded — the contract has always been called &ldquo;
                    {contract?.name ?? "…"}&rdquo;.
                  </p>
                ) : (
                  <div className="space-y-2">
                    {[...nameHistory]
                      .sort((a, b) => b.changed_at.localeCompare(a.changed_at))
                      .map((h) => (
                        <div
                          key={h.id}
                          className="flex items-center gap-3 text-xs bg-[#0a0d12] border border-[#1f2937] rounded-lg px-4 py-2.5"
                        >
                          <span className="font-mono text-slate-500 line-through">
                            {h.old_name}
                          </span>
                          <span className="text-slate-600">→</span>
                          <span className="font-mono text-slate-200">{h.new_name}</span>
                          <span className="ml-auto text-slate-600">
                            {new Date(h.changed_at).toLocaleString()}
                          </span>
                        </div>
                      ))}
                  </div>
                )}
              </div>

              {error && (
                <p className="text-sm text-red-400 bg-red-900/20 border border-red-800/40 rounded p-2">
                  {error}
                </p>
              )}
            </div>
          )}
        </div>

        {/* Footer actions */}
        {!loading && (currentVersion || modalTab === "versions") && (
          <div className="flex items-center gap-3 p-6 border-t border-[#1f2937] flex-wrap">
            {/* YAML-tab-only actions */}
            {modalTab === "yaml" && currentVersion && (
              isDraft ? (
                <>
                  <button
                    onClick={handleSaveDraft}
                    disabled={saving || !dirty}
                    className="px-4 py-2 bg-green-600 hover:bg-green-500 disabled:opacity-40 disabled:cursor-not-allowed text-white text-sm font-medium rounded-lg transition-colors"
                  >
                    {saving ? "Saving…" : dirty ? "Save Draft" : "Saved"}
                  </button>
                  <button
                    onClick={handlePromote}
                    disabled={saving || dirty}
                    title={
                      dirty
                        ? "Save draft changes before promoting"
                        : "Promote this draft to stable (irreversible)"
                    }
                    className="px-4 py-2 bg-indigo-600 hover:bg-indigo-500 disabled:opacity-40 disabled:cursor-not-allowed text-white text-sm font-medium rounded-lg transition-colors"
                  >
                    Promote to Stable
                  </button>
                  <button
                    onClick={handleDeleteDraft}
                    disabled={saving}
                    className="px-4 py-2 bg-red-900/30 hover:bg-red-900/50 disabled:opacity-40 text-red-400 text-sm font-medium rounded-lg transition-colors"
                  >
                    Delete Draft
                  </button>
                </>
              ) : (
                <>
                  <button
                    onClick={handleForkAsDraft}
                    disabled={saving}
                    className="px-4 py-2 bg-green-600 hover:bg-green-500 disabled:opacity-40 text-white text-sm font-medium rounded-lg transition-colors"
                  >
                    {saving ? "Forking…" : "Fork as New Draft"}
                  </button>
                  {currentVersion.state === "stable" && (
                    <button
                      onClick={handleDeprecate}
                      disabled={saving}
                      className="px-4 py-2 bg-amber-900/30 hover:bg-amber-900/50 disabled:opacity-40 text-amber-300 text-sm font-medium rounded-lg transition-colors"
                    >
                      Deprecate
                    </button>
                  )}
                  {/* GitHub sync — available for stable + deprecated versions */}
                  <button
                    onClick={handleGitHubSync}
                    disabled={ghSyncing || saving}
                    title="Commit this version's YAML to the configured GitHub repository"
                    className="flex items-center gap-1.5 px-3 py-2 bg-[#24292e] hover:bg-[#2f363d] disabled:opacity-40 text-slate-200 text-sm font-medium rounded-lg transition-colors border border-[#374151]"
                  >
                    <svg height="14" viewBox="0 0 16 16" width="14" fill="currentColor" aria-hidden="true">
                      <path d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.013 8.013 0 0016 8c0-4.42-3.58-8-8-8z" />
                    </svg>
                    {ghSyncing ? "Syncing…" : "Sync to GitHub"}
                  </button>
                  {ghSyncUrl && (
                    <a
                      href={ghSyncUrl}
                      target="_blank"
                      rel="noopener noreferrer"
                      className="text-xs text-green-400 hover:text-green-300 truncate max-w-xs"
                      title={ghSyncUrl}
                    >
                      ✓ View on GitHub →
                    </a>
                  )}
                  {ghSyncError && (
                    <span className="text-xs text-red-400 truncate max-w-xs" title={ghSyncError}>
                      ✕ {ghSyncError}
                    </span>
                  )}
                </>
              )
            )}
            <button
              onClick={() => { onSaved(); }}
              className="px-4 py-2 bg-[#1f2937] hover:bg-[#374151] text-slate-300 text-sm font-medium rounded-lg transition-colors"
            >
              Close
            </button>
            <span className="ml-auto text-xs text-slate-600">
              Press <kbd className="bg-[#1f2937] px-1 rounded">Esc</kbd> to close
            </span>
          </div>
        )}
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Sub-components
// ---------------------------------------------------------------------------

function ContractList({
  contracts,
  isLoading,
  onDelete,
  onEdit,
}: {
  contracts?: ContractSummary[];
  isLoading: boolean;
  onDelete: (id: string) => void;
  onEdit: (id: string) => void;
}) {
  if (isLoading) return <p className="text-slate-500 text-sm">Loading…</p>;
  if (!contracts || contracts.length === 0) {
    return (
      <div className="flex flex-col items-center justify-center h-64 text-slate-600">
        <p className="text-4xl mb-4">📋</p>
        <p className="text-sm">No contracts yet — create your first one above.</p>
      </div>
    );
  }
  return (
    <div className="space-y-3">
      {contracts.map((c) => (
        <div
          key={c.id}
          className="bg-[#111827] border border-[#1f2937] rounded-xl p-5 flex items-center justify-between"
        >
          <div>
            <div className="flex items-center gap-3 flex-wrap">
              <h3 className="font-semibold text-slate-200">{c.name}</h3>
              {c.latest_stable_version ? (
                <span className="text-xs px-2 py-0.5 rounded-full font-medium bg-green-900/40 text-green-400">
                  stable v{c.latest_stable_version}
                </span>
              ) : (
                <span className="text-xs px-2 py-0.5 rounded-full font-medium bg-amber-900/40 text-amber-400">
                  no stable yet
                </span>
              )}
              <span className="text-xs text-slate-500">
                {c.version_count} version{c.version_count === 1 ? "" : "s"}
              </span>
              {c.multi_stable_resolution === "fallback" && (
                <span
                  className="text-xs px-2 py-0.5 rounded-full bg-indigo-900/40 text-indigo-300"
                  title="Unpinned traffic falls back across stables on failure"
                >
                  fallback
                </span>
              )}
            </div>
            <p className="text-xs text-slate-600 mt-1 font-mono">{c.id}</p>
          </div>
          <div className="flex items-center gap-2">
            <button
              onClick={() => onEdit(c.id)}
              className="px-3 py-1.5 text-xs bg-indigo-900/30 hover:bg-indigo-900/50 text-indigo-400 rounded-lg transition-colors"
            >
              Edit / View
            </button>
            <button
              onClick={() => onDelete(c.id)}
              className="px-3 py-1.5 text-xs bg-red-900/30 hover:bg-red-900/50 text-red-400 rounded-lg transition-colors"
            >
              Delete
            </button>
          </div>
        </div>
      ))}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Quarantine tab
// ---------------------------------------------------------------------------

function QuarantineTab({ contracts }: { contracts?: ContractSummary[] }) {
  // Filters
  const [contractFilter, setContractFilter] = useState<string>("");

  // Multi-select
  const [selected, setSelected] = useState<Set<string>>(new Set());

  // Replay
  const [replayVersion, setReplayVersion] = useState<string>("");
  const [replaying, setReplaying] = useState(false);
  const [replayResult, setReplayResult] = useState<ReplayResponse | null>(null);
  const [replayError, setReplayError] = useState<string | null>(null);

  // Version list for the replay version picker
  const [pickerVersions, setPickerVersions] = useState<VersionSummary[]>([]);

  // Replay-history drawer
  const [drawerEventId, setDrawerEventId] = useState<string | null>(null);
  const [drawerHistory, setDrawerHistory] = useState<ReplayOutcome[] | null>(null);
  const [loadingHistory, setLoadingHistory] = useState(false);

  // Fetch versions when the contract filter changes (for the replay version picker)
  useEffect(() => {
    if (!contractFilter) {
      setPickerVersions([]);
      setReplayVersion("");
      return;
    }
    let cancelled = false;
    listVersions(contractFilter)
      .then((vs) => {
        if (cancelled) return;
        setPickerVersions(vs);
        // Default to the latest stable, falling back to the newest draft
        const stables = vs.filter((v) => v.state === "stable");
        const defaultV =
          stables.length > 0
            ? [...stables].sort((a, b) =>
                (b.promoted_at ?? "").localeCompare(a.promoted_at ?? "")
              )[0].version
            : vs[0]?.version ?? "";
        setReplayVersion(defaultV);
      })
      .catch(() => { if (!cancelled) setPickerVersions([]); });
    return () => { cancelled = true; };
  }, [contractFilter]);

  // Fetch quarantined events via SWR
  const swrKey = contractFilter ? `quarantine:${contractFilter}` : "quarantine:all";
  const { data: events, isLoading, mutate: mutateEvents } = useSWR<QuarantinedEvent[]>(
    swrKey,
    () => listQuarantinedEvents(contractFilter ? { contract_id: contractFilter, limit: 100 } : { limit: 100 }),
    { refreshInterval: 30_000 }
  );

  // Selection helpers
  const allIds = events?.map((e) => e.id) ?? [];
  const allSelected = allIds.length > 0 && allIds.every((id) => selected.has(id));
  const someSelected = selected.size > 0;

  const toggleAll = () =>
    setSelected(allSelected ? new Set() : new Set(allIds));

  const toggleOne = (id: string) => {
    const next = new Set(selected);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    setSelected(next);
  };

  const handleReplay = async () => {
    if (selected.size === 0) return;
    setReplaying(true);
    setReplayError(null);
    setReplayResult(null);
    try {
      const r = await replayEvents(Array.from(selected), {
        ...(replayVersion ? { version: replayVersion } : {}),
        ...(contractFilter ? { contract_id: contractFilter } : {}),
      });
      setReplayResult(r);
      await mutateEvents();
      setSelected(new Set());
    } catch (e: unknown) {
      setReplayError(e instanceof Error ? e.message : String(e));
    } finally {
      setReplaying(false);
    }
  };

  const handleOpenDrawer = async (eventId: string) => {
    setDrawerEventId(eventId);
    setDrawerHistory(null);
    setLoadingHistory(true);
    try {
      const history = await getReplayHistory({ event_id: eventId, limit: 20 });
      setDrawerHistory(history);
    } catch {
      setDrawerHistory([]);
    } finally {
      setLoadingHistory(false);
    }
  };

  const closeDrawer = () => {
    setDrawerEventId(null);
    setDrawerHistory(null);
  };

  return (
    <div className="space-y-5">
      {/* Filter bar */}
      <div className="flex items-center gap-3 flex-wrap">
        <span className="text-xs text-slate-500 uppercase tracking-wider whitespace-nowrap">
          Filter:
        </span>
        <select
          value={contractFilter}
          onChange={(e) => {
            setContractFilter(e.target.value);
            setSelected(new Set());
            setReplayResult(null);
          }}
          className="bg-[#111827] border border-[#1f2937] text-slate-300 text-sm rounded-lg px-3 py-1.5 outline-none focus:border-indigo-600"
        >
          <option value="">All contracts</option>
          {contracts?.map((c) => (
            <option key={c.id} value={c.id}>
              {c.name}
            </option>
          ))}
        </select>
        {contractFilter && (
          <button
            onClick={() => { setContractFilter(""); setSelected(new Set()); }}
            className="text-xs text-slate-500 hover:text-slate-300 transition-colors"
          >
            ✕ Clear filter
          </button>
        )}
      </div>

      {/* Replay action bar — shown when events are selected */}
      {someSelected && (
        <div className="flex items-center gap-3 flex-wrap bg-[#111827] border border-indigo-700/40 rounded-xl px-4 py-3">
          <span className="text-sm font-medium text-indigo-300">
            {selected.size} event{selected.size !== 1 ? "s" : ""} selected
          </span>

          {/* Version picker */}
          {pickerVersions.length > 0 && (
            <div className="flex items-center gap-2">
              <span className="text-xs text-slate-500 whitespace-nowrap">Replay against:</span>
              <select
                value={replayVersion}
                onChange={(e) => setReplayVersion(e.target.value)}
                className="bg-[#0a0d12] border border-[#1f2937] text-slate-300 text-xs rounded-lg px-2 py-1.5 outline-none focus:border-indigo-600"
              >
                {pickerVersions.map((v) => (
                  <option key={v.version} value={v.version}>
                    v{v.version} · {v.state}
                  </option>
                ))}
              </select>
            </div>
          )}

          <button
            onClick={handleReplay}
            disabled={replaying}
            className="px-4 py-1.5 bg-indigo-600 hover:bg-indigo-500 disabled:opacity-50 text-white text-sm font-medium rounded-lg transition-colors"
          >
            {replaying ? "Replaying…" : "▶ Replay"}
          </button>
          <button
            onClick={() => setSelected(new Set())}
            className="px-3 py-1.5 bg-[#1f2937] hover:bg-[#374151] text-slate-400 text-sm rounded-lg transition-colors"
          >
            Deselect all
          </button>
        </div>
      )}

      {/* Replay result card */}
      {replayResult && (
        <div className="bg-[#111827] border border-[#1f2937] rounded-xl p-4 flex items-start gap-4">
          <span className="text-xl">
            {replayResult.outcomes.every((o) => o.passed) ? "✅" : "⚠️"}
          </span>
          <div>
            <p className="text-sm font-medium text-slate-200">
              Replayed {replayResult.replayed} event{replayResult.replayed !== 1 ? "s" : ""}
            </p>
            <p className="text-xs text-slate-500 mt-0.5">
              {replayResult.outcomes.filter((o) => o.passed).length} passed ·{" "}
              {replayResult.outcomes.filter((o) => !o.passed).length} failed
            </p>
          </div>
          <button
            onClick={() => setReplayResult(null)}
            className="ml-auto text-slate-500 hover:text-slate-300 transition-colors text-sm"
          >
            ✕
          </button>
        </div>
      )}

      {replayError && (
        <p className="text-sm text-red-400 bg-red-900/20 border border-red-800/40 rounded p-2">
          {replayError}
        </p>
      )}

      {/* Events table */}
      {isLoading ? (
        <div className="flex items-center justify-center h-48 text-slate-500 text-sm">
          Loading quarantined events…
        </div>
      ) : !events || events.length === 0 ? (
        <div className="flex flex-col items-center justify-center h-64 text-slate-600">
          <p className="text-4xl mb-4">🔒</p>
          <p className="text-sm">
            No quarantined events
            {contractFilter ? " for this contract" : ""}.
          </p>
          <p className="text-xs mt-1">
            Events only land here when the backend quarantines on violation.
          </p>
        </div>
      ) : (
        <div className="bg-[#111827] border border-[#1f2937] rounded-xl overflow-hidden">
          <table className="w-full text-sm">
            <thead>
              <tr className="border-b border-[#1f2937] text-left">
                <th className="w-10 px-4 py-3">
                  <input
                    type="checkbox"
                    checked={allSelected}
                    onChange={toggleAll}
                    className="rounded border-[#374151] bg-[#0a0d12] accent-indigo-500 cursor-pointer"
                  />
                </th>
                <th className="px-3 py-3 text-xs font-medium text-slate-500 uppercase tracking-wider">
                  Time
                </th>
                <th className="px-3 py-3 text-xs font-medium text-slate-500 uppercase tracking-wider">
                  Contract
                </th>
                <th className="px-3 py-3 text-xs font-medium text-slate-500 uppercase tracking-wider">
                  Version
                </th>
                <th className="px-3 py-3 text-xs font-medium text-slate-500 uppercase tracking-wider">
                  Violations
                </th>
                <th className="px-3 py-3 text-xs font-medium text-slate-500 uppercase tracking-wider">
                  Source IP
                </th>
                <th className="px-3 py-3 text-xs font-medium text-slate-500 uppercase tracking-wider">
                  Replays
                </th>
                <th className="px-3 py-3 text-xs font-medium text-slate-500 uppercase tracking-wider">
                  History
                </th>
              </tr>
            </thead>
            <tbody className="divide-y divide-[#1f2937]">
              {events.map((ev) => {
                const contractName =
                  contracts?.find((c) => c.id === ev.contract_id)?.name ??
                  ev.contract_id.slice(0, 8) + "…";
                return (
                  <tr
                    key={ev.id}
                    className={clsx(
                      "transition-colors",
                      selected.has(ev.id)
                        ? "bg-indigo-900/10"
                        : "hover:bg-[#1f2937]/30"
                    )}
                  >
                    <td className="px-4 py-3">
                      <input
                        type="checkbox"
                        checked={selected.has(ev.id)}
                        onChange={() => toggleOne(ev.id)}
                        className="rounded border-[#374151] bg-[#0a0d12] accent-indigo-500 cursor-pointer"
                      />
                    </td>
                    <td className="px-3 py-3 text-xs text-slate-400 whitespace-nowrap">
                      {new Date(ev.quarantined_at).toLocaleString()}
                    </td>
                    <td className="px-3 py-3 text-xs text-slate-300 max-w-[140px] truncate">
                      {contractName}
                    </td>
                    <td className="px-3 py-3 text-xs font-mono text-slate-500">
                      {ev.contract_version ? `v${ev.contract_version}` : "—"}
                    </td>
                    <td className="px-3 py-3">
                      <span className="inline-flex items-center gap-1 text-xs bg-red-900/30 text-red-400 border border-red-800/30 rounded-full px-2 py-0.5">
                        {ev.violation_count}
                      </span>
                    </td>
                    <td className="px-3 py-3 text-xs text-slate-500 font-mono">
                      {ev.source_ip ?? "—"}
                    </td>
                    <td className="px-3 py-3 text-xs text-slate-500">
                      {ev.replay_count > 0 ? (
                        <span
                          className={clsx(
                            "font-medium",
                            ev.last_replay_passed === true
                              ? "text-green-400"
                              : ev.last_replay_passed === false
                              ? "text-red-400"
                              : "text-slate-400"
                          )}
                        >
                          {ev.replay_count}×
                        </span>
                      ) : (
                        <span className="text-slate-700">—</span>
                      )}
                    </td>
                    <td className="px-3 py-3">
                      <button
                        onClick={() => handleOpenDrawer(ev.id)}
                        className="text-xs text-indigo-400 hover:text-indigo-300 transition-colors"
                      >
                        History →
                      </button>
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      )}

      {/* Replay-history drawer */}
      {drawerEventId && (
        <>
          {/* Backdrop */}
          <div
            className="fixed inset-0 z-40 bg-black/40"
            onClick={closeDrawer}
          />
          {/* Drawer */}
          <div className="fixed top-0 right-0 h-full w-full md:w-[480px] bg-[#0d1117] border-l border-[#1f2937] z-50 shadow-2xl flex flex-col">
            {/* Drawer header */}
            <div className="flex items-center justify-between p-5 border-b border-[#1f2937]">
              <div>
                <h3 className="font-semibold text-slate-100">Replay History</h3>
                <p className="text-xs text-slate-600 font-mono mt-0.5">
                  {drawerEventId}
                </p>
              </div>
              <button
                onClick={closeDrawer}
                className="text-slate-500 hover:text-slate-300 transition-colors text-xl leading-none"
                aria-label="Close"
              >
                ✕
              </button>
            </div>

            {/* Drawer body */}
            <div className="flex-1 overflow-auto p-5">
              {loadingHistory ? (
                <div className="flex items-center justify-center h-32 text-slate-500 text-sm">
                  Loading history…
                </div>
              ) : !drawerHistory || drawerHistory.length === 0 ? (
                <div className="flex flex-col items-center justify-center h-32 text-slate-600">
                  <p className="text-3xl mb-3">📭</p>
                  <p className="text-sm">No replay attempts for this event yet.</p>
                </div>
              ) : (
                <div className="space-y-3">
                  {drawerHistory.map((h, i) => (
                    <div
                      key={i}
                      className={clsx(
                        "rounded-xl border p-4",
                        h.passed
                          ? "bg-green-900/20 border-green-800/30"
                          : "bg-red-900/20 border-red-800/30"
                      )}
                    >
                      <div className="flex items-center justify-between mb-2">
                        <span
                          className={clsx(
                            "text-sm font-semibold",
                            h.passed ? "text-green-400" : "text-red-400"
                          )}
                        >
                          {h.passed ? "✅ PASSED" : "❌ FAILED"}
                        </span>
                        <span className="text-xs text-slate-500 font-mono">
                          v{h.version}
                        </span>
                      </div>
                      <p className="text-xs text-slate-500">
                        {new Date(h.replayed_at).toLocaleString()}
                      </p>
                      {h.violations.length > 0 && (
                        <ul className="mt-3 space-y-1.5">
                          {h.violations.map((v, j) => (
                            <li
                              key={j}
                              className="text-xs bg-red-900/20 border border-red-800/30 rounded-lg px-3 py-2"
                            >
                              <span className="font-mono text-red-400">
                                {v.field}
                              </span>
                              <span className="text-slate-500 mx-1">·</span>
                              <span className="text-slate-300">{v.message}</span>
                            </li>
                          ))}
                        </ul>
                      )}
                    </div>
                  ))}
                </div>
              )}
            </div>
          </div>
        </>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Generator tab
// ---------------------------------------------------------------------------

function GeneratorTab({ onSaved }: { onSaved: () => void }) {
  const [sample, setSample] = useState(EXAMPLE_SAMPLE);
  const [contractName, setContractName] = useState("my_events");
  const [generatedYaml, setGeneratedYaml] = useState<string | null>(null);
  const [parseError, setParseError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);

  const handleGenerate = () => {
    setParseError(null);
    setGeneratedYaml(null);
    setSaveError(null);

    let parsed: unknown;
    try {
      parsed = JSON.parse(sample);
    } catch (e) {
      setParseError(`Invalid JSON: ${e instanceof Error ? e.message : String(e)}`);
      return;
    }

    // Accept a single object or an array
    const records: Record<string, unknown>[] = Array.isArray(parsed)
      ? parsed
      : [parsed as Record<string, unknown>];

    if (records.length === 0) {
      setParseError("Sample data is empty — paste at least one event.");
      return;
    }

    const fields = inferFields(records);
    const yaml = buildYaml(contractName, fields);
    setGeneratedYaml(yaml);
  };

  const handleSave = async () => {
    if (!generatedYaml) return;
    setSaving(true);
    setSaveError(null);
    try {
      await createContract(generatedYaml);
      await mutate("contracts");
      onSaved();
    } catch (e: unknown) {
      setSaveError(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="space-y-6">
      <p className="text-sm text-slate-400">
        Paste one or more sample events as JSON. The generator will infer field
        types, detect patterns, and produce a ready-to-edit YAML contract.
      </p>

      {/* Input row */}
      <div className="grid grid-cols-1 lg:grid-cols-2 gap-6">
        {/* Left: sample data */}
        <div>
          <div className="flex items-center justify-between mb-2">
            <label className="text-xs font-medium text-slate-400 uppercase tracking-wider">
              Sample Events (JSON)
            </label>
            <span className="text-xs text-slate-600">array or single object</span>
          </div>
          <textarea
            className="w-full h-72 bg-[#0a0d12] text-blue-300 font-mono text-sm p-4 rounded-lg border border-[#1f2937] outline-none focus:border-blue-700 resize-y"
            value={sample}
            onChange={(e) => {
              setSample(e.target.value);
              setGeneratedYaml(null);
              setParseError(null);
            }}
            spellCheck={false}
            placeholder='[{ "user_id": "alice", "event_type": "click" }]'
          />
          {parseError && (
            <p className="mt-2 text-sm text-red-400 bg-red-900/20 border border-red-800/40 rounded p-2">
              {parseError}
            </p>
          )}
        </div>

        {/* Right: generated YAML */}
        <div>
          <div className="flex items-center justify-between mb-2">
            <label className="text-xs font-medium text-slate-400 uppercase tracking-wider">
              Generated Contract (YAML)
            </label>
            {generatedYaml && (
              <span className="text-xs text-green-500">✔ ready to edit &amp; save</span>
            )}
          </div>
          <textarea
            className={clsx(
              "w-full h-72 font-mono text-sm p-4 rounded-lg border outline-none resize-y transition-colors",
              generatedYaml
                ? "bg-[#0a0d12] text-green-300 border-[#1f2937] focus:border-green-700"
                : "bg-[#0a0d12]/50 text-slate-600 border-[#1f2937]/50 cursor-not-allowed"
            )}
            value={generatedYaml ?? "// Generate a contract to see YAML here…"}
            onChange={(e) => setGeneratedYaml(e.target.value)}
            spellCheck={false}
            readOnly={!generatedYaml}
          />
          {saveError && (
            <p className="mt-2 text-sm text-red-400 bg-red-900/20 border border-red-800/40 rounded p-2">
              {saveError}
            </p>
          )}
        </div>
      </div>

      {/* Contract name + actions */}
      <div className="flex items-center gap-3 flex-wrap">
        <div className="flex items-center gap-2">
          <label className="text-xs text-slate-400 whitespace-nowrap">Contract name</label>
          <input
            type="text"
            value={contractName}
            onChange={(e) => setContractName(e.target.value)}
            className="bg-[#0a0d12] border border-[#1f2937] rounded-lg px-3 py-1.5 text-sm text-slate-200 outline-none focus:border-green-700 w-48"
            placeholder="my_events"
          />
        </div>

        <button
          onClick={handleGenerate}
          className="px-4 py-2 bg-blue-600 hover:bg-blue-500 text-white text-sm font-medium rounded-lg transition-colors"
        >
          ✦ Generate Contract
        </button>

        {generatedYaml && (
          <button
            onClick={handleSave}
            disabled={saving}
            className="px-4 py-2 bg-green-600 hover:bg-green-500 disabled:opacity-50 text-white text-sm font-medium rounded-lg transition-colors"
          >
            {saving ? "Saving…" : "Save Contract"}
          </button>
        )}

        {generatedYaml && (
          <button
            onClick={() => setGeneratedYaml(null)}
            className="px-4 py-2 bg-[#1f2937] hover:bg-[#374151] text-slate-300 text-sm font-medium rounded-lg transition-colors"
          >
            Reset
          </button>
        )}
      </div>

      {/* Inference legend */}
      {generatedYaml && (
        <div className="bg-[#111827] border border-[#1f2937] rounded-xl p-4">
          <p className="text-xs font-medium text-slate-400 uppercase tracking-wider mb-3">
            What was inferred
          </p>
          <div className="flex flex-wrap gap-x-6 gap-y-1 text-xs text-slate-500">
            <span>🔵 Types from JSON values (string / integer / number / boolean)</span>
            <span>🟢 Required = field present in every sample</span>
            <span>🟡 Enum = ≤6 distinct string values</span>
            <span>🔷 Pattern = UUID / email / date / URL / alphanumeric ID detected</span>
            <span>🟠 min: 0 suggested for non-negative numbers</span>
          </div>
        </div>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Manual create panel (existing behaviour, extracted)
// ---------------------------------------------------------------------------

function ManualCreatePanel({ onCancel, onCreated }: { onCancel: () => void; onCreated: () => void }) {
  const [yaml, setYaml] = useState(EXAMPLE_YAML);
  const [creating, setCreating] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const handleCreate = async () => {
    setCreating(true);
    setError(null);
    try {
      await createContract(yaml);
      await mutate("contracts");
      onCreated();
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setCreating(false);
    }
  };

  return (
    <div className="mb-8 bg-[#111827] border border-[#1f2937] rounded-xl p-6">
      <h2 className="text-base font-semibold mb-4">New Contract (YAML)</h2>
      <textarea
        className="w-full h-80 bg-[#0a0d12] text-green-300 font-mono text-sm p-4 rounded-lg border border-[#1f2937] outline-none focus:border-green-700 resize-y"
        value={yaml}
        onChange={(e) => setYaml(e.target.value)}
        spellCheck={false}
      />
      {error && (
        <p className="mt-2 text-sm text-red-400 bg-red-900/20 border border-red-800/40 rounded p-2">
          {error}
        </p>
      )}
      <div className="flex gap-3 mt-4">
        <button
          onClick={handleCreate}
          disabled={creating}
          className="px-4 py-2 bg-green-600 hover:bg-green-500 disabled:opacity-50 text-white text-sm font-medium rounded-lg transition-colors"
        >
          {creating ? "Creating…" : "Create Contract"}
        </button>
        <button
          onClick={onCancel}
          className="px-4 py-2 bg-[#1f2937] hover:bg-[#374151] text-slate-300 text-sm font-medium rounded-lg transition-colors"
        >
          Cancel
        </button>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Page
// ---------------------------------------------------------------------------

type Tab = "list" | "build" | "generate" | "quarantine";

function ContractsContent() {
  const router = useRouter();
  const { org } = useOrg();
  // Gate on org resolving so x-org-id is set before the first fetch fires.
  const { data: contracts, isLoading } = useSWR<ContractSummary[]>(
    org ? "contracts" : null,
    listContracts
  );
  const [tab, setTab] = useState<Tab>("list");
  const [showCreate, setShowCreate] = useState(false);
  const [editingId, setEditingId] = useState<string | null>(null);

  const handleDelete = async (id: string) => {
    if (!confirm("Delete this contract? This cannot be undone.")) return;
    await deleteContract(id);
    await mutate("contracts");
  };

  const handleTestInPlayground = (yaml: string, contractId: string) => {
    // Store in sessionStorage so the Playground page can pick it up
    sessionStorage.setItem("playground_yaml", yaml);
    sessionStorage.setItem("playground_contract_id", contractId);
    router.push("/playground");
  };

  return (
    <div>
      {/* Edit modal (portal-style overlay) */}
      {editingId && (
        <EditContractModal
          contractId={editingId}
          onClose={() => setEditingId(null)}
          onSaved={() => setEditingId(null)}
          onTestInPlayground={handleTestInPlayground}
        />
      )}

      {/* Header */}
      <div className="flex items-center justify-between mb-6">
        <div>
          <h1 className="text-2xl font-bold">Contracts</h1>
          <p className="text-sm text-slate-500 mt-1">
            Create and manage versioned semantic contracts
          </p>
        </div>
        {tab === "list" && (
          <button
            onClick={() => setShowCreate((v) => !v)}
            className="px-4 py-2 bg-green-600 hover:bg-green-500 text-white text-sm font-medium rounded-lg transition-colors"
          >
            + New Contract
          </button>
        )}
      </div>

      {/* Tabs */}
      <div className="flex gap-1 mb-6 bg-[#111827] border border-[#1f2937] rounded-xl p-1 w-fit">
        <button
          onClick={() => { setTab("list"); setShowCreate(false); }}
          className={clsx(
            "px-4 py-2 text-sm font-medium rounded-lg transition-colors",
            tab === "list"
              ? "bg-[#1f2937] text-slate-100"
              : "text-slate-500 hover:text-slate-300"
          )}
        >
          My Contracts
        </button>
        <button
          onClick={() => { setTab("build"); setShowCreate(false); }}
          className={clsx(
            "px-4 py-2 text-sm font-medium rounded-lg transition-colors flex items-center gap-2",
            tab === "build"
              ? "bg-[#1f2937] text-slate-100"
              : "text-slate-500 hover:text-slate-300"
          )}
        >
          🧱 Visual Builder
        </button>
        <button
          onClick={() => { setTab("generate"); setShowCreate(false); }}
          className={clsx(
            "px-4 py-2 text-sm font-medium rounded-lg transition-colors flex items-center gap-2",
            tab === "generate"
              ? "bg-[#1f2937] text-slate-100"
              : "text-slate-500 hover:text-slate-300"
          )}
        >
          <span>✦</span> Generate from Sample
        </button>
        <button
          onClick={() => { setTab("quarantine"); setShowCreate(false); }}
          className={clsx(
            "px-4 py-2 text-sm font-medium rounded-lg transition-colors flex items-center gap-2",
            tab === "quarantine"
              ? "bg-[#1f2937] text-slate-100"
              : "text-slate-500 hover:text-slate-300"
          )}
        >
          🔒 Quarantine
        </button>
      </div>

      {/* Tab content */}
      {tab === "list" && (
        <>
          {showCreate && (
            <ManualCreatePanel
              onCancel={() => setShowCreate(false)}
              onCreated={() => setShowCreate(false)}
            />
          )}
          <ContractList
            contracts={contracts}
            isLoading={isLoading}
            onDelete={handleDelete}
            onEdit={(id) => setEditingId(id)}
          />
        </>
      )}

      {tab === "build" && (
        <VisualBuilder onSaved={() => setTab("list")} />
      )}

      {tab === "generate" && (
        <div className="bg-[#111827] border border-[#1f2937] rounded-xl p-6">
          <GeneratorTab onSaved={() => setTab("list")} />
        </div>
      )}

      {tab === "quarantine" && (
        <div className="bg-[#111827] border border-[#1f2937] rounded-xl p-6">
          <div className="mb-5">
            <h2 className="text-base font-semibold text-slate-100">Quarantine</h2>
            <p className="text-sm text-slate-500 mt-1">
              Events that failed validation and were held for review. Select one or more to replay against any contract version.
            </p>
          </div>
          <QuarantineTab contracts={contracts} />
        </div>
      )}
    </div>
  );
}

export default function ContractsPage() {
  return (
    <AuthGate page="contracts">
      <ContractsContent />
    </AuthGate>
  );
}
