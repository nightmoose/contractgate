"use client";

/**
 * Contracts page — orchestrating shell.
 * RFC-020: page.tsx split from 1860 lines into tab components.
 *
 * Tab components:
 *   _tabs/yaml.tsx        — YAML editor + version picker
 *   _tabs/versions.tsx    — Versions ladder, compare, diff drawer
 *   _tabs/quarantine.tsx  — Quarantine search/filter/replay
 *
 * Shared helpers/primitives:
 *   _lib.tsx              — TooltipWrap, ConfirmActionModal, ConfirmReplayModal,
 *                           ReplaySummaryModal, inferFields, buildYaml, etc.
 */

import { useState, useEffect, useMemo } from "react";
import { useRouter } from "next/navigation";
import useSWR, { mutate } from "swr";
import AuthGate from "@/components/AuthGate";
import { useOrg } from "@/lib/org";
import { createClient } from "@/lib/supabase/client";
import {
  listContracts,
  getContract,
  createContract,
  deleteContract,
  listVersions,
  getVersion,
  createVersion,
  patchVersionYaml,
  patchVersionLeakageMode,
  promoteVersion,
  deprecateVersion,
  deleteVersion,
  suggestNextVersion,
  listNameHistory,
  importOdcs,
  publishVersion,
  getImportStatus,
  fetchPublished,
  importPublished,
  inferCsv,
} from "@/lib/api";
import type {
  ContractSummary,
  ContractResponse,
  VersionSummary,
  VersionResponse,
  NameHistoryEntry,
  PublicationVisibility,
  PublishResponse,
  ImportStatusResult,
  EgressLeakageMode,
  ImportMode,
} from "@/lib/api";
import VisualBuilder from "./VisualBuilder";
import { EXAMPLE_YAML, EXAMPLE_SAMPLE } from "./examples";
import { YamlTab } from "./_tabs/yaml";
import { VersionsTab } from "./_tabs/versions";
import { QuarantineTab } from "./_tabs/quarantine";
import { KafkaTab } from "./_tabs/kafka";
import { KinesisTab } from "./_tabs/kinesis";
import { CollaborateTab } from "./_tabs/collaborate";
import {
  pickDefaultVersion,
  newestVersionString,
  inferFields,
  buildYaml,
  InferredField,
} from "./_lib";
import clsx from "clsx";

// Re-export for VisualBuilder / tests that import from this file
export type { InferredField };

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
  const [currentVersion, setCurrentVersion] = useState<VersionResponse | null>(null);
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

  // Publish modal state (RFC-032)
  const [publishModalOpen, setPublishModalOpen] = useState(false);

  // Modal-level tab state
  type ModalTab = "yaml" | "versions" | "kafka" | "kinesis" | "collaborate";
  const [modalTab, setModalTab] = useState<ModalTab>("yaml");

  // RFC-030: egress leakage mode
  const [leakageSaving, setLeakageSaving] = useState(false);
  const [nameHistory, setNameHistory] = useState<NameHistoryEntry[] | null>(null);
  const [loadingNameHistory, setLoadingNameHistory] = useState(false);

  const BASE = process.env.NEXT_PUBLIC_API_URL ?? "http://localhost:3001";
  const ingestUrl = `${BASE}/ingest/${contractId}`;

  const isDraft = currentVersion?.state === "draft";
  const dirty = currentVersion != null && yamlDraft !== currentVersion.yaml_content;

  // Load contract + version list on mount
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
      .catch((e) => { if (!cancelled) setError(e instanceof Error ? e.message : String(e)); })
      .finally(() => { if (!cancelled) setLoading(false); });
    return () => { cancelled = true; };
  }, [contractId]);

  // Load YAML when selected version changes
  useEffect(() => {
    if (!selectedVersion) { setCurrentVersion(null); setYamlDraft(""); return; }
    let cancelled = false;
    setLoadingVersion(true);
    setError(null);
    getVersion(contractId, selectedVersion)
      .then((v) => { if (cancelled) return; setCurrentVersion(v); setYamlDraft(v.yaml_content); })
      .catch((e) => { if (!cancelled) setError(e instanceof Error ? e.message : String(e)); })
      .finally(() => { if (!cancelled) setLoadingVersion(false); });
    return () => { cancelled = true; };
  }, [contractId, selectedVersion]);

  const refreshVersions = async () => {
    const vs = await listVersions(contractId);
    setVersions(vs);
    return vs;
  };

  const handleSaveDraft = async () => {
    if (!currentVersion || currentVersion.state !== "draft") return;
    setSaving(true); setError(null);
    try {
      const v = await patchVersionYaml(contractId, currentVersion.version, yamlDraft);
      setCurrentVersion(v); setYamlDraft(v.yaml_content);
      await refreshVersions(); await mutate("contracts");
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    } finally { setSaving(false); }
  };

  const handleForkAsDraft = async () => {
    setSaving(true); setError(null);
    try {
      const seed = newestVersionString(versions);
      const next = suggestNextVersion(seed);
      const v = await createVersion(contractId, { version: next, yaml_content: yamlDraft });
      const vs = await refreshVersions(); await mutate("contracts");
      const found = vs.find((row) => row.version === v.version);
      setSelectedVersion(found?.version ?? v.version);
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    } finally { setSaving(false); }
  };

  const handlePromote = async () => {
    if (!currentVersion || currentVersion.state !== "draft") return;
    if (dirty) { setError("Save draft changes before promoting."); return; }
    // Handled by VersionsTab ConfirmActionModal; this path is from footer button
    setSaving(true); setError(null);
    try {
      const v = await promoteVersion(contractId, currentVersion.version);
      setCurrentVersion(v); setYamlDraft(v.yaml_content);
      await refreshVersions(); await mutate("contracts");
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    } finally { setSaving(false); }
  };

  const handleDeprecate = async () => {
    if (!currentVersion || currentVersion.state !== "stable") return;
    setSaving(true); setError(null);
    try {
      const v = await deprecateVersion(contractId, currentVersion.version);
      setCurrentVersion(v); setYamlDraft(v.yaml_content);
      await refreshVersions(); await mutate("contracts");
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    } finally { setSaving(false); }
  };

  const handleDeleteDraft = async () => {
    if (!currentVersion || currentVersion.state !== "draft") return;
    if (!confirm(`Delete draft v${currentVersion.version}? This cannot be undone.`)) return;
    setSaving(true); setError(null);
    try {
      await deleteVersion(contractId, currentVersion.version);
      const vs = await refreshVersions(); await mutate("contracts");
      const pick = pickDefaultVersion(vs);
      setSelectedVersion(pick);
      if (!pick) { setCurrentVersion(null); setYamlDraft(""); }
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    } finally { setSaving(false); }
  };

  const handleCopyEndpoint = () => {
    navigator.clipboard.writeText(ingestUrl);
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  };

  const handleGitHubSync = async () => {
    if (!currentVersion || !contract) return;
    setGhSyncing(true); setGhSyncError(null); setGhSyncUrl(null);
    try {
      const res = await fetch("/api/github/sync", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          contractId, contractName: contract.name,
          version: currentVersion.version, yamlContent: currentVersion.yaml_content,
        }),
      });
      const data = await res.json();
      if (!res.ok) setGhSyncError(data.error ?? "GitHub sync failed");
      else setGhSyncUrl(data.url);
    } catch { setGhSyncError("Network error — please try again"); }
    finally { setGhSyncing(false); }
  };

  // Close on Escape
  useEffect(() => {
    const handler = (e: KeyboardEvent) => { if (e.key === "Escape") onClose(); };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [onClose]);

  // Load name history on first Versions tab open
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

  /** RFC-030: called from YamlTab leakage mode selector */
  const handleChangeLeakageMode = async (mode: EgressLeakageMode) => {
    if (!currentVersion) return;
    setLeakageSaving(true);
    try {
      const v = await patchVersionLeakageMode(contractId, currentVersion.version, mode);
      setCurrentVersion(v);
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    } finally { setLeakageSaving(false); }
  };

  /** Used by VersionsTab promote button */
  const handlePromoteVersion = async (version: string) => {
    setSaving(true); setError(null);
    try {
      const v = await promoteVersion(contractId, version);
      if (currentVersion?.version === version) { setCurrentVersion(v); setYamlDraft(v.yaml_content); }
      await refreshVersions(); await mutate("contracts");
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    } finally { setSaving(false); }
  };

  /** Used by VersionsTab deprecate button */
  const handleDeprecateVersion = async (version: string) => {
    setSaving(true); setError(null);
    try {
      const v = await deprecateVersion(contractId, version);
      if (currentVersion?.version === version) { setCurrentVersion(v); setYamlDraft(v.yaml_content); }
      await refreshVersions(); await mutate("contracts");
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    } finally { setSaving(false); }
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/70 backdrop-blur-sm p-4"
      onClick={(e) => { if (e.target === e.currentTarget) onClose(); }}
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
            <code className="text-xs text-blue-400 font-mono truncate flex-1">{ingestUrl}</code>
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

        {/* Tab bar */}
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
            <button
              onClick={() => setModalTab("kafka")}
              className={clsx(
                "px-4 py-2 text-sm font-medium rounded-t-lg transition-colors border-b-2 -mb-px",
                modalTab === "kafka"
                  ? "text-slate-100 border-emerald-500"
                  : "text-slate-500 hover:text-slate-300 border-transparent"
              )}
            >
              Kafka
            </button>
            <button
              onClick={() => setModalTab("kinesis")}
              className={clsx(
                "px-4 py-2 text-sm font-medium rounded-t-lg transition-colors border-b-2 -mb-px",
                modalTab === "kinesis"
                  ? "text-slate-100 border-emerald-500"
                  : "text-slate-500 hover:text-slate-300 border-transparent"
              )}
            >
              Kinesis
            </button>
            {/* RFC-033: Collaborate tab */}
            <button
              onClick={() => setModalTab("collaborate")}
              className={clsx(
                "px-4 py-2 text-sm font-medium rounded-t-lg transition-colors border-b-2 -mb-px",
                modalTab === "collaborate"
                  ? "text-slate-100 border-indigo-400"
                  : "text-slate-500 hover:text-slate-300 border-transparent"
              )}
            >
              👥 Collaborate
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
            <YamlTab
              versions={versions}
              selectedVersion={selectedVersion}
              setSelectedVersion={setSelectedVersion}
              currentVersion={currentVersion}
              loadingVersion={loadingVersion}
              yamlDraft={yamlDraft}
              setYamlDraft={setYamlDraft}
              isDraft={isDraft}
              error={error}
              setError={setError}
              egressLeakageMode={currentVersion?.egress_leakage_mode}
              onChangeLeakageMode={handleChangeLeakageMode}
              leakageSaving={leakageSaving}
            />
          ) : modalTab === "kafka" ? (
            <KafkaTab contractId={contractId} />
          ) : modalTab === "kinesis" ? (
            <KinesisTab contractId={contractId} />
          ) : modalTab === "collaborate" ? (
            <CollaborateTab
              contractName={contract?.name ?? contractId}
              contractCurrentYaml={yamlDraft}
              onProposalApplied={(yaml) => {
                setYamlDraft(yaml);
                setModalTab("yaml");
              }}
            />
          ) : (
            <VersionsTab
              contractId={contractId}
              contract={contract}
              versions={versions}
              currentVersion={currentVersion}
              saving={saving}
              error={error}
              setError={setError}
              nameHistory={nameHistory}
              loadingNameHistory={loadingNameHistory}
              onPromoteVersion={handlePromoteVersion}
              onDeprecateVersion={handleDeprecateVersion}
              onViewYaml={(version) => { setSelectedVersion(version); setModalTab("yaml"); }}
            />
          )}
        </div>

        {/* RFC-032: Publish Modal */}
        {publishModalOpen && currentVersion && (
          <PublishModal
            contractId={contractId}
            version={currentVersion.version}
            onClose={() => setPublishModalOpen(false)}
          />
        )}

        {/* Footer actions */}
        {!loading && (currentVersion || modalTab === "versions") && (
          <div className="flex items-center gap-3 p-6 border-t border-[#1f2937] flex-wrap">
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
                    title={dirty ? "Save draft changes before promoting" : "Promote this draft to stable (irreversible)"}
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
                  {/* RFC-032: Publish button */}
                  <button
                    onClick={() => setPublishModalOpen(true)}
                    disabled={saving}
                    title="Publish this contract version so others can import it by reference"
                    className="flex items-center gap-1.5 px-3 py-2 bg-teal-900/30 hover:bg-teal-900/50 disabled:opacity-40 text-teal-300 text-sm font-medium rounded-lg transition-colors border border-teal-800/50"
                  >
                    ↑ Publish
                  </button>
                  {ghSyncUrl && (
                    <a href={ghSyncUrl} target="_blank" rel="noopener noreferrer"
                      className="text-xs text-green-400 hover:text-green-300 truncate max-w-xs" title={ghSyncUrl}>
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
// PublishModal (RFC-032)
// ---------------------------------------------------------------------------

function PublishModal({
  contractId,
  version,
  onClose,
}: {
  contractId: string;
  version: string;
  onClose: () => void;
}) {
  const [visibility, setVisibility] = useState<PublicationVisibility>("link");
  const [publishing, setPublishing] = useState(false);
  const [result, setResult] = useState<PublishResponse | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [tokenCopied, setTokenCopied] = useState(false);
  const [refCopied, setRefCopied] = useState(false);

  const handlePublish = async () => {
    setPublishing(true); setErr(null);
    try {
      const res = await publishVersion(contractId, version, { visibility });
      setResult(res);
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    } finally { setPublishing(false); }
  };

  const copy = (text: string, cb: (v: boolean) => void) => {
    navigator.clipboard.writeText(text);
    cb(true); setTimeout(() => cb(false), 2000);
  };

  return (
    <div
      className="fixed inset-0 z-[60] flex items-center justify-center bg-black/60 backdrop-blur-sm"
      onClick={(e) => { if (e.target === e.currentTarget) onClose(); }}
    >
      <div className="bg-[#0f1623] border border-[#1f2937] rounded-2xl w-full max-w-md p-6 shadow-2xl">
        <div className="flex items-center justify-between mb-4">
          <h3 className="text-base font-semibold text-slate-100">
            Publish v{version}
          </h3>
          <button onClick={onClose} className="text-slate-500 hover:text-slate-300 text-xl leading-none">✕</button>
        </div>

        {!result ? (
          <>
            <p className="text-sm text-slate-400 mb-4">
              Publishing generates a stable reference that others can use to import
              this contract version directly — no manual reconstruction needed.
            </p>

            <div className="mb-5">
              <label className="text-xs font-medium text-slate-400 uppercase tracking-wider mb-2 block">
                Visibility
              </label>
              <div className="space-y-2">
                {(["public", "link"] as PublicationVisibility[]).map((v) => (
                  <label key={v} className="flex items-start gap-3 cursor-pointer group">
                    <input
                      type="radio"
                      name="visibility"
                      value={v}
                      checked={visibility === v}
                      onChange={() => setVisibility(v)}
                      className="mt-0.5 accent-teal-500"
                    />
                    <div>
                      <span className="text-sm font-medium text-slate-200">
                        {v === "public" ? "Public" : "Link-only"}
                      </span>
                      <p className="text-xs text-slate-500">
                        {v === "public"
                          ? "Anyone with the publication ref can fetch this contract."
                          : "Requires both the ref and an unguessable token — safe to share privately."}
                      </p>
                    </div>
                  </label>
                ))}
              </div>
            </div>

            {err && <p className="text-xs text-red-400 mb-3">✕ {err}</p>}

            <div className="flex gap-2">
              <button
                onClick={handlePublish}
                disabled={publishing}
                className="flex-1 px-4 py-2 bg-teal-700 hover:bg-teal-600 disabled:opacity-40 text-white text-sm font-medium rounded-lg transition-colors"
              >
                {publishing ? "Publishing…" : "Publish"}
              </button>
              <button
                onClick={onClose}
                className="px-4 py-2 bg-[#1f2937] hover:bg-[#374151] text-slate-300 text-sm font-medium rounded-lg transition-colors"
              >
                Cancel
              </button>
            </div>
          </>
        ) : (
          <>
            <div className="bg-teal-900/20 border border-teal-800/40 rounded-lg p-4 mb-4">
              <p className="text-xs font-medium text-teal-300 uppercase tracking-wider mb-3">
                ✓ Published — share these details
              </p>

              <div className="space-y-3">
                <div>
                  <p className="text-xs text-slate-500 mb-1">Publication Ref</p>
                  <div className="flex items-center gap-2">
                    <code className="text-xs text-slate-200 font-mono bg-[#0a0d12] px-2 py-1 rounded flex-1 truncate">
                      {result.publication_ref}
                    </code>
                    <button
                      onClick={() => copy(result.publication_ref, setRefCopied)}
                      className="shrink-0 px-2 py-1 text-xs bg-[#1f2937] hover:bg-[#374151] text-slate-300 rounded transition-colors"
                    >
                      {refCopied ? "✔" : "Copy"}
                    </button>
                  </div>
                </div>

                {result.link_token && (
                  <div>
                    <p className="text-xs text-slate-500 mb-1">Link Token <span className="text-amber-400">(shown once — save it)</span></p>
                    <div className="flex items-center gap-2">
                      <code className="text-xs text-amber-300 font-mono bg-[#0a0d12] px-2 py-1 rounded flex-1 truncate">
                        {result.link_token}
                      </code>
                      <button
                        onClick={() => copy(result.link_token!, setTokenCopied)}
                        className="shrink-0 px-2 py-1 text-xs bg-[#1f2937] hover:bg-[#374151] text-slate-300 rounded transition-colors"
                      >
                        {tokenCopied ? "✔" : "Copy"}
                      </button>
                    </div>
                  </div>
                )}

                <div className="flex gap-4 text-xs text-slate-500 pt-1">
                  <span>Visibility: <span className="text-slate-300">{result.visibility}</span></span>
                  <span>Version: <span className="text-slate-300">v{result.contract_version}</span></span>
                </div>
              </div>
            </div>

            <button
              onClick={onClose}
              className="w-full px-4 py-2 bg-[#1f2937] hover:bg-[#374151] text-slate-300 text-sm font-medium rounded-lg transition-colors"
            >
              Done
            </button>
          </>
        )}
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Deploy metadata — sourced from active_contracts_public (RFC-028)
// ---------------------------------------------------------------------------

interface DeployMeta {
  name: string;
  version: string;
  source: string | null;
  deployed_at: string | null;
  deployed_by: string | null;
}

/** Query active_contracts_public and return a map keyed by contract name. */
function useDeployMeta(): Map<string, DeployMeta> {
  const [meta, setMeta] = useState<Map<string, DeployMeta>>(new Map());

  useEffect(() => {
    const supabase = createClient();
    supabase
      .from("active_contracts_public")
      .select("name, version, source, deployed_at, deployed_by")
      .then(({ data }) => {
        if (!data) return;
        const m = new Map<string, DeployMeta>();
        for (const row of data as DeployMeta[]) {
          // Keep the newest deployed_at per name if multiple stable versions exist
          const existing = m.get(row.name);
          if (!existing || (row.deployed_at ?? "") > (existing.deployed_at ?? "")) {
            m.set(row.name, row);
          }
        }
        setMeta(m);
      });
  }, []);

  return meta;
}

function fmtDeployedAt(iso: string | null): string {
  if (!iso) return "";
  const d = new Date(iso);
  return d.toLocaleDateString(undefined, { month: "short", day: "numeric", year: "numeric" });
}

// ---------------------------------------------------------------------------
// useImportStatuses — RFC-032: poll import-status for all subscribe contracts
// ---------------------------------------------------------------------------

function useImportStatuses(contracts: ContractSummary[] | undefined): Map<string, ImportStatusResult> {
  const [statuses, setStatuses] = useState<Map<string, ImportStatusResult>>(new Map());

  useEffect(() => {
    if (!contracts || contracts.length === 0) return;
    let cancelled = false;

    Promise.all(
      contracts.map((c) =>
        getImportStatus(c.id)
          .then((s) => ({ id: c.id, status: s }))
          .catch(() => null)
      )
    ).then((results) => {
      if (cancelled) return;
      const m = new Map<string, ImportStatusResult>();
      for (const r of results) {
        if (r && r.status.import_mode === "subscribe") {
          m.set(r.id, r.status);
        }
      }
      setStatuses(m);
    });

    return () => { cancelled = true; };
  }, [contracts]);

  return statuses;
}

// ---------------------------------------------------------------------------
// ConsumedContractsList — RFC-032 imported contracts view
// ---------------------------------------------------------------------------

function ConsumedContractsList({
  contracts,
  isLoading,
  onEdit,
}: {
  contracts?: ContractSummary[];
  isLoading: boolean;
  onEdit: (id: string) => void;
}) {
  const importStatuses = useImportStatuses(contracts);

  if (isLoading) return <p className="text-slate-500 text-sm">Loading…</p>;

  // Only show contracts that have an import_mode (snapshot or subscribe)
  const consumed = (contracts ?? []).filter((c) => {
    const st = importStatuses.get(c.id);
    return st && st.import_mode !== null;
  });

  if (consumed.length === 0) {
    return (
      <div className="flex flex-col items-center justify-center h-64 text-slate-600">
        <p className="text-4xl mb-4">📥</p>
        <p className="text-sm">No consumed contracts yet.</p>
        <p className="text-xs mt-2">
          Use <span className="text-teal-400 font-medium">↓ Import from Ref</span> to add one.
        </p>
      </div>
    );
  }

  return (
    <div className="space-y-3">
      {consumed.map((c) => {
        const st = importStatuses.get(c.id)!;
        return (
          <div
            key={c.id}
            className="bg-[#111827] border border-[#1f2937] rounded-xl p-5 flex items-center justify-between"
          >
            <div className="min-w-0 flex-1 mr-4">
              <div className="flex items-center gap-3 flex-wrap">
                <h3 className="font-semibold text-slate-200">{c.name}</h3>
                {c.latest_stable_version && (
                  <span className="text-xs px-2 py-0.5 rounded-full bg-green-900/40 text-green-400">
                    v{c.latest_stable_version}
                  </span>
                )}
                <span className="text-xs px-2 py-0.5 rounded-full bg-teal-900/30 text-teal-300 border border-teal-800/40">
                  {st.import_mode}
                </span>
                {st.update_available && (
                  <span
                    className="text-xs px-2 py-0.5 rounded-full bg-teal-900/40 text-teal-300 border border-teal-700/50 font-medium"
                    title={`Provider published v${st.latest_published_version}`}
                  >
                    ↑ Update available
                  </span>
                )}
                {st.source_revoked && (
                  <span className="text-xs px-2 py-0.5 rounded-full bg-red-900/30 text-red-400">
                    source revoked
                  </span>
                )}
              </div>
              <p className="text-xs text-slate-600 font-mono mt-1.5">
                ref: {st.publication_ref ?? "—"}
                {st.imported_version && (
                  <span className="text-slate-500 ml-2">· imported v{st.imported_version}</span>
                )}
              </p>
            </div>
            <button
              onClick={() => onEdit(c.id)}
              className="px-3 py-1.5 text-xs bg-indigo-900/30 hover:bg-indigo-900/50 text-indigo-400 rounded-lg transition-colors shrink-0"
            >
              View
            </button>
          </div>
        );
      })}
    </div>
  );
}

// ---------------------------------------------------------------------------
// ContractList
// ---------------------------------------------------------------------------

function ContractList({
  contracts,
  isLoading,
  deployMeta,
  onDelete,
  onEdit,
}: {
  contracts?: ContractSummary[];
  isLoading: boolean;
  deployMeta: Map<string, DeployMeta>;
  onDelete: (id: string) => void;
  onEdit: (id: string) => void;
}) {
  const importStatuses = useImportStatuses(contracts);
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
      {contracts.map((c) => {
        const dm = deployMeta.get(c.name);
        const importStatus = importStatuses.get(c.id);
        return (
          <div
            key={c.id}
            className="bg-[#111827] border border-[#1f2937] rounded-xl p-5 flex items-center justify-between"
          >
            <div className="min-w-0 flex-1 mr-4">
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
                {/* RFC-028 deploy metadata badges */}
                {dm?.source && (
                  <span className="text-xs px-2 py-0.5 rounded-full bg-sky-900/40 text-sky-300 font-medium">
                    {dm.source}
                  </span>
                )}
                {/* RFC-032: update available badge for subscribed imports */}
                {importStatus?.update_available && (
                  <span
                    className="text-xs px-2 py-0.5 rounded-full bg-teal-900/40 text-teal-300 font-medium border border-teal-800/50"
                    title={`Provider published v${importStatus.latest_published_version} — open the contract to pull the update`}
                  >
                    ↑ Update available
                  </span>
                )}
                {importStatus?.source_revoked && (
                  <span
                    className="text-xs px-2 py-0.5 rounded-full bg-red-900/30 text-red-400"
                    title="The source publication has been revoked"
                  >
                    source revoked
                  </span>
                )}
              </div>
              <div className="flex items-center gap-3 mt-1.5 flex-wrap">
                <p className="text-xs text-slate-600 font-mono">{c.id}</p>
                {dm?.deployed_at && (
                  <span
                    className="text-xs text-slate-500"
                    title={`Deployed ${dm.deployed_at}${dm.deployed_by ? ` by ${dm.deployed_by}` : ""}`}
                  >
                    deployed {fmtDeployedAt(dm.deployed_at)}
                    {dm.deployed_by && (
                      <span className="text-slate-600"> · {dm.deployed_by}</span>
                    )}
                  </span>
                )}
              </div>
            </div>
            <div className="flex items-center gap-2 shrink-0">
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
        );
      })}
    </div>
  );
}

// ---------------------------------------------------------------------------
// GeneratorTab
// ---------------------------------------------------------------------------

function GeneratorTab({ onSaved }: { onSaved: () => void }) {
  const [sample, setSample] = useState(EXAMPLE_SAMPLE);
  const [contractName, setContractName] = useState("my_events");
  const [generatedYaml, setGeneratedYaml] = useState<string | null>(null);
  const [parseError, setParseError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);

  const handleGenerate = () => {
    setParseError(null); setGeneratedYaml(null); setSaveError(null);
    let parsed: unknown;
    try { parsed = JSON.parse(sample); }
    catch (e) { setParseError(`Invalid JSON: ${e instanceof Error ? e.message : String(e)}`); return; }
    const records: Record<string, unknown>[] = Array.isArray(parsed)
      ? parsed : [parsed as Record<string, unknown>];
    if (records.length === 0) { setParseError("Sample data is empty — paste at least one event."); return; }
    setGeneratedYaml(buildYaml(contractName, inferFields(records)));
  };

  const handleSave = async () => {
    if (!generatedYaml) return;
    setSaving(true); setSaveError(null);
    try { await createContract(generatedYaml); await mutate("contracts"); onSaved(); }
    catch (e: unknown) { setSaveError(e instanceof Error ? e.message : String(e)); }
    finally { setSaving(false); }
  };

  return (
    <div className="space-y-6">
      <p className="text-sm text-slate-400">
        Paste one or more sample events as JSON. The generator will infer field types, detect patterns,
        and produce a ready-to-edit YAML contract.
      </p>
      <div className="grid grid-cols-1 lg:grid-cols-2 gap-6">
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
            onChange={(e) => { setSample(e.target.value); setGeneratedYaml(null); setParseError(null); }}
            spellCheck={false}
            placeholder='[{ "user_id": "alice", "event_type": "click" }]'
          />
          {parseError && (
            <p className="mt-2 text-sm text-red-400 bg-red-900/20 border border-red-800/40 rounded p-2">
              {parseError}
            </p>
          )}
        </div>
        <div>
          <div className="flex items-center justify-between mb-2">
            <label className="text-xs font-medium text-slate-400 uppercase tracking-wider">
              Generated Contract (YAML)
            </label>
            {generatedYaml && <span className="text-xs text-green-500">✔ ready to edit &amp; save</span>}
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
          <button onClick={handleSave} disabled={saving}
            className="px-4 py-2 bg-green-600 hover:bg-green-500 disabled:opacity-50 text-white text-sm font-medium rounded-lg transition-colors">
            {saving ? "Saving…" : "Save Contract"}
          </button>
        )}
        {generatedYaml && (
          <button onClick={() => setGeneratedYaml(null)}
            className="px-4 py-2 bg-[#1f2937] hover:bg-[#374151] text-slate-300 text-sm font-medium rounded-lg transition-colors">
            Reset
          </button>
        )}
      </div>
      {generatedYaml && (
        <div className="bg-[#111827] border border-[#1f2937] rounded-xl p-4">
          <p className="text-xs font-medium text-slate-400 uppercase tracking-wider mb-3">What was inferred</p>
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
// CsvGeneratorTab (RFC-035)
// ---------------------------------------------------------------------------

type CsvInputMode = "paste" | "upload";

function CsvGeneratorTab({ onSaved }: { onSaved: () => void }) {
  const [inputMode, setInputMode] = useState<CsvInputMode>("paste");
  const [csvText, setCsvText] = useState("");
  const [fileName, setFileName] = useState<string | null>(null);
  const [contractName, setContractName] = useState("my_contract");
  const [generatedYaml, setGeneratedYaml] = useState<string | null>(null);
  const [fieldCount, setFieldCount] = useState<number | null>(null);
  const [rowCount, setRowCount] = useState<number | null>(null);
  const [inferring, setInferring] = useState(false);
  const [inferError, setInferError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);

  const handleFileChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (!file) return;
    setFileName(file.name);
    // Derive contract name from filename (strip extension)
    const base = file.name.replace(/\.[^.]+$/, "").replace(/[^a-zA-Z0-9_]/g, "_").toLowerCase();
    if (base) setContractName(base);
    const reader = new FileReader();
    reader.onload = (ev) => {
      setCsvText(ev.target?.result as string ?? "");
      setGeneratedYaml(null);
      setInferError(null);
    };
    reader.readAsText(file);
  };

  const handleInfer = async () => {
    const content = csvText.trim();
    if (!content) { setInferError("Paste or upload a CSV first."); return; }
    setInferring(true); setInferError(null); setGeneratedYaml(null);
    try {
      const res = await inferCsv({ name: contractName, csv_content: content });
      setGeneratedYaml(res.yaml_content);
      setFieldCount(res.field_count);
      setRowCount(res.sample_count);
    } catch (e: unknown) {
      setInferError(e instanceof Error ? e.message : String(e));
    } finally { setInferring(false); }
  };

  const handleSave = async () => {
    if (!generatedYaml) return;
    setSaving(true); setSaveError(null);
    try { await createContract(generatedYaml); await mutate("contracts"); onSaved(); }
    catch (e: unknown) { setSaveError(e instanceof Error ? e.message : String(e)); }
    finally { setSaving(false); }
  };

  return (
    <div className="space-y-6">
      <p className="text-sm text-slate-400">
        Upload or paste a CSV file. The backend auto-detects delimiters and infers field types,
        producing a ready-to-edit YAML contract.
      </p>

      {/* Input mode tabs */}
      <div className="flex gap-1 bg-[#0a0d12] border border-[#1f2937] rounded-lg p-1 w-fit">
        {(["paste", "upload"] as CsvInputMode[]).map((m) => (
          <button
            key={m}
            onClick={() => { setInputMode(m); setGeneratedYaml(null); setInferError(null); }}
            className={clsx(
              "px-4 py-1.5 text-sm font-medium rounded-md transition-colors",
              inputMode === m ? "bg-[#1f2937] text-slate-100" : "text-slate-500 hover:text-slate-300"
            )}
          >
            {m === "paste" ? "📋 Paste" : "📁 Upload"}
          </button>
        ))}
      </div>

      <div className="grid grid-cols-1 lg:grid-cols-2 gap-6">
        {/* Left: CSV input */}
        <div>
          <div className="flex items-center justify-between mb-2">
            <label className="text-xs font-medium text-slate-400 uppercase tracking-wider">
              {inputMode === "paste" ? "CSV Content" : "CSV File"}
            </label>
            {csvText && <span className="text-xs text-slate-600">{csvText.split("\n").length} lines</span>}
          </div>

          {inputMode === "paste" ? (
            <textarea
              className="w-full h-72 bg-[#0a0d12] text-orange-300 font-mono text-sm p-4 rounded-lg border border-[#1f2937] outline-none focus:border-orange-700 resize-y"
              value={csvText}
              onChange={(e) => { setCsvText(e.target.value); setGeneratedYaml(null); setInferError(null); }}
              spellCheck={false}
              placeholder={"id,name,amount,created_at\n1,Alice,99.99,2024-01-01\n2,Bob,149.00,2024-01-02"}
            />
          ) : (
            <div className="flex flex-col items-center justify-center h-72 border-2 border-dashed border-[#2d3748] rounded-lg bg-[#0a0d12] gap-3">
              <span className="text-4xl">📊</span>
              <p className="text-sm text-slate-400">
                {fileName ? (
                  <span className="text-orange-400 font-mono">{fileName}</span>
                ) : (
                  "Select a CSV file"
                )}
              </p>
              {csvText && (
                <p className="text-xs text-slate-500">{csvText.split("\n").length} lines loaded</p>
              )}
              <label className="px-4 py-2 bg-[#1f2937] hover:bg-[#374151] text-slate-300 text-sm font-medium rounded-lg cursor-pointer transition-colors">
                Browse…
                <input
                  type="file"
                  accept=".csv,.tsv,.txt"
                  className="hidden"
                  onChange={handleFileChange}
                />
              </label>
            </div>
          )}

          {inferError && (
            <p className="mt-2 text-sm text-red-400 bg-red-900/20 border border-red-800/40 rounded p-2">
              {inferError}
            </p>
          )}
        </div>

        {/* Right: generated YAML */}
        <div>
          <div className="flex items-center justify-between mb-2">
            <label className="text-xs font-medium text-slate-400 uppercase tracking-wider">
              Generated Contract (YAML)
            </label>
            {generatedYaml && fieldCount !== null && (
              <span className="text-xs text-green-500">
                ✔ {fieldCount} field{fieldCount !== 1 ? "s" : ""} · {rowCount} row{rowCount !== 1 ? "s" : ""}
              </span>
            )}
          </div>
          <textarea
            className={clsx(
              "w-full h-72 font-mono text-sm p-4 rounded-lg border outline-none resize-y transition-colors",
              generatedYaml
                ? "bg-[#0a0d12] text-green-300 border-[#1f2937] focus:border-green-700"
                : "bg-[#0a0d12]/50 text-slate-600 border-[#1f2937]/50 cursor-not-allowed"
            )}
            value={generatedYaml ?? "// Infer a contract to see YAML here…"}
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

      {/* Controls row */}
      <div className="flex items-center gap-3 flex-wrap">
        <div className="flex items-center gap-2">
          <label className="text-xs text-slate-400 whitespace-nowrap">Contract name</label>
          <input
            type="text"
            value={contractName}
            onChange={(e) => setContractName(e.target.value)}
            className="bg-[#0a0d12] border border-[#1f2937] rounded-lg px-3 py-1.5 text-sm text-slate-200 outline-none focus:border-orange-700 w-48"
            placeholder="my_contract"
          />
        </div>
        <button
          onClick={handleInfer}
          disabled={inferring || !csvText.trim()}
          className="px-4 py-2 bg-orange-600 hover:bg-orange-500 disabled:opacity-40 text-white text-sm font-medium rounded-lg transition-colors"
        >
          {inferring ? "Inferring…" : "✦ Infer from CSV"}
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
            onClick={() => { setGeneratedYaml(null); setFieldCount(null); setRowCount(null); }}
            className="px-4 py-2 bg-[#1f2937] hover:bg-[#374151] text-slate-300 text-sm font-medium rounded-lg transition-colors"
          >
            Reset
          </button>
        )}
      </div>

      {generatedYaml && (
        <div className="bg-[#111827] border border-[#1f2937] rounded-xl p-4">
          <p className="text-xs font-medium text-slate-400 uppercase tracking-wider mb-3">What was inferred</p>
          <div className="flex flex-wrap gap-x-6 gap-y-1 text-xs text-slate-500">
            <span>🔵 Delimiter auto-detected (comma / tab / semicolon)</span>
            <span>🔵 Types from CSV values (string / integer / number / boolean)</span>
            <span>🟢 Required = field present in every row</span>
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
// OdcsImportModal
// ---------------------------------------------------------------------------

type ImportTab = "paste" | "upload";

function OdcsImportModal({
  onClose,
  onImported,
}: {
  onClose: () => void;
  onImported: () => void;
}) {
  const [activeTab, setActiveTab] = useState<ImportTab>("paste");
  const [pasteYaml, setPasteYaml] = useState("");
  const [fileYaml, setFileYaml] = useState<string | null>(null);
  const [fileName, setFileName] = useState<string | null>(null);
  const [importing, setImporting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [success, setSuccess] = useState<string | null>(null);

  const handleFileChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (!file) return;
    setFileName(file.name);
    const reader = new FileReader();
    reader.onload = (ev) => {
      setFileYaml(ev.target?.result as string);
    };
    reader.readAsText(file);
  };

  const handleImport = async () => {
    const yaml = activeTab === "paste" ? pasteYaml : fileYaml;
    if (!yaml?.trim()) {
      setError("Please provide ODCS YAML before importing.");
      return;
    }
    setImporting(true);
    setError(null);
    setSuccess(null);
    try {
      const result = await importOdcs(yaml);
      await mutate("contracts");
      const reviewNote =
        result.requires_review
          ? " — review required before promotion (stripped ODCS import)."
          : ".";
      setSuccess(`Imported v${result.version}${reviewNote}`);
      setTimeout(() => {
        onImported();
      }, 1800);
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setImporting(false);
    }
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/70 p-4">
      <div className="w-full max-w-2xl bg-[#0f1117] border border-[#1f2937] rounded-2xl shadow-2xl overflow-hidden">
        {/* Header */}
        <div className="flex items-center justify-between px-6 py-4 border-b border-[#1f2937]">
          <div>
            <h2 className="text-base font-semibold text-slate-100">Import ODCS Contract</h2>
            <p className="text-xs text-slate-500 mt-0.5">Paste or upload an ODCS v3.1.0 YAML document</p>
          </div>
          <button
            onClick={onClose}
            className="text-slate-500 hover:text-slate-300 text-xl leading-none"
            aria-label="Close"
          >
            ×
          </button>
        </div>

        {/* Tabs */}
        <div className="flex border-b border-[#1f2937]">
          {(["paste", "upload"] as ImportTab[]).map((t) => (
            <button
              key={t}
              onClick={() => setActiveTab(t)}
              className={clsx(
                "px-6 py-3 text-sm font-medium border-b-2 transition-colors",
                activeTab === t
                  ? "border-blue-500 text-blue-400"
                  : "border-transparent text-slate-500 hover:text-slate-300"
              )}
            >
              {t === "paste" ? "📋 Paste YAML" : "📁 Upload File"}
            </button>
          ))}
        </div>

        {/* Body */}
        <div className="p-6">
          {activeTab === "paste" && (
            <textarea
              className="w-full h-72 bg-[#0a0d12] text-green-300 font-mono text-sm p-4 rounded-lg border border-[#1f2937] outline-none focus:border-blue-700 resize-y"
              placeholder={"apiVersion: v3.1.0\nkind: DataContract\n…"}
              value={pasteYaml}
              onChange={(e) => setPasteYaml(e.target.value)}
              spellCheck={false}
            />
          )}

          {activeTab === "upload" && (
            <div className="flex flex-col items-center justify-center h-72 border-2 border-dashed border-[#2d3748] rounded-lg bg-[#0a0d12] gap-3">
              <span className="text-4xl">📄</span>
              <p className="text-sm text-slate-400">
                {fileName ? (
                  <span className="text-green-400 font-mono">{fileName}</span>
                ) : (
                  "Select an ODCS YAML file"
                )}
              </p>
              <label className="px-4 py-2 bg-[#1f2937] hover:bg-[#374151] text-slate-300 text-sm font-medium rounded-lg cursor-pointer transition-colors">
                Browse…
                <input
                  type="file"
                  accept=".yaml,.yml"
                  className="hidden"
                  onChange={handleFileChange}
                />
              </label>
              {fileYaml && (
                <p className="text-xs text-slate-500">{fileYaml.split("\n").length} lines loaded</p>
              )}
            </div>
          )}

          {error && (
            <p className="mt-3 text-sm text-red-400 bg-red-900/20 border border-red-800/40 rounded p-2">
              {error}
            </p>
          )}
          {success && (
            <p className="mt-3 text-sm text-green-400 bg-green-900/20 border border-green-800/40 rounded p-2">
              ✓ {success}
            </p>
          )}
        </div>

        {/* Footer */}
        <div className="flex justify-end gap-3 px-6 py-4 border-t border-[#1f2937]">
          <button
            onClick={onClose}
            className="px-4 py-2 bg-[#1f2937] hover:bg-[#374151] text-slate-300 text-sm font-medium rounded-lg transition-colors"
          >
            Cancel
          </button>
          <button
            onClick={handleImport}
            disabled={importing || !!success}
            className="px-4 py-2 bg-blue-600 hover:bg-blue-500 disabled:opacity-50 text-white text-sm font-medium rounded-lg transition-colors"
          >
            {importing ? "Importing…" : "Import Contract"}
          </button>
        </div>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// ImportFromRefModal (RFC-032 consumer import)
// ---------------------------------------------------------------------------

function ImportFromRefModal({
  onClose,
  onImported,
}: {
  onClose: () => void;
  onImported: () => void;
}) {
  const [ref, setRef] = useState("");
  const [token, setToken] = useState("");
  const [mode, setMode] = useState<ImportMode>("snapshot");
  const [preview, setPreview] = useState<import("@/lib/api").FetchedPublication | null>(null);
  const [fetching, setFetching] = useState(false);
  const [importing, setImporting] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const [success, setSuccess] = useState<string | null>(null);

  const handlePreview = async () => {
    if (!ref.trim()) { setErr("Publication ref is required."); return; }
    setFetching(true); setErr(null); setPreview(null);
    try {
      const p = await fetchPublished(ref.trim(), token.trim() || undefined);
      setPreview(p);
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    } finally { setFetching(false); }
  };

  const handleImport = async () => {
    if (!ref.trim()) return;
    setImporting(true); setErr(null);
    try {
      await importPublished({
        publication_ref: ref.trim(),
        ...(token.trim() ? { link_token: token.trim() } : {}),
        import_mode: mode,
      });
      await mutate("contracts");
      setSuccess(`Imported as ${mode} — contract added to your list.`);
      setTimeout(() => { onImported(); }, 1800);
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    } finally { setImporting(false); }
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/70 p-4">
      <div className="w-full max-w-2xl bg-[#0f1117] border border-[#1f2937] rounded-2xl shadow-2xl overflow-hidden">
        {/* Header */}
        <div className="flex items-center justify-between px-6 py-4 border-b border-[#1f2937]">
          <div>
            <h2 className="text-base font-semibold text-slate-100">Import Published Contract</h2>
            <p className="text-xs text-slate-500 mt-0.5">Import a contract by publication ref</p>
          </div>
          <button onClick={onClose} className="text-slate-500 hover:text-slate-300 text-xl leading-none">×</button>
        </div>

        <div className="p-6 space-y-4">
          <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
            <div>
              <label className="text-xs font-medium text-slate-400 uppercase tracking-wider mb-1.5 block">
                Publication Ref
              </label>
              <input
                type="text"
                value={ref}
                onChange={(e) => { setRef(e.target.value); setPreview(null); setErr(null); }}
                placeholder="e.g. a3f8c2d1b094…"
                className="w-full bg-[#0a0d12] border border-[#1f2937] rounded-lg px-3 py-2 text-sm text-slate-200 font-mono placeholder-slate-600 outline-none focus:border-teal-600 transition-colors"
              />
            </div>
            <div>
              <label className="text-xs font-medium text-slate-400 uppercase tracking-wider mb-1.5 block">
                Link Token <span className="normal-case text-slate-600">(link-only publications)</span>
              </label>
              <input
                type="text"
                value={token}
                onChange={(e) => { setToken(e.target.value); setPreview(null); }}
                placeholder="Leave blank for public refs"
                className="w-full bg-[#0a0d12] border border-[#1f2937] rounded-lg px-3 py-2 text-sm text-slate-200 font-mono placeholder-slate-600 outline-none focus:border-teal-600 transition-colors"
              />
            </div>
          </div>

          <button
            onClick={handlePreview}
            disabled={fetching || !ref.trim()}
            className="px-4 py-2 bg-slate-700 hover:bg-slate-600 disabled:opacity-40 text-slate-200 text-sm font-medium rounded-lg transition-colors"
          >
            {fetching ? "Fetching…" : "Preview"}
          </button>

          {/* Preview panel */}
          {preview && (
            <div className="bg-teal-950/20 border border-teal-800/30 rounded-lg p-4 space-y-3">
              <div className="flex items-center justify-between flex-wrap gap-2">
                <div>
                  <p className="text-sm font-semibold text-teal-300">{preview.contract_name}</p>
                  <p className="text-xs text-slate-500">
                    v{preview.contract_version} · {preview.visibility} · published{" "}
                    {new Date(preview.published_at).toLocaleDateString()}
                  </p>
                </div>
                <span className="text-xs text-teal-500 font-mono">{preview.publication_ref}</span>
              </div>
              <details>
                <summary className="text-xs text-slate-500 cursor-pointer hover:text-slate-300 select-none">
                  View YAML ▾
                </summary>
                <pre className="mt-2 text-[10px] text-green-300 font-mono bg-[#0a0d12] rounded p-3 max-h-48 overflow-auto whitespace-pre-wrap leading-relaxed">
                  {preview.yaml_content}
                </pre>
              </details>

              {/* Import mode */}
              <div>
                <p className="text-xs font-medium text-slate-400 uppercase tracking-wider mb-2">
                  Import mode
                </p>
                <div className="space-y-2">
                  {(["snapshot", "subscribe"] as ImportMode[]).map((m) => (
                    <label key={m} className="flex items-start gap-3 cursor-pointer">
                      <input
                        type="radio"
                        name="import-mode"
                        value={m}
                        checked={mode === m}
                        onChange={() => setMode(m)}
                        className="mt-0.5 accent-teal-500"
                      />
                      <div>
                        <span className="text-sm font-medium text-slate-200 capitalize">{m}</span>
                        <p className="text-xs text-slate-500">
                          {m === "snapshot"
                            ? "One-time copy. Provenance is recorded but the contract never auto-updates."
                            : "Live link — when the provider publishes a newer version you'll see an update-available badge."}
                        </p>
                      </div>
                    </label>
                  ))}
                </div>
              </div>
            </div>
          )}

          {err && (
            <p className="text-sm text-red-400 bg-red-900/20 border border-red-800/40 rounded p-2">
              {err}
            </p>
          )}
          {success && (
            <p className="text-sm text-green-400 bg-green-900/20 border border-green-800/40 rounded p-2">
              ✓ {success}
            </p>
          )}
        </div>

        <div className="flex justify-end gap-3 px-6 py-4 border-t border-[#1f2937]">
          <button
            onClick={onClose}
            className="px-4 py-2 bg-[#1f2937] hover:bg-[#374151] text-slate-300 text-sm font-medium rounded-lg transition-colors"
          >
            Cancel
          </button>
          <button
            onClick={handleImport}
            disabled={importing || !preview || !!success}
            className="px-4 py-2 bg-teal-700 hover:bg-teal-600 disabled:opacity-40 text-white text-sm font-medium rounded-lg transition-colors"
          >
            {importing ? "Importing…" : "Import Contract"}
          </button>
        </div>
      </div>
    </div>
  );
}

// ManualCreatePanel
// ---------------------------------------------------------------------------

function ManualCreatePanel({ onCancel, onCreated }: { onCancel: () => void; onCreated: () => void }) {
  const [yaml, setYaml] = useState(EXAMPLE_YAML);
  const [creating, setCreating] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const handleCreate = async () => {
    setCreating(true); setError(null);
    try { await createContract(yaml); await mutate("contracts"); onCreated(); }
    catch (e: unknown) { setError(e instanceof Error ? e.message : String(e)); }
    finally { setCreating(false); }
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
        <p className="mt-2 text-sm text-red-400 bg-red-900/20 border border-red-800/40 rounded p-2">{error}</p>
      )}
      <div className="flex gap-3 mt-4">
        <button onClick={handleCreate} disabled={creating}
          className="px-4 py-2 bg-green-600 hover:bg-green-500 disabled:opacity-50 text-white text-sm font-medium rounded-lg transition-colors">
          {creating ? "Creating…" : "Create Contract"}
        </button>
        <button onClick={onCancel}
          className="px-4 py-2 bg-[#1f2937] hover:bg-[#374151] text-slate-300 text-sm font-medium rounded-lg transition-colors">
          Cancel
        </button>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Page
// ---------------------------------------------------------------------------

type Tab = "list" | "consumed" | "build" | "generate" | "csv" | "quarantine";

function ContractsContent() {
  const router = useRouter();
  const { org } = useOrg();
  const { data: contracts, isLoading } = useSWR<ContractSummary[]>(
    org ? "contracts" : null,
    listContracts
  );
  const [tab, setTab] = useState<Tab>("list");
  const [showCreate, setShowCreate] = useState(false);
  const [showImport, setShowImport] = useState(false);
  const [showImportRef, setShowImportRef] = useState(false);
  const [editingId, setEditingId] = useState<string | null>(null);

  // RFC-028: search + source filter
  const [query, setQuery] = useState("");
  const [sourceFilter, setSourceFilter] = useState<string>("all");
  const deployMeta = useDeployMeta();

  // Unique sources from deploy metadata, sorted
  const availableSources = useMemo(() => {
    const s = new Set<string>();
    deployMeta.forEach((dm) => { if (dm.source) s.add(dm.source); });
    return Array.from(s).sort();
  }, [deployMeta]);

  // Filtered + searched contract list
  const filteredContracts = useMemo(() => {
    if (!contracts) return contracts;
    let list = contracts;
    if (query.trim()) {
      const q = query.trim().toLowerCase();
      list = list.filter((c) => c.name.toLowerCase().includes(q));
    }
    if (sourceFilter !== "all") {
      list = list.filter((c) => deployMeta.get(c.name)?.source === sourceFilter);
    }
    return list;
  }, [contracts, query, sourceFilter, deployMeta]);

  const handleDelete = async (id: string) => {
    if (!confirm("Delete this contract? This cannot be undone.")) return;
    await deleteContract(id);
    await mutate("contracts");
  };

  const handleTestInPlayground = (yaml: string, contractId: string) => {
    sessionStorage.setItem("playground_yaml", yaml);
    sessionStorage.setItem("playground_contract_id", contractId);
    router.push("/playground");
  };

  return (
    <div>
      {editingId && (
        <EditContractModal
          contractId={editingId}
          onClose={() => setEditingId(null)}
          onSaved={() => setEditingId(null)}
          onTestInPlayground={handleTestInPlayground}
        />
      )}

      <div className="flex items-center justify-between mb-6">
        <div>
          <h1 className="text-2xl font-bold">Contracts</h1>
          <p className="text-sm text-slate-500 mt-1">
            Create and manage versioned semantic contracts
          </p>
        </div>
        {(tab === "list" || tab === "consumed") && (
          <div className="flex gap-2 flex-wrap">
            {/* RFC-032: import from publication ref (consumer flow) */}
            <button
              onClick={() => { setShowImportRef(true); setShowImport(false); setShowCreate(false); }}
              className="px-4 py-2 bg-teal-800 hover:bg-teal-700 text-white text-sm font-medium rounded-lg transition-colors"
            >
              ↓ Import from Ref
            </button>
            {tab === "list" && (
              <>
                <button
                  onClick={() => { setShowImport(true); setShowCreate(false); setShowImportRef(false); }}
                  className="px-4 py-2 bg-blue-700 hover:bg-blue-600 text-white text-sm font-medium rounded-lg transition-colors"
                >
                  ⬆ Import ODCS
                </button>
                <button
                  onClick={() => { setShowCreate((v) => !v); setShowImport(false); setShowImportRef(false); }}
                  className="px-4 py-2 bg-green-600 hover:bg-green-500 text-white text-sm font-medium rounded-lg transition-colors"
                >
                  + New Contract
                </button>
              </>
            )}
          </div>
        )}
      </div>

      <div className="flex gap-1 mb-6 bg-[#111827] border border-[#1f2937] rounded-xl p-1 w-fit flex-wrap">
        {(["list", "consumed", "build", "generate", "csv", "quarantine"] as Tab[]).map((t) => (
          <button
            key={t}
            onClick={() => { setTab(t); setShowCreate(false); setShowImport(false); setShowImportRef(false); }}
            className={clsx(
              "px-4 py-2 text-sm font-medium rounded-lg transition-colors",
              tab === t ? "bg-[#1f2937] text-slate-100" : "text-slate-500 hover:text-slate-300"
            )}
          >
            {t === "list" && "My Contracts"}
            {t === "consumed" && "📥 Consumed"}
            {t === "build" && "🧱 Visual Builder"}
            {t === "generate" && "✦ Generate from Sample"}
            {t === "csv" && "📊 From CSV"}
            {t === "quarantine" && "🔒 Quarantine"}
          </button>
        ))}
      </div>

      {/* RFC-032: Consumed contracts tab */}
      {tab === "consumed" && (
        <>
          <div className="mb-4">
            <p className="text-sm text-slate-400">
              Contracts you imported from a provider publication ref. Subscribe-mode contracts show
              update-available badges when the source publishes a newer version.
            </p>
          </div>
          <ConsumedContractsList
            contracts={contracts}
            isLoading={isLoading}
            onEdit={(id) => setEditingId(id)}
          />
        </>
      )}

      {tab === "list" && (
        <>
          {/* RFC-028: search + source filter */}
          <div className="flex items-center gap-3 mb-4 flex-wrap">
            <div className="relative flex-1 min-w-[200px] max-w-sm">
              <span className="absolute left-3 top-1/2 -translate-y-1/2 text-slate-500 text-sm pointer-events-none">
                🔍
              </span>
              <input
                type="text"
                placeholder="Search contracts…"
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                className="w-full pl-8 pr-4 py-2 bg-[#111827] border border-[#1f2937] rounded-lg text-sm text-slate-200 placeholder-slate-600 outline-none focus:border-indigo-600 transition-colors"
              />
            </div>
            {availableSources.length > 0 && (
              <select
                value={sourceFilter}
                onChange={(e) => setSourceFilter(e.target.value)}
                className="px-3 py-2 bg-[#111827] border border-[#1f2937] rounded-lg text-sm text-slate-300 outline-none focus:border-indigo-600 transition-colors"
              >
                <option value="all">All sources</option>
                {availableSources.map((s) => (
                  <option key={s} value={s}>{s}</option>
                ))}
              </select>
            )}
            {(query || sourceFilter !== "all") && (
              <button
                onClick={() => { setQuery(""); setSourceFilter("all"); }}
                className="text-xs text-slate-500 hover:text-slate-300 transition-colors"
              >
                ✕ Clear
              </button>
            )}
            {!isLoading && contracts && filteredContracts && filteredContracts.length !== contracts.length && (
              <span className="text-xs text-slate-500 ml-auto">
                {filteredContracts.length} of {contracts.length}
              </span>
            )}
          </div>

          {showCreate && (
            <ManualCreatePanel onCancel={() => setShowCreate(false)} onCreated={() => setShowCreate(false)} />
          )}
          <ContractList
            contracts={filteredContracts}
            isLoading={isLoading}
            deployMeta={deployMeta}
            onDelete={handleDelete}
            onEdit={(id) => setEditingId(id)}
          />
        </>
      )}
      {tab === "build" && <VisualBuilder onSaved={() => setTab("list")} />}
      {tab === "generate" && (
        <div className="bg-[#111827] border border-[#1f2937] rounded-xl p-6">
          <GeneratorTab onSaved={() => setTab("list")} />
        </div>
      )}
      {tab === "csv" && (
        <div className="bg-[#111827] border border-[#1f2937] rounded-xl p-6">
          <CsvGeneratorTab onSaved={() => setTab("list")} />
        </div>
      )}
      {tab === "quarantine" && (
        <div className="bg-[#111827] border border-[#1f2937] rounded-xl p-6">
          <div className="mb-5">
            <h2 className="text-base font-semibold text-slate-100">
              Quarantine
            </h2>
            <p className="text-sm text-slate-500 mt-1">
              Events that failed validation and were held for review.
              Select one or more to replay against any contract version.
            </p>
          </div>
          <QuarantineTab contracts={contracts} />
        </div>
      )}

      {/* ODCS import modal — fixed overlay, rendered outside tab flow */}
      {showImport && (
        <OdcsImportModal
          onClose={() => setShowImport(false)}
          onImported={() => { setShowImport(false); mutate("contracts"); }}
        />
      )}

      {/* RFC-032: Import from publication ref modal */}
      {showImportRef && (
        <ImportFromRefModal
          onClose={() => setShowImportRef(false)}
          onImported={() => { setShowImportRef(false); mutate("contracts"); setTab("consumed"); }}
        />
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
