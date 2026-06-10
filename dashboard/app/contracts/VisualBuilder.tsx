"use client";

import { useState, useCallback } from "react";
import { createContract, type TransformKind, type MaskStyle } from "@/lib/api";
import { mutate } from "swr";
import clsx from "clsx";

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

type FieldType = "string" | "integer" | "number" | "boolean" | "date" | "object";

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
  // RFC-004: PII transform (string fields only; server rejects transforms
  // declared on non-string entities at contract-compile time).
  // Empty string = "no transform declared" and the transform block is
  // omitted from the emitted YAML entirely.
  transformKind: TransformKind | "";
  // Only meaningful when `transformKind === "mask"`.  "opaque" is the
  // server-side default — we emit the `style:` key in YAML only when
  // the user explicitly picks a non-default (i.e. `format_preserving`).
  maskStyle: MaskStyle | "";
  // For type: object — recursive child fields. Ignored for scalar types.
  // Mirrors the YAML `properties:` under a `type: object` entity.
  properties?: FieldState[];
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
  // RFC-004: when on, the validator raises UNDECLARED_FIELD on any
  // inbound field not listed in `ontology.entities`, AND the transform
  // engine strips those fields from the stored payload.  Off = legacy
  // behavior (undeclared fields pass through).
  complianceMode: boolean;
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
  date:    "bg-rose-900/40 text-rose-300 border-rose-700/50",
  object:  "bg-indigo-900/40 text-indigo-300 border-indigo-700/50",
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

  // RFC-004: emit compliance_mode only when true.  Default `false` matches
  // the server-side `#[serde(default)]`, so omitting it keeps the legacy
  // pre-RFC-004 YAML shape byte-identical for contracts that don't opt in.
  if (state.complianceMode) {
    lines.push("compliance_mode: true");
  }

  lines.push("", "ontology:", "  entities:");

  const validFields = state.fields.filter((f) => f.name.trim());
  if (validFields.length === 0) {
    lines.push("    [] # add fields above");
  } else {
    for (const f of validFields) {
      lines.push(...emitField(f, 4));
    }
  }

  // Glossary — only fields that have a description.
  // For nested objects we use dot-paths (e.g. _cg.source) so the server
  // glossary + quality rules can target them (see RFC-077 RAG profile).
  const withDesc = collectDescribedFields(validFields);
  lines.push("", "glossary:");
  if (withDesc.length === 0) {
    lines[lines.length - 1] = "glossary: []";
  } else {
    for (const entry of withDesc) {
      lines.push(`  - field: "${entry.path}"`);
      lines.push(`    description: "${entry.description}"`);
      if (entry.constraints) {
        lines.push(`    constraints: "${entry.constraints}"`);
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

// Recursive emitter for a single entity (supports type: object + properties).
// itemIndent is the column for the leading "- " (e.g. 4 for top-level entities).
function emitField(f: FieldState, itemIndent: number): string[] {
  const keyIndent = itemIndent + 2;
  const itemPad = " ".repeat(itemIndent);
  const keyPad = " ".repeat(keyIndent);

  const lines: string[] = [
    `${itemPad}- name: ${f.name.trim()}`,
    `${keyPad}type: ${f.type}`,
    `${keyPad}required: ${f.required}`,
  ];

  if (f.type === "object") {
    // Only emit `properties:` when at least one child has a name — avoids
    // a bare `properties:` key (YAML null) when all children are still unnamed.
    const namedChildren = (f.properties || []).filter((c) => c.name.trim());
    if (namedChildren.length > 0) {
      lines.push(`${keyPad}properties:`);
      const childItemIndent = keyIndent + 2; // children listed under "properties:"
      for (const child of namedChildren) {
        lines.push(...emitField(child, childItemIndent));
      }
    }
  } else {
    // Scalar attribute emission (unchanged behavior for non-object fields).
    const pattern = resolvedPattern(f);
    if (f.type === "string" && pattern) {
      lines.push(`${keyPad}pattern: "${pattern}"`);
    }
    if (f.type === "string" && f.enumValues.length > 0 && !pattern) {
      lines.push(`${keyPad}enum:`);
      for (const v of f.enumValues) lines.push(`${keyPad}  - "${v}"`);
    }
    if ((f.type === "integer" || f.type === "number") && f.min !== "") {
      lines.push(`${keyPad}min: ${f.min}`);
    }
    if ((f.type === "integer" || f.type === "number") && f.max !== "") {
      lines.push(`${keyPad}max: ${f.max}`);
    }
    if (f.type === "string" && f.minLength !== "") {
      lines.push(`${keyPad}min_length: ${f.minLength}`);
    }
    if (f.type === "string" && f.maxLength !== "") {
      lines.push(`${keyPad}max_length: ${f.maxLength}`);
    }
    // RFC-004: emit the PII transform block when declared.  Only
    // strings can carry transforms (gated in the UI); emit `style`
    // only for mask, and only when non-default (opaque is default).
    if (f.type === "string" && f.transformKind) {
      lines.push(`${keyPad}transform:`);
      lines.push(`${keyPad}  kind: ${f.transformKind}`);
      if (f.transformKind === "mask" && f.maskStyle === "format_preserving") {
        lines.push(`${keyPad}  style: format_preserving`);
      }
    }
  }

  return lines;
}

// Collect (path, description, constraints) for glossary.
// For nested objects we emit dot-paths so quality rules and the server
// can target them (e.g. "_cg.source").
function collectDescribedFields(fields: FieldState[], prefix = ""): Array<{ path: string; description: string; constraints: string }> {
  const out: Array<{ path: string; description: string; constraints: string }> = [];
  for (const f of fields) {
    if (!f.name.trim()) continue;
    const path = prefix ? `${prefix}.${f.name.trim()}` : f.name.trim();
    if (f.description.trim()) {
      out.push({
        path,
        description: f.description.trim(),
        constraints: f.constraints.trim(),
      });
    }
    if (f.type === "object" && f.properties) {
      out.push(...collectDescribedFields(f.properties, path));
    }
  }
  return out;
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
    transformKind: "",
    maskStyle: "",
  };
}

function defaultMetric(): MetricState {
  return { id: uid(), name: "", formula: "" };
}

// ---------------------------------------------------------------------------
// Recursive tree helpers for nested object fields (RFC-080)
// ---------------------------------------------------------------------------

function updateFieldTree(fields: FieldState[], id: string, patch: Partial<FieldState>): FieldState[] {
  return fields.map((f) => {
    if (f.id === id) {
      return { ...f, ...patch };
    }
    if (f.properties && f.properties.length > 0) {
      return {
        ...f,
        properties: updateFieldTree(f.properties, id, patch),
      };
    }
    return f;
  });
}

function removeFieldTree(fields: FieldState[], id: string): FieldState[] {
  const filtered = fields.filter((f) => f.id !== id);
  return filtered.map((f) => {
    if (f.properties && f.properties.length > 0) {
      return {
        ...f,
        properties: removeFieldTree(f.properties, id),
      };
    }
    return f;
  });
}

function addFieldToTree(fields: FieldState[], parentId: string, newField: FieldState): FieldState[] {
  return fields.map((f) => {
    if (f.id === parentId) {
      const existing = f.properties || [];
      return {
        ...f,
        properties: [...existing, newField],
      };
    }
    if (f.properties && f.properties.length > 0) {
      return {
        ...f,
        properties: addFieldToTree(f.properties, parentId, newField),
      };
    }
    return f;
  });
}

function moveFieldInTree(fields: FieldState[], id: string, dir: -1 | 1): FieldState[] {
  for (let i = 0; i < fields.length; i++) {
    const f = fields[i];
    if (f.id === id) {
      const newIdx = i + dir;
      if (newIdx < 0 || newIdx >= fields.length) return fields;
      const next = [...fields];
      [next[i], next[newIdx]] = [next[newIdx], next[i]];
      return next;
    }
    if (f.properties && f.properties.length > 0) {
      const updated = moveFieldInTree(f.properties, id, dir);
      if (updated !== f.properties) {
        return fields.map((ff, idx) =>
          idx === i ? { ...ff, properties: updated } : ff
        );
      }
    }
  }
  return fields;
}

// ---------------------------------------------------------------------------
// FieldCard
// ---------------------------------------------------------------------------

function FieldCard({
  field,
  index,
  total,
  updateField,
  deleteField,
  moveField,
  addChildTo,
}: {
  field: FieldState;
  index: number;
  total: number;
  updateField: (id: string, patch: Partial<FieldState>) => void;
  deleteField: (id: string) => void;
  moveField: (id: string, dir: -1 | 1) => void;
  addChildTo?: (parentId: string) => void;
}) {
  // Bind the tree-aware dispatchers to this field's id so the rest of the
  // (large) FieldCard body can continue to use the original onChange/onDelete/onMove names.
  const onChange = (patch: Partial<FieldState>) => updateField(field.id, patch);
  const onDelete = () => deleteField(field.id);
  const onMove = (dir: -1 | 1) => moveField(field.id, dir);
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
          {(["string", "integer", "number", "boolean", "date", "object"] as FieldType[]).map((t) => (
            <button
              key={t}
              onClick={() => {
                const base: Partial<FieldState> = {
                  type: t,
                  enumValues: [],
                  patternPreset: "",
                  customPattern: "",
                  min: "",
                  max: "",
                  minLength: "",
                  maxLength: "",
                  transformKind: "",
                  maskStyle: "",
                };
                if (t === "object") {
                  base.properties = field.properties || [];
                } else if (field.type === "object") {
                  // switching away from object: drop children (can be re-added)
                  base.properties = undefined;
                }
                onChange(base);
              }}
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

      {/* Nested object editor (RFC-080) */}
      {field.type === "object" && (
        <div className="pl-7 mt-1 border-l-2 border-[#1f2937] space-y-2">
          <div className="flex items-center justify-between">
            <span className="text-xs text-slate-500">Properties</span>
            <button
              onClick={() => addChildTo?.(field.id)}
              className="px-2 py-0.5 text-[10px] bg-blue-600/70 hover:bg-blue-600 text-white rounded transition-colors"
            >
              + Add field
            </button>
          </div>
          {(field.properties || []).length === 0 && (
            <div className="text-[10px] text-slate-600 italic pl-1">No nested fields — add children for the object</div>
          )}
          {(field.properties || []).map((child, j) => (
            <FieldCard
              key={child.id}
              field={child}
              index={j}
              total={(field.properties || []).length}
              updateField={updateField}
              deleteField={deleteField}
              moveField={moveField}
              addChildTo={addChildTo}
            />
          ))}
        </div>
      )}

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

          {/* RFC-004: PII transform — only valid on string fields.
              Selecting a kind means the raw value is replaced before any
              durable write.  Mask has a sub-style; the other kinds take
              no options. */}
          <div className="flex items-center gap-2 flex-wrap">
            <label className="text-xs text-slate-500 w-20 shrink-0" title="Applied after validation; raw value never reaches audit / forward.">
              Transform
            </label>
            <select
              value={field.transformKind}
              onChange={(e) => {
                const next = e.target.value as TransformKind | "";
                // When switching away from mask, drop the style too so
                // the emitted YAML stays minimal.
                onChange({
                  transformKind: next,
                  maskStyle: next === "mask" ? field.maskStyle : "",
                });
              }}
              className="bg-[#111827] border border-[#1f2937] text-slate-300 text-xs rounded-lg px-2 py-1.5 outline-none focus:border-amber-600"
            >
              <option value="">None</option>
              <option value="mask">Mask</option>
              <option value="hash">Hash (HMAC-SHA256)</option>
              <option value="drop">Drop</option>
              <option value="redact">Redact</option>
            </select>
            {field.transformKind === "mask" && (
              <select
                value={field.maskStyle}
                onChange={(e) => onChange({ maskStyle: e.target.value as MaskStyle | "" })}
                className="bg-[#111827] border border-[#1f2937] text-slate-300 text-xs rounded-lg px-2 py-1.5 outline-none focus:border-amber-600"
              >
                <option value="">Opaque (****)</option>
                <option value="format_preserving">Format preserving</option>
              </select>
            )}
            {field.transformKind && (
              <span className="text-[10px] text-amber-400/70 font-medium inline-flex items-center gap-1">
                <span aria-hidden>🔒</span> PII — raw value never stored
              </span>
            )}
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
  complianceMode: false,
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
      fields: updateFieldTree(s.fields, id, patch),
    }));
  }, []);

  const deleteField = (id: string) => {
    setState((s) => ({
      ...s,
      fields: removeFieldTree(s.fields, id),
    }));
  };

  const moveField = (id: string, dir: -1 | 1) => {
    setState((s) => ({
      ...s,
      fields: moveFieldInTree(s.fields, id, dir),
    }));
  };

  const addField = () => {
    setState((s) => ({ ...s, fields: [...s.fields, defaultField()] }));
  };

  const addChildTo = (parentId: string) => {
    setState((s) => ({
      ...s,
      fields: addFieldToTree(s.fields, parentId, defaultField()),
    }));
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

          {/* RFC-004: compliance mode toggle.  Once a version is promoted
              (stable / deprecated) the server freezes this flag — you can
              only change it while the version is still a draft. */}
          <label className="flex items-start gap-2.5 pt-1 cursor-pointer group">
            <input
              type="checkbox"
              checked={state.complianceMode}
              onChange={(e) => setState((s) => ({ ...s, complianceMode: e.target.checked }))}
              className="mt-0.5 h-4 w-4 rounded border-[#1f2937] bg-[#0a0d12] text-amber-600 focus:ring-amber-600 focus:ring-offset-0 accent-amber-600"
            />
            <span className="flex-1">
              <span className="flex items-center gap-1.5 text-sm text-slate-300 group-hover:text-slate-200">
                <span aria-hidden>🔒</span>
                Compliance mode
                {state.complianceMode && (
                  <span className="text-[10px] font-semibold uppercase tracking-wider text-amber-400 bg-amber-950/60 border border-amber-800/60 rounded px-1.5 py-0.5">
                    ON
                  </span>
                )}
              </span>
              <span className="block text-xs text-slate-500 mt-0.5">
                Reject events containing fields not declared above, and strip them from the stored payload. Frozen once this version is promoted.
              </span>
            </span>
          </label>
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
              updateField={patchField}
              deleteField={deleteField}
              moveField={moveField}
              addChildTo={addChildTo}
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
