"use client";

/**
 * NewContractWizard — RFC-036
 *
 * Source-first new contract creation modal.
 * Three paths:
 *   catalog  → fork a curated open-data contract
 *   csv      → infer a contract from CSV content
 *   blank    → raw YAML editor (existing ManualCreatePanel behaviour)
 */

import { useState, useEffect, useRef } from "react";
import { mutate } from "swr";
import clsx from "clsx";
import {
  listOpenDataContracts,
  forkPublicContract,
  inferCsv,
  createContract,
  type OpenDataContract,
} from "@/lib/api";
import { EXAMPLE_YAML } from "./examples";

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

type WizardStep = "pick" | "catalog" | "csv" | "blank";

interface Props {
  onClose: () => void;
  onCreated: () => void;
}

// ---------------------------------------------------------------------------
// Source tile
// ---------------------------------------------------------------------------

function SourceTile({
  icon,
  title,
  description,
  onClick,
}: {
  icon: string;
  title: string;
  description: string;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      className="group flex flex-col items-start gap-3 p-5 bg-[#111827] hover:bg-[#1a2333] border border-[#1f2937] hover:border-indigo-700/60 rounded-xl text-left transition-all focus:outline-none focus:border-indigo-500"
    >
      <span className="text-3xl">{icon}</span>
      <div>
        <p className="text-sm font-semibold text-slate-100 group-hover:text-white">
          {title}
        </p>
        <p className="text-xs text-slate-500 mt-1 leading-relaxed">{description}</p>
      </div>
    </button>
  );
}

// ---------------------------------------------------------------------------
// Step: pick source
// ---------------------------------------------------------------------------

function PickStep({ onPick }: { onPick: (step: WizardStep) => void }) {
  return (
    <div className="space-y-5">
      <div>
        <h3 className="text-base font-semibold text-slate-100">How do you want to start?</h3>
        <p className="text-sm text-slate-500 mt-1">
          Choose a source for your contract. You can customise everything after creation.
        </p>
      </div>
      <div className="grid grid-cols-1 sm:grid-cols-3 gap-4">
        <SourceTile
          icon="🗂"
          title="Fork from Catalog"
          description="Pick a curated open-data source (census, transit, etc.) and fork it into your org."
          onClick={() => onPick("catalog")}
        />
        <SourceTile
          icon="📊"
          title="Infer from CSV"
          description="Upload or paste a CSV. Field types, enums, and patterns are inferred automatically."
          onClick={() => onPick("csv")}
        />
        <SourceTile
          icon="✏️"
          title="Start Blank"
          description="Open a YAML editor pre-filled with an example contract. Full control from day one."
          onClick={() => onPick("blank")}
        />
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Step: fork from catalog
// ---------------------------------------------------------------------------

function CatalogStep({ onCreated, onBack }: { onCreated: () => void; onBack: () => void }) {
  const [contracts, setContracts] = useState<OpenDataContract[] | null>(null);
  const [loadErr, setLoadErr] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState<OpenDataContract | null>(null);
  const [forkName, setForkName] = useState("");
  const [forking, setForking] = useState(false);
  const [forkErr, setForkErr] = useState<string | null>(null);

  useEffect(() => {
    listOpenDataContracts()
      .then(setContracts)
      .catch((e) => setLoadErr(e instanceof Error ? e.message : String(e)));
  }, []);

  const filtered = contracts?.filter((c) =>
    c.name.toLowerCase().includes(query.toLowerCase()) ||
    (c.description ?? "").toLowerCase().includes(query.toLowerCase())
  ) ?? [];

  const handleSelect = (c: OpenDataContract) => {
    setSelected(c);
    // Pre-fill fork name from the catalog contract name
    setForkName(c.name);
    setForkErr(null);
  };

  const handleFork = async () => {
    if (!selected || !forkName.trim()) return;
    setForking(true);
    setForkErr(null);
    try {
      await forkPublicContract(selected.id, { name: forkName.trim() });
      await mutate("contracts");
      onCreated();
    } catch (e: unknown) {
      setForkErr(e instanceof Error ? e.message : String(e));
    } finally {
      setForking(false);
    }
  };

  return (
    <div className="space-y-4">
      <div className="flex items-center gap-3">
        <button
          onClick={onBack}
          className="text-slate-500 hover:text-slate-300 text-sm transition-colors"
        >
          ← Back
        </button>
        <h3 className="text-base font-semibold text-slate-100">Fork from Catalog</h3>
      </div>

      {loadErr && (
        <p className="text-sm text-red-400 bg-red-900/20 border border-red-800/40 rounded p-2">
          {loadErr}
        </p>
      )}

      {!contracts && !loadErr && (
        <p className="text-sm text-slate-500">Loading catalog…</p>
      )}

      {contracts && contracts.length === 0 && (
        <div className="flex flex-col items-center justify-center h-40 text-slate-600">
          <p className="text-4xl mb-3">🗂</p>
          <p className="text-sm">No curated contracts in the catalog yet.</p>
        </div>
      )}

      {contracts && contracts.length > 0 && (
        <>
          {/* Search */}
          <div className="relative">
            <span className="absolute left-3 top-1/2 -translate-y-1/2 text-slate-500 text-sm pointer-events-none">
              🔍
            </span>
            <input
              type="text"
              placeholder="Filter contracts…"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              className="w-full pl-8 pr-4 py-2 bg-[#0a0d12] border border-[#1f2937] rounded-lg text-sm text-slate-200 placeholder-slate-600 outline-none focus:border-indigo-600 transition-colors"
            />
          </div>

          {/* Contract list */}
          <div className="space-y-2 max-h-56 overflow-y-auto pr-1">
            {filtered.length === 0 ? (
              <p className="text-sm text-slate-500 py-4 text-center">No matches.</p>
            ) : (
              filtered.map((c) => (
                <button
                  key={c.id}
                  onClick={() => handleSelect(c)}
                  className={clsx(
                    "w-full text-left p-4 rounded-lg border transition-all",
                    selected?.id === c.id
                      ? "bg-indigo-900/20 border-indigo-700/60 text-slate-100"
                      : "bg-[#111827] border-[#1f2937] hover:border-[#374151] text-slate-300"
                  )}
                >
                  <div className="flex items-center gap-2 flex-wrap">
                    <span className="text-sm font-medium">{c.name}</span>
                    <span className="text-[10px] px-1.5 py-0.5 rounded-full bg-slate-700 text-slate-400 uppercase tracking-wider font-mono">
                      {c.source_format} · v{c.version}
                    </span>
                  </div>
                  {c.description && (
                    <p className="text-xs text-slate-500 mt-1 leading-relaxed line-clamp-2">
                      {c.description}
                    </p>
                  )}
                </button>
              ))
            )}
          </div>

          {/* Fork name + action */}
          {selected && (
            <div className="pt-2 border-t border-[#1f2937] space-y-3">
              <div>
                <label className="text-xs font-medium text-slate-400 uppercase tracking-wider mb-1.5 block">
                  Fork Name
                </label>
                <input
                  type="text"
                  value={forkName}
                  onChange={(e) => setForkName(e.target.value)}
                  placeholder="my_contract_name"
                  className="w-full bg-[#0a0d12] border border-[#1f2937] rounded-lg px-3 py-2 text-sm text-slate-200 font-mono placeholder-slate-600 outline-none focus:border-indigo-600 transition-colors"
                />
              </div>
              {forkErr && (
                <p className="text-xs text-red-400 bg-red-900/20 border border-red-800/40 rounded p-2">
                  {forkErr}
                </p>
              )}
              <button
                onClick={handleFork}
                disabled={forking || !forkName.trim()}
                className="px-4 py-2 bg-indigo-600 hover:bg-indigo-500 disabled:opacity-40 disabled:cursor-not-allowed text-white text-sm font-medium rounded-lg transition-colors"
              >
                {forking ? "Forking…" : "Fork into my contracts →"}
              </button>
            </div>
          )}
        </>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Step: infer from CSV
// ---------------------------------------------------------------------------

type CsvInputMode = "paste" | "upload";

function CsvStep({ onCreated, onBack }: { onCreated: () => void; onBack: () => void }) {
  const [inputMode, setInputMode] = useState<CsvInputMode>("paste");
  const [csvText, setCsvText] = useState("");
  const [fileName, setFileName] = useState<string | null>(null);
  const [contractName, setContractName] = useState("my_contract");
  const [generatedYaml, setGeneratedYaml] = useState<string | null>(null);
  const [fieldCount, setFieldCount] = useState<number | null>(null);
  const [rowCount, setRowCount] = useState<number | null>(null);
  const [inferring, setInferring] = useState(false);
  const [inferErr, setInferErr] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [saveErr, setSaveErr] = useState<string | null>(null);

  const handleFileChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (!file) return;
    setFileName(file.name);
    const base = file.name.replace(/\.[^.]+$/, "").replace(/[^a-zA-Z0-9_]/g, "_").toLowerCase();
    if (base) setContractName(base);
    const reader = new FileReader();
    reader.onload = (ev) => {
      setCsvText(ev.target?.result as string ?? "");
      setGeneratedYaml(null);
      setInferErr(null);
    };
    reader.readAsText(file);
  };

  const handleInfer = async () => {
    const content = csvText.trim();
    if (!content) { setInferErr("Paste or upload a CSV first."); return; }
    setInferring(true); setInferErr(null); setGeneratedYaml(null);
    try {
      const res = await inferCsv({ name: contractName, csv_content: content });
      setGeneratedYaml(res.yaml_content);
      setFieldCount(res.field_count);
      setRowCount(res.sample_count);
    } catch (e: unknown) {
      setInferErr(e instanceof Error ? e.message : String(e));
    } finally { setInferring(false); }
  };

  const handleSave = async () => {
    if (!generatedYaml) return;
    setSaving(true); setSaveErr(null);
    try {
      await createContract(generatedYaml);
      await mutate("contracts");
      onCreated();
    } catch (e: unknown) {
      setSaveErr(e instanceof Error ? e.message : String(e));
    } finally { setSaving(false); }
  };

  return (
    <div className="space-y-4">
      <div className="flex items-center gap-3">
        <button
          onClick={onBack}
          className="text-slate-500 hover:text-slate-300 text-sm transition-colors"
        >
          ← Back
        </button>
        <h3 className="text-base font-semibold text-slate-100">Infer from CSV</h3>
      </div>

      {/* Input mode toggle */}
      <div className="flex gap-1 bg-[#0a0d12] border border-[#1f2937] rounded-lg p-1 w-fit">
        {(["paste", "upload"] as CsvInputMode[]).map((m) => (
          <button
            key={m}
            onClick={() => { setInputMode(m); setGeneratedYaml(null); setInferErr(null); }}
            className={clsx(
              "px-4 py-1.5 text-sm font-medium rounded-md transition-colors",
              inputMode === m ? "bg-[#1f2937] text-slate-100" : "text-slate-500 hover:text-slate-300"
            )}
          >
            {m === "paste" ? "📋 Paste" : "📁 Upload"}
          </button>
        ))}
      </div>

      <div className="grid grid-cols-1 lg:grid-cols-2 gap-4">
        {/* CSV input */}
        <div>
          {inputMode === "paste" ? (
            <textarea
              className="w-full h-52 bg-[#0a0d12] text-orange-300 font-mono text-sm p-4 rounded-lg border border-[#1f2937] outline-none focus:border-orange-700 resize-y"
              value={csvText}
              onChange={(e) => { setCsvText(e.target.value); setGeneratedYaml(null); setInferErr(null); }}
              spellCheck={false}
              placeholder={"id,name,amount,created_at\n1,Alice,99.99,2024-01-01"}
            />
          ) : (
            <div className="flex flex-col items-center justify-center h-52 border-2 border-dashed border-[#2d3748] rounded-lg bg-[#0a0d12] gap-3">
              <span className="text-3xl">📊</span>
              <p className="text-sm text-slate-400">
                {fileName ? <span className="text-orange-400 font-mono">{fileName}</span> : "Select a CSV file"}
              </p>
              {csvText && <p className="text-xs text-slate-500">{csvText.split("\n").length} lines loaded</p>}
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
          {inferErr && (
            <p className="mt-2 text-xs text-red-400 bg-red-900/20 border border-red-800/40 rounded p-2">{inferErr}</p>
          )}
        </div>

        {/* Generated YAML */}
        <div>
          <div className="flex items-center justify-between mb-1.5">
            <label className="text-xs font-medium text-slate-400 uppercase tracking-wider">
              Generated YAML
            </label>
            {generatedYaml && fieldCount !== null && (
              <span className="text-xs text-green-500">
                ✔ {fieldCount} field{fieldCount !== 1 ? "s" : ""} · {rowCount} row{rowCount !== 1 ? "s" : ""}
              </span>
            )}
          </div>
          <textarea
            className={clsx(
              "w-full h-52 font-mono text-sm p-4 rounded-lg border outline-none resize-y transition-colors",
              generatedYaml
                ? "bg-[#0a0d12] text-green-300 border-[#1f2937] focus:border-green-700"
                : "bg-[#0a0d12]/50 text-slate-600 border-[#1f2937]/50 cursor-not-allowed"
            )}
            value={generatedYaml ?? "// Infer a contract to see YAML here…"}
            onChange={(e) => setGeneratedYaml(e.target.value)}
            spellCheck={false}
            readOnly={!generatedYaml}
          />
          {saveErr && (
            <p className="mt-2 text-xs text-red-400 bg-red-900/20 border border-red-800/40 rounded p-2">{saveErr}</p>
          )}
        </div>
      </div>

      {/* Controls */}
      <div className="flex items-center gap-3 flex-wrap pt-1">
        <div className="flex items-center gap-2">
          <label className="text-xs text-slate-400 whitespace-nowrap">Contract name</label>
          <input
            type="text"
            value={contractName}
            onChange={(e) => setContractName(e.target.value)}
            className="bg-[#0a0d12] border border-[#1f2937] rounded-lg px-3 py-1.5 text-sm text-slate-200 outline-none focus:border-orange-700 w-44"
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
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Step: start blank
// ---------------------------------------------------------------------------

function BlankStep({ onCreated, onBack }: { onCreated: () => void; onBack: () => void }) {
  const [yaml, setYaml] = useState(EXAMPLE_YAML);
  const [creating, setCreating] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const handleCreate = async () => {
    setCreating(true); setError(null);
    try {
      await createContract(yaml);
      await mutate("contracts");
      onCreated();
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    } finally { setCreating(false); }
  };

  return (
    <div className="space-y-4">
      <div className="flex items-center gap-3">
        <button
          onClick={onBack}
          className="text-slate-500 hover:text-slate-300 text-sm transition-colors"
        >
          ← Back
        </button>
        <h3 className="text-base font-semibold text-slate-100">New Contract (YAML)</h3>
      </div>

      <textarea
        className="w-full h-80 bg-[#0a0d12] text-green-300 font-mono text-sm p-4 rounded-lg border border-[#1f2937] outline-none focus:border-green-700 resize-y"
        value={yaml}
        onChange={(e) => setYaml(e.target.value)}
        spellCheck={false}
      />

      {error && (
        <p className="text-sm text-red-400 bg-red-900/20 border border-red-800/40 rounded p-2">{error}</p>
      )}

      <div className="flex gap-3">
        <button
          onClick={handleCreate}
          disabled={creating}
          className="px-4 py-2 bg-green-600 hover:bg-green-500 disabled:opacity-50 text-white text-sm font-medium rounded-lg transition-colors"
        >
          {creating ? "Creating…" : "Create Contract"}
        </button>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Wizard shell
// ---------------------------------------------------------------------------

export function NewContractWizard({ onClose, onCreated }: Props) {
  const [step, setStep] = useState<WizardStep>("pick");
  const overlayRef = useRef<HTMLDivElement>(null);

  // Close on Escape
  useEffect(() => {
    const handler = (e: KeyboardEvent) => { if (e.key === "Escape") onClose(); };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [onClose]);

  const handleCreated = () => {
    onCreated();
  };

  return (
    <div
      ref={overlayRef}
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/70 backdrop-blur-sm p-4"
      onClick={(e) => { if (e.target === overlayRef.current) onClose(); }}
    >
      <div className="bg-[#0f1623] border border-[#1f2937] rounded-2xl w-full max-w-3xl shadow-2xl flex flex-col max-h-[90vh]">
        {/* Header */}
        <div className="flex items-center justify-between px-6 py-5 border-b border-[#1f2937]">
          <div>
            <h2 className="text-lg font-semibold text-slate-100">New Contract</h2>
            {step !== "pick" && (
              <p className="text-xs text-slate-500 mt-0.5">
                {step === "catalog" && "Fork a curated open-data source"}
                {step === "csv" && "Infer contract from CSV"}
                {step === "blank" && "Write YAML from scratch"}
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

        {/* Body */}
        <div className="flex-1 overflow-auto p-6">
          {step === "pick" && <PickStep onPick={setStep} />}
          {step === "catalog" && (
            <CatalogStep onCreated={handleCreated} onBack={() => setStep("pick")} />
          )}
          {step === "csv" && (
            <CsvStep onCreated={handleCreated} onBack={() => setStep("pick")} />
          )}
          {step === "blank" && (
            <BlankStep onCreated={handleCreated} onBack={() => setStep("pick")} />
          )}
        </div>

        {/* Footer hint */}
        <div className="px-6 py-3 border-t border-[#1f2937] flex items-center justify-end">
          <span className="text-xs text-slate-600">
            Press <kbd className="bg-[#1f2937] px-1 rounded">Esc</kbd> to close
          </span>
        </div>
      </div>
    </div>
  );
}
