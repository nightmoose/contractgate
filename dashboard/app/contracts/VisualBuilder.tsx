"use client";

import { useState, useCallback, useId } from "react";
import { createContract } from "@/lib/api";
import { mutate } from "swr";
import clsx from "clsx";

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

type FieldType = "string" | "integer" | "number" | "boolean";

interface FieldState {
  id: string;
  name: string;
  type: FieldType;
  required: boolean;
  // String-specific
  patternPreset: string;   // "" = none, "__custom__" = custom
  customPattern: string;
  enumInput: string;       // raw input buffer
  enumValues: string[];
  // Number-specific
  min: string;
  max: string;
  // String length-specific
  minLength: string;
  maxLength: string;
  // Glossary
  description: string;
  constraints: string;
}

interface MetricState {
  id: string;
  name: string;
  formula: string;
}

interface BuilderState {
  name: string;
  version: string;
  description: string;
  fields: FieldState[];
  metrics: MetricState[];
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const PATTERN_PRESETS: { label: string; value: string; hint: string }[] = [
  { label: "None",           value: "",            hint: "" },
  { label: "UUID",           value: "^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$", hint: "e.g. 550e8400-e29b-..." },
  { label: "Email",          value: "^[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\\.[a-zA-Z]{2,}$", hint: "e.g. user@example.com" },
  { label: "ISO Date",       value: "^\\d{4}-\\d{2}-\\d{2}", hint: "e.g. 2024-04-15" },
  { label: "URL",            value: "^https?:\\/\\/", hint: "http:// or https://" },
  { label: "Alphanumeric ID",value: "^[a-zA-Z0-9_-]{3,64}$", hint: "e.g. alice_01, user-99" },
  { label: "Custom…",        value: "__custom__",  hint: "" },
];

const TYPE_COLORS: Record<FieldType, string> = {
  string:  "bg-blue-900/40 text-blue-300 border-blue-700/50",
  integer: "bg-purple-900/40 text-purple-300 border-purple-700/50",
  number:  "bg-orange-900/40 text-orange-300 border-orange-700/50",
  boolean: "bg-teal-900/40 text-teal-300 border-teal-700/50",
};

// ---------------------------------------------------------------------------
// YAML builder
// ---------------------------------------------------------------------------

function resolvedPattern(f: FieldState): string {
  if (f.patternPreset === "__custom__") return f.customPattern.trim();
  return f.patternPreset;
}

function buildYaml(state: BuilderState): string {
  const safeName = (state.name.trim() || "my_contract").replace(/\s+/g, "_").toLowerCase();
  const lines: string[] = [
    `version: "${state.version.trim() || "1.0"}"`,
    `name: "${safeName}"`,
  ];
  if (state.description.trim()) {
    lines.push(`description: "${state.description.trim()}"`);
  }

  lines.push("", "ontology:", "  entities:");

  const validFields = state.fields.filter((f) => f.name.trim());
  if (validFields.length === 0) {
    lines.push("    [] # add fields above");
  } else {
    for (const f of validFields) {
      lines.push(`    - name: ${f.name.trim()}`);
      lines.push(`      type: ${f.type}`);
      lines.push(`      required: ${f.required}`);
      const pattern = resolvedPattern(f);
      if (f.type === "string" && pattern) {
        lines.push(`      pattern: "${pattern}"`);
      }
      if (f.type === "string" && f.enumValues.length > 0 && !pattern) {
        lines.push("      enum:");
        for (const v of f.enumValues) lines.push(`        - "${v}"`);
      }
      if ((f.type === "integer" || f.type === "number") && f.min !== "") {
        lines.push(`      min: ${f.min}`);
      }
      if ((f.type === "integer" || f.type === "number") && f.max !== "") {
        lines.push(`      max: ${f.max}`);
      }
      if (f.type === "string" && f.minLength !== "") {
        lines.push(`      min_length: ${f.minLength}`);
      }
      if (f.type === "string" && f.maxLength !== "") {
        lines.push(`      max_length: ${f.maxLength}`);
      }
    }
  }

  // Glossary — only fields that have a description
  const withDesc = validFields.filter((f) => f.description.trim());
  lines.push("", "glossary:");
  if (withDesc.length === 0) {
    lines[lines.length - 1] = "glossary: []";
  } else {
    for (const f of withDesc) {
      lines.push(`  - field: "${f.name.trim()}"`);
      lines.push(`    description: "${f.description.trim()}"`);
      if (f.constraints.trim()) {
        lines.push(`    constraints: "${f.constraints.trim()}"`);
      }
    }
  }

  // Metrics
  const validMetrics = state.metrics.filter((m) => m.name.trim() && m.formula.trim());
  lines.push("", "metrics:");
  if (validMetrics.length === 0) {
    lines[lines.length - 1] = "metrics: []";
  } else {
    for (const m of validMetrics) {
      lines.push(`  - name: "${m.name.trim()}"`);
      lines.push(`    formula: "${m.formula.trim()}"`);
    }
  }

  return lines.join("\n") + "\n";
}

// ---------------------------------------------------------------------------
// Default field factory
// ---------------------------------------------------------------------------

let _uid = 0;
function uid() { return `f_${++_uid}`; }

function defaultField(): FieldState {
  return {
    id: uid(),
    name: "",
    type: "string",
    required: true,
    patternPreset: "",
    customPattern: "",
    enumInput: "",
    enumValues: [],
    min: "",
    max: "",
    minLength: "",
    maxLength: "",
    description: "",
    constraints: "",
  };
}

function defaultMetric(): MetricState {
  return { id: uid(), name: "", formula: "" };
}

// ---------------------------------------------------------------------------
// FieldCard
// ---------------------------------------------------------------------------

function FieldCard({
  field,
  index,
  total,
  onChange,
  onDelete,
  onMove,
}: {
  field: FieldState;
  index: number;
  total: number;
  onChange: (patch: Partial<FieldState>) => void;
  onDelete: () => void;
  onMove: (dir: -1 | 1) => void;
}) {
  const handleEnumKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Enter" || e.key === ",") {
      e.preventDefault();
      const val = field.enumInput.trim().replace(/,$/, "");
      if (val && !field.enumValues.includes(val)) {
        onChange({ enumValues: [...field.enumValues, val], enumInput: "" });
      } else {
        onChange({ enumInput: "" });
      }
    }
  };

  const removeEnum = (v: string) => {
    onChange({ enumValues: field.enumValues.filter((x) => x !== v) });
  };

  const selectedPreset = PATTERN_PRESETS.find((p) => p.value === field.patternPreset) ?? PATTERN_PRESETS[0];

  return (
    <div className={clsx(
      "bg-[#0f1623] border rounded-xl p-4 space-y-3 transition-colors",
      field.name.trim() ? "border-[#1f2937]" : "border-[#1f2937]/50"
    )}>
      {/* Top row: name + type + required + actions */}
      <div className="flex items-center gap-2 flex-wrap">
        {/* Drag handle / order */}
        <div className="flex flex-col gap-0.5 shrink-0">
          <button
            onClick={() => onMove(-1)}
            disabled={index === 0}
            className="text-slate-600 hover:text-slate-400 disabled:opacity-20 text-xs leading-none"
            title="Move up"
          >▲</button>
          <button
            onClick={() => onMove(1)}
            disabled={index === total - 1}
            className="text-slate-600 hover:text-slate-400 disabled:opacity-20 text-xs leading-none"
            title="Move down"
          >▼</button>
        </div>

        {/* Field name */}
        <input
          type="text"
          value={field.name}
          onChange={(e) => onChange({ name: e.target.value })}
          placeholder="field_name"
          className="flex-1 min-w-[120px] bg-[#0a0d12] border border-[#1f2937] rounded-lg px-3 py-1.5 text-sm text-slate-200 font-mono outline-none focus:border-blue-600"
        />

        {/* Type selector */}
        <div className="flex gap-1 shrink-0">
          {(["string", "integer", "number", "boolean"] as FieldType[]).map((t) => (
            <button
              key={t}
              onClick={() => onChange({ type: t, enumValues: [], patternPreset: "", customPattern: "", min: "", max: "", minLength: "", maxLength: "" })}
              className={clsx(
                "px-2.5 py-1 text-xs rounded-lg border font-medium transition-colors",
                field.type === t
                  ? TYPE_COLORS[t]
                  : "bg-[#111827] border-[#1f2937] text-slate-500 hover:text-slate-300"
              )}
            >
              {t}
            </button>
          ))}
        </div>

        {/* Required toggle */}
        <button
          onClick={() => onChange({ required: !field.required })}
          className={clsx(
            "px-2.5 py-1 text-xs rounded-lg border font-medium transition-colors shrink-0",
            field.required
              ? "bg-green-900/30 border-green-700/50 text-green-400"
              : "bg-[#111827] border-[#1f2937] text-slate-500"
          )}
        >
          {field.required ? "required" : "optional"}
        </button>

        {/* Delete */}
        <button
          onClick={onDelete}
          className="ml-auto shrink-0 text-slate-600 hover:text-red-400 transition-colors text-lg leading-none"
          title="Remove field"
        >×</button>
      </div>

      {/* String options */}
      {field.type === "string" && (
        <div className="space-y-2 pl-7">
          {/* Pattern picker */}
          <div className="flex items-center gap-2 flex-wrap">
            <label className="text-xs text-slate-500 w-20 shrink-0">Pattern</label>
            <select
              value={field.patternPreset}
              onChange={(e) => onChange({ patternPreset: e.target.value, enumValues: [], enumInput: "" })}
              className="bg-[#111827] border border-[#1f2937] text-slate-300 text-xs rounded-lg px-2 py-1.5 outline-none focus:border-blue-600"
            >
              {PATTERN_PRESETS.map((p) => (
                <option key={p.value} value={p.value}>{p.label}</option>
              ))}
            </select>
            {selectedPreset.hint && (
              <span className="text-xs text-slate-600 font-mono">{selectedPreset.hint}</span>
            )}
          </div>

          {/* Custom pattern input */}
          {field.patternPreset === "__custom__" && (
            <div className="flex items-center gap-2">
              <label className="text-xs text-slate-500 w-20 shrink-0" />
              <input
                type="text"
                value={field.customPattern}
                onChange={(e) => onChange({ customPattern: e.target.value })}
                placeholder="^your-regex-here$"
                className="flex-1 bg-[#0a0d12] border border-[#1f2937] rounded-lg px-3 py-1.5 text-xs text-yellow-300 font-mono outline-none focus:border-yellow-600"
              />
            </div>
          )}

          {/* Enum values — only when no pattern */}
          {!field.patternPreset && (
            <div className="flex items-start gap-2">
              <label className="text-xs text-slate-500 w-20 shrink-0 pt-1.5">Enum</label>
              <div className="flex-1 space-y-1.5">
                <div className="flex flex-wrap gap-1.5">
                  {field.enumValues.map((v) => (
                    <span
                      key={v}
                      className="inline-flex items-center gap-1 bg-blue-900/30 border border-blue-700/40 text-blue-300 text-xs px-2 py-0.5 rounded-full"
                    >
                      {v}
                      <button onClick={() => removeEnum(v)} className="text-blue-500 hover:text-blue-200 leading-none">×</button>
                    </span>
                  ))}
                </div>
                <input
                  type="text"
                  value={field.enumInput}
                  onChange={(e) => onChange({ enumInput: e.target.value })}
                  onKeyDown={handleEnumKeyDown}
                  placeholder='Type value, press Enter or ","'
                  className="w-full bg-[#0a0d12] border border-[#1f2937] rounded-lg px-3 py-1.5 text-xs text-slate-300 outline-none focus:border-blue-600"
                />
              </div>
            </div>
          )}

          {/* Length constraints */}
          <div className="flex items-center gap-2 flex-wrap">
            <label className="text-xs text-slate-500 w-20 shrink-0">Length</label>
            <div className="flex items-center gap-1.5">
              <input
                type="number"
                value={field.minLength}
                onChange={(e) => onChange({ minLength: e.target.value })}
                placeholder="min"
                className="w-16 bg-[#0a0d12] border border-[#1f2937] rounded-lg px-2 py-1.5 text-xs text-slate-300 outline-none focus:border-blue-600"
              />
              <span className="text-slate-600 text-xs">–</span>
              <input
                type="number"
                value={field.maxLength}
                onChange={(e) => onChange({ maxLength: e.target.value })}
                placeholder="max"
                className="w-16 bg-[#0a0d12] border border-[#1f2937] rounded-lg px-2 py-1.5 text-xs text-slate-300 outline-none focus:border-blue-600"
              />
            </div>
          </div>
        </div>
      )}

      {/* Numeric options */}
      {(field.type === "integer" || field.type === "number") && (
        <div className="flex items-center gap-2 pl-7 flex-wrap">
          <label className="text-xs text-slate-500 w-20 shrink-0">Range</label>
          <div className="flex items-center gap-1.5">
            <input
              type="number"
              value={field.min}
              onChange={(e) => onChange({ min: e.target.value })}
              placeholder="min"
              className="w-20 bg-[#0a0d12] border border-[#1f2937] rounded-lg px-2 py-1.5 text-xs text-slate-300 outline-none focus:border-purple-600"
            />
            <span className="text-slate-600 text-xs">–</span>
            <input
              type="number"
              value={field.max}
              onChange={(e) => onChange({ max: e.target.value })}
              placeholder="max"
              className="w-20 bg-[#0a0d12] border border-[#1f2937] rounded-lg px-2 py-1.5 text-xs text-slate-300 outline-none focus:border-purple-600"
            />
          </div>
        </div>
      )}

      {/* Description (glossary) */}
      <div className="flex items-start gap-2 pl-7">
        <label className="text-xs text-slate-500 w-20 shrink-0 pt-1.5">Description</label>
        <div className="flex-1 space-y-1.5">
          <input
            type="text"
            value={field.description}
            onChange={(e) => onChange({ description: e.target.value })}
            placeholder="What does this field represent?"
            className="w-full bg-[#0a0d12] border border-[#1f2937] rounded-lg px-3 py-1.5 text-xs text-slate-300 outline-none focus:border-green-700"
          />
          {field.description && (
            <input
              type="text"
              value={field.constraints}
              onChange={(e) => onChange({ constraints: e.target.value })}
              placeholder="Constraints note (optional)"
              className="w-full bg-[#0a0d12] border border-[#1f2937] rounded-lg px-3 py-1.5 text-xs text-slate-400 outline-none focus:border-green-700"
            />
          )}
        </div>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// MetricRow
// ---------------------------------------------------------------------------

function MetricRow({
  metric,
  onChange,
  onDelete,
}: {
  metric: MetricState;
  onChange: (patch: Partial<MetricState>) => void;
  onDelete: () => void;
}) {
  return (
    <div className="flex items-center gap-2">
      <input
        type="text"
        value={metric.name}
        onChange={(e) => onChange({ name: e.target.value })}
        placeholder="metric_name"
        className="w-40 bg-[#0a0d12] border border-[#1f2937] rounded-lg px-3 py-1.5 text-xs text-slate-200 font-mono outline-none focus:border-indigo-600"
      />
      <input
        type="text"
        value={metric.formula}
        onChange={(e) => onChange({ formula: e.target.value })}
        placeholder='sum(amount) where event_type = "purchase"'
        className="flex-1 bg-[#0a0d12] border border-[#1f2937] rounded-lg px-3 py-1.5 text-xs text-indigo-300 font-mono outline-none focus:border-indigo-600"
      />
      <button
        onClick={onDelete}
        className="text-slate-600 hover:text-red-400 transition-colors text-lg leading-none shrink-0"
      >×</button>
    </div>
  );
}

// ---------------------------------------------------------------------------
// VisualBuilder
// ---------------------------------------------------------------------------

const INITIAL_STATE: BuilderState = {
  name: "my_contract",
  version: "1.0",
  description: "",
  fields: [
    {
      ...defaultField(),
      name: "user_id",
      type: "string",
      required: true,
      patternPreset: "^[a-zA-Z0-9_-]{3,64}$",
      description: "Unique user identifier",
      enumInput: "",
      enumValues: [],
      customPattern: "",
      min: "",
      max: "",
      minLength: "",
      maxLength: "",
      constraints: "",
    },
    {
      ...defaultField(),
      name: "event_type",
      type: "string",
      required: true,
      patternPreset: "",
      enumValues: ["click", "view", "purchase"],
      description: "Type of user interaction",
      enumInput: "",
      customPattern: "",
      min: "",
      max: "",
      minLength: "",
      maxLength: "",
      constraints: "",
    },
    {
      ...defaultField(),
      name: "timestamp",
      type: "integer",
      required: true,
      min: "0",
      patternPreset: "",
      enumValues: [],
      description: "Unix timestamp in seconds",
      enumInput: "",
      customPattern: "",
      max: "",
      minLength: "",
      maxLength: "",
      constraints: "",
    },
  ],
  metrics: [],
};

export default function VisualBuilder({ onSaved }: { onSaved: () => void }) {
  const [state, setState] = useState<BuilderState>(INITIAL_STATE);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [yamlCopied, setYamlCopied] = useState(false);

  const yaml = buildYaml(state);

  const patchField = useCallback((id: string, patch: Partial<FieldState>) => {
    setState((s) => ({
      ...s,
      fields: s.fields.map((f) => (f.id === id ? { ...f, ...patch } : f)),
    }));
  }, []);

  const addField = () => {
    setState((s) => ({ ...s, fields: [...s.fields, defaultField()] }));
  };

  const deleteField = (id: string) => {
    setState((s) => ({ ...s, fields: s.fields.filter((f) => f.id !== id) }));
  };

  const moveField = (id: string, dir: -1 | 1) => {
    setState((s) => {
      const idx = s.fields.findIndex((f) => f.id === id);
      if (idx < 0) return s;
      const next = [...s.fields];
      const swap = idx + dir;
      if (swap < 0 || swap >= next.length) return s;
      [next[idx], next[swap]] = [next[swap], next[idx]];
      return { ...s, fields: next };
    });
  };

  const patchMetric = (id: string, patch: Partial<MetricState>) => {
    setState((s) => ({
      ...s,
      metrics: s.metrics.map((m) => (m.id === id ? { ...m, ...patch } : m)),
    }));
  };

  const addMetric = () => {
    setState((s) => ({ ...s, metrics: [...s.metrics, defaultMetric()] }));
  };

  const deleteMetric = (id: string) => {
    setState((s) => ({ ...s, metrics: s.metrics.filter((m) => m.id !== id) }));
  };

  const handleSave = async () => {
    setSaving(true);
    setSaveError(null);
    try {
      await createContract(yaml);
      await mutate("contracts");
      onSaved();
    } catch (e: unknown) {
      setSaveError(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  };

  const handleCopyYaml = () => {
    navigator.clipboard.writeText(yaml);
    setYamlCopied(true);
    setTimeout(() => setYamlCopied(false), 2000);
  };

  return (
    <div className="grid grid-cols-1 xl:grid-cols-2 gap-6">
      {/* ---- LEFT: builder form ---- */}
      <div className="space-y-6 min-w-0">

        {/* Contract metadata */}
        <section className="bg-[#111827] border border-[#1f2937] rounded-xl p-5 space-y-3">
          <h3 className="text-xs font-semibold text-slate-400 uppercase tracking-wider">Contract Info</h3>
          <div className="grid grid-cols-2 gap-3">
            <div>
              <label className="block text-xs text-slate-500 mb-1">Name</label>
              <input
                type="text"
                value={state.name}
                onChange={(e) => setState((s) => ({ ...s, name: e.target.value }))}
                className="w-full bg-[#0a0d12] border border-[#1f2937] rounded-lg px-3 py-1.5 text-sm text-slate-200 font-mono outline-none focus:border-green-700"
                placeholder="my_contract"
              />
            </div>
            <div>
              <label className="block text-xs text-slate-500 mb-1">Version</label>
              <input
                type="text"
                value={state.version}
                onChange={(e) => setState((s) => ({ ...s, version: e.target.value }))}
                className="w-full bg-[#0a0d12] border border-[#1f2937] rounded-lg px-3 py-1.5 text-sm text-slate-200 font-mono outline-none focus:border-green-700"
                placeholder="1.0"
              />
            </div>
          </div>
          <div>
            <label className="block text-xs text-slate-500 mb-1">Description</label>
            <input
              type="text"
              value={state.description}
              onChange={(e) => setState((s) => ({ ...s, description: e.target.value }))}
              className="w-full bg-[#0a0d12] border border-[#1f2937] rounded-lg px-3 py-1.5 text-sm text-slate-300 outline-none focus:border-green-700"
              placeholder="What does this contract validate?"
            />
          </div>
        </section>

        {/* Fields */}
        <section className="space-y-3">
          <div className="flex items-center justify-between">
            <h3 className="text-xs font-semibold text-slate-400 uppercase tracking-wider">
              Fields <span className="ml-1 text-slate-600 font-normal normal-case">({state.fields.length})</span>
            </h3>
            <button
              onClick={addField}
              className="px-3 py-1.5 text-xs bg-blue-600 hover:bg-blue-500 text-white rounded-lg transition-colors font-medium"
            >
              + Add Field
            </button>
          </div>

          {state.fields.length === 0 && (
            <div className="flex items-center justify-center h-24 bg-[#111827] border border-dashed border-[#1f2937] rounded-xl text-slate-600 text-sm">
              No fields yet — click Add Field to start
            </div>
          )}

          {state.fields.map((f, i) => (
            <FieldCard
              key={f.id}
              field={f}
              index={i}
              total={state.fields.length}
              onChange={(patch) => patchField(f.id, patch)}
              onDelete={() => deleteField(f.id)}
              onMove={(dir) => moveField(f.id, dir)}
            />
          ))}
        </section>

        {/* Metrics */}
        <section className="bg-[#111827] border border-[#1f2937] rounded-xl p-5 space-y-3">
          <div className="flex items-center justify-between">
            <h3 className="text-xs font-semibold text-slate-400 uppercase tracking-wider">Metrics</h3>
            <button
              onClick={addMetric}
              className="px-3 py-1.5 text-xs bg-indigo-600/40 hover:bg-indigo-600/60 text-indigo-300 rounded-lg transition-colors font-medium"
            >
              + Add Metric
            </button>
          </div>
          {state.metrics.length === 0 ? (
            <p className="text-xs text-slate-600">
              Optional — define computed metrics like <span className="font-mono text-slate-500">sum(amount) where event_type = &quot;purchase&quot;</span>
            </p>
          ) : (
            <div className="space-y-2">
              {state.metrics.map((m) => (
                <MetricRow
                  key={m.id}
                  metric={m}
                  onChange={(patch) => patchMetric(m.id, patch)}
                  onDelete={() => deleteMetric(m.id)}
                />
              ))}
            </div>
          )}
        </section>

        {/* Save */}
        <div className="flex items-center gap-3">
          <button
            onClick={handleSave}
            disabled={saving || state.fields.filter((f) => f.name.trim()).length === 0}
            className="px-5 py-2.5 bg-green-600 hover:bg-green-500 disabled:opacity-40 text-white text-sm font-semibold rounded-lg transition-colors"
          >
            {saving ? "Saving…" : "Save Contract"}
          </button>
          <button
            onClick={() => setState(INITIAL_STATE)}
            className="px-4 py-2.5 bg-[#1f2937] hover:bg-[#374151] text-slate-300 text-sm font-medium rounded-lg transition-colors"
          >
            Reset
          </button>
          {saveError && (
            <p className="text-sm text-red-400">{saveError}</p>
          )}
        </div>
      </div>

      {/* ---- RIGHT: live YAML preview ---- */}
      <div className="min-w-0">
        <div className="sticky top-4">
          <div className="flex items-center justify-between mb-2">
            <h3 className="text-xs font-semibold text-slate-400 uppercase tracking-wider">
              Live YAML Preview
            </h3>
            <button
              onClick={handleCopyYaml}
              className="px-3 py-1 text-xs bg-[#1f2937] hover:bg-[#374151] text-slate-300 rounded-lg transition-colors"
            >
              {yamlCopied ? "✔ Copied!" : "Copy YAML"}
            </button>
          </div>
          <pre className="bg-[#0a0d12] border border-[#1f2937] rounded-xl p-5 text-green-300 font-mono text-xs leading-relaxed overflow-auto max-h-[calc(100vh-12rem)] whitespace-pre-wrap break-words">
            {yaml}
          </pre>
          <p className="mt-2 text-xs text-slate-600 text-right">
            Updates live as you edit fields →
          </p>
        </div>
      </div>
    </div>
  );
}
