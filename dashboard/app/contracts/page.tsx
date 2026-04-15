"use client";

import { useState, useEffect } from "react";
import { useRouter } from "next/navigation";
import useSWR, { mutate } from "swr";
import {
  listContracts,
  getContract,
  createContract,
  updateContract,
  deleteContract,
} from "@/lib/api";
import type { ContractSummary, ContractResponse } from "@/lib/api";
import clsx from "clsx";

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const EXAMPLE_YAML = `version: "1.0"
name: "my_events"
description: "Replace this with your contract"

ontology:
  entities:
    - name: user_id
      type: string
      required: true
      pattern: "^[a-zA-Z0-9_-]{3,64}$"

    - name: event_type
      type: string
      required: true
      enum:
        - "click"
        - "view"
        - "purchase"

    - name: timestamp
      type: integer
      required: true
      min: 0

    - name: amount
      type: number
      required: false
      min: 0

glossary:
  - field: "user_id"
    description: "Unique user identifier"
  - field: "amount"
    description: "Monetary value in USD"
    constraints: "must be non-negative"

metrics:
  - name: "total_revenue"
    formula: "sum(amount) where event_type = 'purchase'"
`;

const EXAMPLE_SAMPLE = `[
  { "user_id": "alice_01", "event_type": "click", "timestamp": 1712000001, "page": "/home" },
  { "user_id": "bob_99",   "event_type": "purchase", "timestamp": 1712000002, "amount": 49.99, "page": "/checkout" },
  { "user_id": "carol_x",  "event_type": "login",  "timestamp": 1712000003 },
  { "user_id": "dave_7",   "event_type": "view",   "timestamp": 1712000004, "amount": 0, "page": "/product" },
  { "user_id": "eve_22",   "event_type": "click",  "timestamp": 1712000005, "page": "/about" }
]`;

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
  const [yaml, setYaml] = useState<string>("");
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [contract, setContract] = useState<ContractResponse | null>(null);
  const [copied, setCopied] = useState(false);

  const BASE = process.env.NEXT_PUBLIC_API_URL ?? "http://localhost:3001";
  const ingestUrl = `${BASE}/ingest/${contractId}`;

  useEffect(() => {
    setLoading(true);
    getContract(contractId)
      .then((c) => {
        setContract(c);
        setYaml(c.yaml_content);
      })
      .catch((e) => setError(e instanceof Error ? e.message : String(e)))
      .finally(() => setLoading(false));
  }, [contractId]);

  const handleSave = async () => {
    setSaving(true);
    setError(null);
    try {
      await updateContract(contractId, { yaml_content: yaml });
      await mutate("contracts");
      onSaved();
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

  // Close on Escape key
  useEffect(() => {
    const handler = (e: KeyboardEvent) => { if (e.key === "Escape") onClose(); };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [onClose]);

  return (
    /* Backdrop */
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/70 backdrop-blur-sm p-4"
      onClick={(e) => { if (e.target === e.currentTarget) onClose(); }}
    >
      <div className="bg-[#0f1623] border border-[#1f2937] rounded-2xl w-full max-w-3xl shadow-2xl flex flex-col max-h-[90vh]">
        {/* Header */}
        <div className="flex items-start justify-between p-6 border-b border-[#1f2937]">
          <div>
            <h2 className="text-lg font-semibold text-slate-100">
              {loading ? "Loading…" : contract?.name ?? "Contract"}
            </h2>
            {contract && (
              <p className="text-xs text-slate-500 font-mono mt-1">{contract.id}</p>
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
              onClick={() => onTestInPlayground(yaml, contractId)}
              className="shrink-0 px-3 py-1 text-xs bg-indigo-600 hover:bg-indigo-500 text-white rounded-lg transition-colors"
            >
              Test in Playground →
            </button>
          </div>
        )}

        {/* YAML editor */}
        <div className="flex-1 overflow-auto p-6">
          {loading ? (
            <div className="flex items-center justify-center h-48 text-slate-500 text-sm">
              Loading contract…
            </div>
          ) : (
            <>
              <div className="flex items-center justify-between mb-2">
                <label className="text-xs font-medium text-slate-400 uppercase tracking-wider">
                  Contract YAML
                </label>
                {contract && (
                  <span className="text-xs text-slate-600">
                    Updated {new Date(contract.updated_at).toLocaleString()}
                  </span>
                )}
              </div>
              <textarea
                className="w-full h-80 bg-[#0a0d12] text-green-300 font-mono text-sm p-4 rounded-lg border border-[#1f2937] outline-none focus:border-green-700 resize-y"
                value={yaml}
                onChange={(e) => { setYaml(e.target.value); setError(null); }}
                spellCheck={false}
              />
              {error && (
                <p className="mt-2 text-sm text-red-400 bg-red-900/20 border border-red-800/40 rounded p-2">
                  {error}
                </p>
              )}
            </>
          )}
        </div>

        {/* Footer actions */}
        {!loading && (
          <div className="flex items-center gap-3 p-6 border-t border-[#1f2937]">
            <button
              onClick={handleSave}
              disabled={saving}
              className="px-4 py-2 bg-green-600 hover:bg-green-500 disabled:opacity-50 text-white text-sm font-medium rounded-lg transition-colors"
            >
              {saving ? "Saving…" : "Save Changes"}
            </button>
            <button
              onClick={onClose}
              className="px-4 py-2 bg-[#1f2937] hover:bg-[#374151] text-slate-300 text-sm font-medium rounded-lg transition-colors"
            >
              Cancel
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
  onToggleActive,
  onDelete,
  onEdit,
}: {
  contracts?: ContractSummary[];
  isLoading: boolean;
  onToggleActive: (c: ContractSummary) => void;
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
            <div className="flex items-center gap-3">
              <h3 className="font-semibold text-slate-200">{c.name}</h3>
              <span className="text-xs text-slate-500">v{c.version}</span>
              <span
                className={clsx(
                  "text-xs px-2 py-0.5 rounded-full font-medium",
                  c.active
                    ? "bg-green-900/40 text-green-400"
                    : "bg-slate-800 text-slate-500"
                )}
              >
                {c.active ? "Active" : "Inactive"}
              </span>
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
              onClick={() => onToggleActive(c)}
              className="px-3 py-1.5 text-xs bg-[#1f2937] hover:bg-[#374151] text-slate-300 rounded-lg transition-colors"
            >
              {c.active ? "Deactivate" : "Activate"}
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

type Tab = "list" | "generate";

export default function ContractsPage() {
  const router = useRouter();
  const { data: contracts, isLoading } = useSWR<ContractSummary[]>(
    "contracts",
    listContracts
  );
  const [tab, setTab] = useState<Tab>("list");
  const [showCreate, setShowCreate] = useState(false);
  const [editingId, setEditingId] = useState<string | null>(null);

  const handleToggleActive = async (c: ContractSummary) => {
    await updateContract(c.id, { active: !c.active });
    await mutate("contracts");
  };

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
            onToggleActive={handleToggleActive}
            onDelete={handleDelete}
            onEdit={(id) => setEditingId(id)}
          />
        </>
      )}

      {tab === "generate" && (
        <div className="bg-[#111827] border border-[#1f2937] rounded-xl p-6">
          <GeneratorTab onSaved={() => setTab("list")} />
        </div>
      )}
    </div>
  );
}
