"use client";

/**
 * Catalog page — RFC-029 + RFC-032.
 *
 * Two panels:
 *   1. Public Contract Catalog — browse and import published contracts by ref
 *   2. Egress Validator — test an outbound payload against a contract
 *
 * This is the primary consumer-facing landing page: "what contracts can I
 * consume, and how do I validate what I'm sending out?"
 */

import { useState, useEffect, Suspense } from "react";
import { useSearchParams } from "next/navigation";
import AuthGate from "@/components/AuthGate";
import {
  fetchPublished,
  importPublished,
  egressValidate,
  listContracts,
  listPublicCatalog,
} from "@/lib/api";
import type {
  FetchedPublication,
  ImportMode,
  EgressResponse,
  EgressDisposition,
  ContractSummary,
  CatalogEntry,
} from "@/lib/api";
import useSWR, { mutate } from "swr";
import { useOrg } from "@/lib/org";
import clsx from "clsx";

// ---------------------------------------------------------------------------
// Import-from-Ref panel (inline version of ImportFromRefModal for this page)
// ---------------------------------------------------------------------------

function ImportPanel() {
  const searchParams = useSearchParams();
  const [ref, setRef] = useState(searchParams.get("ref") ?? "");
  const [token, setToken] = useState("");
  const [mode, setMode] = useState<ImportMode>("snapshot");
  const [preview, setPreview] = useState<FetchedPublication | null>(null);
  const [fetching, setFetching] = useState(false);
  const [importing, setImporting] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const [success, setSuccess] = useState<string | null>(null);

  // Fetch public catalog for discovery — no auth required.
  const { data: publicEntries } = useSWR<CatalogEntry[]>(
    "public-catalog",
    () => listPublicCatalog(20)
  );

  // If the page was opened with ?ref=, auto-preview it.
  useEffect(() => {
    const initial = searchParams.get("ref");
    if (initial) {
      setRef(initial);
      fetchPublished(initial)
        .then(setPreview)
        .catch(() => {/* silently ignore — user can retry */});
    }
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const handlePreview = async () => {
    if (!ref.trim()) { setErr("Publication ref is required."); return; }
    setFetching(true); setErr(null); setPreview(null); setSuccess(null);
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
      setSuccess(`Imported as ${mode}. Contract is now in your list.`);
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    } finally { setImporting(false); }
  };

  const handleSelectCatalogEntry = (entry: CatalogEntry) => {
    setRef(entry.publication_ref);
    setToken("");
    setPreview(null);
    setErr(null);
    setSuccess(null);
    // Auto-preview on selection.
    setFetching(true);
    fetchPublished(entry.publication_ref)
      .then((p) => setPreview(p))
      .catch((e) => setErr(e instanceof Error ? e.message : String(e)))
      .finally(() => setFetching(false));
  };

  const showBrowse = ref.trim() === "" && !preview;

  return (
    <div className="space-y-5">
      {/* Browsable public list — collapses when user starts typing a ref */}
      {showBrowse && publicEntries && publicEntries.length > 0 && (
        <div>
          <p className="text-xs font-medium text-slate-400 uppercase tracking-wider mb-2">
            Available Public Contracts
          </p>
          <div className="divide-y divide-[#1f2937] border border-[#1f2937] rounded-xl overflow-hidden">
            {publicEntries.map((e) => (
              <button
                key={e.publication_ref}
                onClick={() => handleSelectCatalogEntry(e)}
                className="w-full flex items-center justify-between gap-3 px-4 py-3 hover:bg-[#1f2937]/50 transition-colors text-left group"
              >
                <div className="min-w-0">
                  <p className="text-sm font-medium text-slate-300 group-hover:text-teal-400 transition-colors truncate">
                    {e.contract_name}
                  </p>
                  <p className="text-xs text-slate-600 mt-0.5 font-mono">
                    v{e.contract_version}
                    <span className="ml-2 text-slate-700 font-sans">
                      · {new Date(e.published_at).toLocaleDateString()}
                    </span>
                    {e.published_by && (
                      <span className="ml-2 text-slate-700 font-sans">· {e.published_by}</span>
                    )}
                  </p>
                </div>
                <span className="text-xs text-slate-600 group-hover:text-teal-400 transition-colors shrink-0">
                  Preview →
                </span>
              </button>
            ))}
          </div>
          <p className="text-xs text-slate-700 mt-2">
            Or paste a ref below to import a private (link-gated) contract.
          </p>
        </div>
      )}

      {/* Ref + token inputs */}
      <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
        <div>
          <label className="text-xs font-medium text-slate-400 uppercase tracking-wider mb-1.5 block">
            Publication Ref
          </label>
          <input
            type="text"
            value={ref}
            onChange={(e) => { setRef(e.target.value); setPreview(null); setErr(null); setSuccess(null); }}
            placeholder="e.g. a3f8c2d1b094…"
            className="w-full bg-[#0a0d12] border border-[#1f2937] rounded-lg px-3 py-2 text-sm text-slate-200 font-mono placeholder-slate-600 outline-none focus:border-teal-600 transition-colors"
          />
        </div>
        <div>
          <label className="text-xs font-medium text-slate-400 uppercase tracking-wider mb-1.5 block">
            Link Token <span className="normal-case font-normal text-slate-600">(link-only)</span>
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
        {fetching ? "Fetching…" : "Preview contract"}
      </button>

      {preview && (
        <div className="bg-teal-950/20 border border-teal-800/30 rounded-xl p-5 space-y-4">
          <div className="flex items-center justify-between flex-wrap gap-2">
            <div>
              <p className="text-base font-semibold text-teal-300">{preview.contract_name}</p>
              <p className="text-xs text-slate-500">
                v{preview.contract_version} · {preview.visibility} visibility ·{" "}
                published {new Date(preview.published_at).toLocaleDateString()}
              </p>
            </div>
            <div className="flex items-center gap-2">
              <button
                onClick={() => { setPreview(null); setRef(""); }}
                className="text-xs text-slate-600 hover:text-slate-400 transition-colors"
              >
                ← Back to list
              </button>
              <code className="text-xs text-slate-500 font-mono">{preview.publication_ref}</code>
            </div>
          </div>

          <details>
            <summary className="text-xs text-slate-500 cursor-pointer hover:text-slate-300 select-none">
              View YAML ▾
            </summary>
            <pre className="mt-2 text-[10px] text-green-300 font-mono bg-[#0a0d12] rounded-lg p-3 max-h-56 overflow-auto whitespace-pre-wrap leading-relaxed">
              {preview.yaml_content}
            </pre>
          </details>

          <div>
            <p className="text-xs font-medium text-slate-400 uppercase tracking-wider mb-2">
              Import mode
            </p>
            <div className="space-y-2">
              {(["snapshot", "subscribe"] as ImportMode[]).map((m) => (
                <label key={m} className="flex items-start gap-3 cursor-pointer">
                  <input
                    type="radio"
                    name="cat-import-mode"
                    value={m}
                    checked={mode === m}
                    onChange={() => setMode(m)}
                    className="mt-0.5 accent-teal-500"
                  />
                  <div>
                    <span className="text-sm font-medium text-slate-200 capitalize">{m}</span>
                    <p className="text-xs text-slate-500 leading-relaxed">
                      {m === "snapshot"
                        ? "One-time copy with provenance. Contract never auto-updates."
                        : "Live link — shows an update-available badge when the provider publishes a new version."}
                    </p>
                  </div>
                </label>
              ))}
            </div>
          </div>

          {!success && (
            <button
              onClick={handleImport}
              disabled={importing}
              className="px-4 py-2 bg-teal-700 hover:bg-teal-600 disabled:opacity-40 text-white text-sm font-medium rounded-lg transition-colors"
            >
              {importing ? "Importing…" : "Import contract"}
            </button>
          )}
        </div>
      )}

      {err && (
        <p className="text-sm text-red-400 bg-red-900/20 border border-red-800/40 rounded p-3">{err}</p>
      )}
      {success && (
        <p className="text-sm text-green-400 bg-green-900/20 border border-green-800/40 rounded p-3">
          ✓ {success}
        </p>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Egress Validator — RFC-029
// ---------------------------------------------------------------------------

function EgressValidator({ contracts }: { contracts: ContractSummary[] }) {
  const [contractId, setContractId] = useState(contracts[0]?.id ?? "");
  const [disposition, setDisposition] = useState<EgressDisposition>("block");
  const [payloadText, setPayloadText] = useState(
    JSON.stringify(
      [
        { user_id: "alice", event_type: "purchase", timestamp: 1716000000, amount: 42.5 },
        { user_id: "bob", event_type: "click", timestamp: 1716000001 },
      ],
      null,
      2
    )
  );
  const [dryRun, setDryRun] = useState(true);
  const [result, setResult] = useState<EgressResponse | null>(null);
  const [running, setRunning] = useState(false);
  const [parseErr, setParseErr] = useState<string | null>(null);
  const [apiErr, setApiErr] = useState<string | null>(null);

  const handleValidate = async () => {
    setParseErr(null); setApiErr(null); setResult(null);
    let payload: unknown;
    try {
      payload = JSON.parse(payloadText);
    } catch (e) {
      setParseErr(`Invalid JSON: ${e instanceof Error ? e.message : String(e)}`);
      return;
    }
    if (!Array.isArray(payload)) payload = [payload];
    setRunning(true);
    try {
      const res = await egressValidate(contractId, payload, { disposition, dryRun });
      setResult(res);
    } catch (e) {
      setApiErr(e instanceof Error ? e.message : String(e));
    } finally { setRunning(false); }
  };

  const actionColor = (action: string) => {
    if (action === "included") return "text-green-400 bg-green-900/30";
    if (action === "blocked") return "text-red-400 bg-red-900/30";
    if (action === "rejected") return "text-red-400 bg-red-900/30";
    if (action === "tagged") return "text-amber-400 bg-amber-900/30";
    return "text-slate-400";
  };

  return (
    <div className="space-y-4">
      {/* Controls */}
      <div className="grid grid-cols-1 sm:grid-cols-3 gap-3">
        <div>
          <label className="text-xs font-medium text-slate-400 uppercase tracking-wider mb-1.5 block">
            Contract
          </label>
          <select
            value={contractId}
            onChange={(e) => setContractId(e.target.value)}
            className="w-full bg-[#0a0d12] border border-[#1f2937] rounded-lg px-3 py-2 text-sm text-slate-200 outline-none"
          >
            {contracts.map((c) => (
              <option key={c.id} value={c.id}>{c.name}</option>
            ))}
          </select>
        </div>
        <div>
          <label className="text-xs font-medium text-slate-400 uppercase tracking-wider mb-1.5 block">
            Disposition
          </label>
          <select
            value={disposition}
            onChange={(e) => setDisposition(e.target.value as EgressDisposition)}
            className="w-full bg-[#0a0d12] border border-[#1f2937] rounded-lg px-3 py-2 text-sm text-slate-200 outline-none"
          >
            <option value="block">block — drop failing records</option>
            <option value="fail">fail — reject entire response</option>
            <option value="tag">tag — pass through with flags</option>
          </select>
        </div>
        <div className="flex flex-col justify-end">
          <label className="flex items-center gap-2 cursor-pointer mb-1.5">
            <input
              type="checkbox"
              checked={dryRun}
              onChange={(e) => setDryRun(e.target.checked)}
              className="accent-indigo-500"
            />
            <span className="text-sm text-slate-300">Dry run</span>
          </label>
          <p className="text-xs text-slate-600">
            Dry run validates without writing to the audit log.
          </p>
        </div>
      </div>

      {/* Payload editor */}
      <div>
        <div className="flex items-center justify-between mb-1.5">
          <label className="text-xs font-medium text-slate-400 uppercase tracking-wider">
            Outbound Payload (JSON array or object)
          </label>
        </div>
        <textarea
          value={payloadText}
          onChange={(e) => { setPayloadText(e.target.value); setParseErr(null); }}
          rows={10}
          className="w-full bg-[#0a0d12] text-blue-300 font-mono text-sm p-4 rounded-lg border border-[#1f2937] outline-none focus:border-indigo-600 resize-y transition-colors"
          spellCheck={false}
        />
        {parseErr && (
          <p className="mt-1 text-xs text-red-400">{parseErr}</p>
        )}
      </div>

      <button
        onClick={handleValidate}
        disabled={running || !contractId}
        className="px-5 py-2 bg-indigo-700 hover:bg-indigo-600 disabled:opacity-40 text-white text-sm font-medium rounded-lg transition-colors"
      >
        {running ? "Validating…" : "▶ Validate Egress"}
      </button>

      {apiErr && (
        <p className="text-sm text-red-400 bg-red-900/20 border border-red-800/40 rounded p-3">{apiErr}</p>
      )}

      {/* Results */}
      {result && (
        <div className="space-y-4">
          {/* Summary bar */}
          <div className="grid grid-cols-2 sm:grid-cols-4 gap-3">
            {[
              { label: "Total", value: result.total, color: "text-white" },
              { label: "Passed", value: result.passed, color: "text-green-400" },
              { label: "Failed", value: result.failed, color: "text-red-400" },
              {
                label: "Dry run",
                value: result.dry_run ? "yes" : "no",
                color: result.dry_run ? "text-amber-400" : "text-slate-400",
              },
            ].map((s) => (
              <div key={s.label} className="bg-[#0d1117] border border-[#1f2937] rounded-lg px-3 py-3">
                <p className="text-xs text-slate-500 mb-1">{s.label}</p>
                <p className={clsx("text-xl font-bold", s.color)}>{s.value}</p>
              </div>
            ))}
          </div>

          {/* Per-record outcomes */}
          <div className="bg-[#0d1117] border border-[#1f2937] rounded-xl overflow-hidden">
            <div className="px-4 py-3 border-b border-[#1f2937]">
              <p className="text-xs font-medium text-slate-400 uppercase tracking-wider">
                Per-Record Outcomes · disposition: <span className="text-slate-200">{result.disposition}</span>
                {" · "}v{result.resolved_version}
              </p>
            </div>
            <div className="divide-y divide-[#1f2937]/50">
              {result.outcomes.map((o) => (
                <div key={o.index} className="px-4 py-3 flex items-start gap-3">
                  <span className="text-xs text-slate-600 font-mono w-6 shrink-0">
                    [{o.index}]
                  </span>
                  <span
                    className={clsx(
                      "text-[10px] uppercase tracking-wider border rounded px-2 py-0.5 shrink-0 font-medium border-transparent",
                      actionColor(o.action)
                    )}
                  >
                    {o.action}
                  </span>
                  <div className="flex-1 min-w-0">
                    {o.violations.length > 0 ? (
                      <ul className="space-y-0.5">
                        {o.violations.map((v, vi) => (
                          <li key={vi} className="text-xs text-slate-400">
                            <span className="text-red-400 font-mono">{v.field}</span>
                            {" · "}
                            <span className="text-slate-500">{v.message}</span>
                          </li>
                        ))}
                      </ul>
                    ) : (
                      <p className="text-xs text-slate-600">no violations</p>
                    )}
                  </div>
                  <span className="text-[10px] text-slate-600 font-mono shrink-0">
                    {o.validation_us}µs
                  </span>
                </div>
              ))}
            </div>
          </div>

          {/* Cleaned payload */}
          {result.payload.length > 0 && (
            <details>
              <summary className="text-xs text-slate-500 cursor-pointer hover:text-slate-300 select-none">
                Cleaned payload ({result.payload.length} record{result.payload.length !== 1 ? "s" : ""}) ▾
              </summary>
              <pre className="mt-2 text-[10px] text-green-300 font-mono bg-[#0a0d12] rounded-lg p-4 max-h-64 overflow-auto whitespace-pre-wrap leading-relaxed">
                {JSON.stringify(result.payload, null, 2)}
              </pre>
            </details>
          )}
        </div>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Page
// ---------------------------------------------------------------------------

type PageTab = "import" | "egress";

function CatalogContent() {
  const [tab, setTab] = useState<PageTab>("import");
  const { org } = useOrg();
  const { data: contracts, isLoading: contractsLoading } = useSWR<ContractSummary[]>(
    org ? "contracts" : null,
    listContracts
  );

  return (
    <div>
      {/* Header */}
      <div className="mb-6">
        <h1 className="text-2xl font-bold">Contract Catalog</h1>
        <p className="text-sm text-slate-500 mt-1">
          Import contracts from providers and validate your outbound data.
        </p>
      </div>

      {/* Tab bar */}
      <div className="flex gap-1 mb-6 bg-[#111827] border border-[#1f2937] rounded-xl p-1 w-fit">
        <button
          onClick={() => setTab("import")}
          className={clsx(
            "px-4 py-2 text-sm font-medium rounded-lg transition-colors",
            tab === "import" ? "bg-[#1f2937] text-slate-100" : "text-slate-500 hover:text-slate-300"
          )}
        >
          📥 Import Contract
        </button>
        <button
          onClick={() => setTab("egress")}
          className={clsx(
            "px-4 py-2 text-sm font-medium rounded-lg transition-colors",
            tab === "egress" ? "bg-[#1f2937] text-slate-100" : "text-slate-500 hover:text-slate-300"
          )}
        >
          ↗ Egress Validator
        </button>
      </div>

      {/* Tab bodies */}
      {tab === "import" && (
        <div className="bg-[#111827] border border-[#1f2937] rounded-xl p-6">
          <div className="mb-5">
            <h2 className="text-base font-semibold text-slate-100">Import from Publication Ref</h2>
            <p className="text-sm text-slate-500 mt-1">
              A provider shares a publication ref (and optional link token). Paste it here to preview and
              import their contract directly — no manual reconstruction needed.
            </p>
          </div>
          <ImportPanel />
        </div>
      )}

      {tab === "egress" && (
        <div className="bg-[#111827] border border-[#1f2937] rounded-xl p-6">
          <div className="mb-5">
            <h2 className="text-base font-semibold text-slate-100">Egress Validator</h2>
            <p className="text-sm text-slate-500 mt-1">
              Validate an outbound payload against one of your contracts before it leaves your API.
              The same engine that runs on ingest — identical rules, identical latency budget.
            </p>
          </div>
          {contractsLoading ? (
            <p className="text-sm text-slate-500">Loading contracts…</p>
          ) : !contracts || contracts.length === 0 ? (
            <div className="flex flex-col items-center justify-center h-48 text-slate-600">
              <p className="text-4xl mb-3">📋</p>
              <p className="text-sm">No contracts yet — create one first.</p>
            </div>
          ) : (
            <EgressValidator contracts={contracts} />
          )}
        </div>
      )}
    </div>
  );
}

export default function CatalogPage() {
  return (
    <AuthGate page="catalog">
      <Suspense fallback={null}>
        <CatalogContent />
      </Suspense>
    </AuthGate>
  );
}
