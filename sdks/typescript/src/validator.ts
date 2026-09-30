// Pure TypeScript port of src/validation.rs — same check order, same violation
// kind values, same field-path format, same message text as the Rust/Python engines.
// The shared fixture corpus under tests/conformance/ locks all three against drift.
//
// import type only — no runtime import from contract.ts (avoids circular in ESM).
import type { CompiledContract, FieldDefinition, MetricDefinition } from './contract.js';
import { fieldTypeDisplay } from './contract.js';
import type { ValidationResult, Violation, ViolationKind } from './types.js';

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

export function validate(compiled: CompiledContract, event: unknown): ValidationResult {
  const t0 = performance.now();
  const violations: Violation[] = [];

  if (event === null || typeof event !== 'object' || Array.isArray(event)) {
    return {
      passed: false,
      violations: [
        {
          field: '<root>',
          message: 'Event must be a JSON object',
          kind: 'type_mismatch',
        },
      ],
      validation_us: 0,
    };
  }

  const ev = event as Record<string, unknown>;

  // 1. Ontology fields
  validateFields(compiled.contract.entities, ev, '', compiled.patterns, violations);

  // 2. Metric definitions
  for (const metric of compiled.contract.metrics) {
    validateMetric(metric, ev, violations);
  }

  // 3. Compliance-mode undeclared field check (last — keeps standard violations
  //    first, matching Rust order so triage workflows need no special-casing).
  if (compiled.contract.complianceMode) {
    for (const key of Object.keys(ev)) {
      if (!compiled.declaredTopLevelFields.has(key)) {
        violations.push({
          field: key,
          message:
            `Field '${key}' is not declared in the contract ontology. ` +
            'Compliance mode rejects undeclared fields.',
          kind: 'undeclared_field',
        });
      }
    }
  }

  const elapsed_us = Math.round((performance.now() - t0) * 1000);
  return { passed: violations.length === 0, violations, validation_us: elapsed_us };
}

// ---------------------------------------------------------------------------
// Field walker
// ---------------------------------------------------------------------------

function validateFields(
  fields: FieldDefinition[],
  data: Record<string, unknown>,
  prefix: string,
  patterns: Map<string, RegExp>,
  violations: Violation[],
): void {
  for (const f of fields) {
    const path = prefix ? `${prefix}.${f.name}` : f.name;
    if (!(f.name in data)) {
      if (f.required) {
        violations.push({
          field: path,
          message: `Required field '${f.name}' is missing`,
          kind: 'missing_required_field',
        });
      }
      continue;
    }
    validateValue(f, data[f.name], path, patterns, violations);
  }
}

function validateValue(
  f: FieldDefinition,
  value: unknown,
  path: string,
  patterns: Map<string, RegExp>,
  violations: Violation[],
): void {
  // --- Type check ---
  if (!typeMatches(f.fieldType, value)) {
    violations.push({
      field: path,
      message:
        `Field '${path}' expected type ${fieldTypeDisplay(f.fieldType)}, ` +
        `got ${jsonTypeName(value)}`,
      kind: 'type_mismatch',
    });
    return; // Further checks on the wrong type are noise.
  }

  // --- String checks ---
  if (typeof value === 'string') {
    if (f.minLength != null && value.length < f.minLength) {
      violations.push({
        field: path,
        message:
          `Field '${path}' length ${value.length} is below minimum ${f.minLength}`,
        kind: 'length_violation',
      });
    }
    if (f.maxLength != null && value.length > f.maxLength) {
      violations.push({
        field: path,
        message:
          `Field '${path}' length ${value.length} exceeds maximum ${f.maxLength}`,
        kind: 'length_violation',
      });
    }
    const regex = patterns.get(path);
    if (regex != null && !regex.test(value)) {
      violations.push({
        field: path,
        message:
          `Field '${path}' value ${JSON.stringify(value)} does not match required pattern`,
        kind: 'pattern_mismatch',
      });
    }
  }

  // --- Numeric range checks ---
  const n = numericValue(value);
  if (n !== null) {
    if (f.min != null && n < f.min) {
      violations.push({
        field: path,
        message: `Field '${path}' value ${formatNumber(n)} is below minimum ${formatNumber(f.min)}`,
        kind: 'range_violation',
      });
    }
    if (f.max != null && n > f.max) {
      violations.push({
        field: path,
        message:
          `Field '${path}' value ${formatNumber(n)} exceeds maximum ${formatNumber(f.max)}`,
        kind: 'range_violation',
      });
    }
  }

  // --- Enum check ---
  if (f.allowedValues != null && !f.allowedValues.some((av) => deepEqual(av, value))) {
    const rendered = f.allowedValues.map((v) => JSON.stringify(v)).join(', ');
    violations.push({
      field: path,
      message:
        `Field '${path}' value ${JSON.stringify(value)} not in allowed set: [${rendered}]`,
      kind: 'enum_violation',
    });
  }

  // --- Recurse into nested objects ---
  if (f.fieldType === 'object' && f.properties && typeof value === 'object' && value !== null) {
    validateFields(
      f.properties,
      value as Record<string, unknown>,
      path,
      patterns,
      violations,
    );
  }

  // --- Recurse into array items ---
  if (f.fieldType === 'array' && f.items != null && Array.isArray(value)) {
    for (let i = 0; i < value.length; i++) {
      validateValue(f.items, value[i], `${path}[${i}]`, patterns, violations);
    }
  }
}

// ---------------------------------------------------------------------------
// Metric walker
// ---------------------------------------------------------------------------

function validateMetric(
  metric: MetricDefinition,
  event: Record<string, unknown>,
  violations: Violation[],
): void {
  if (metric.field == null) return; // formula-only metric
  if (metric.min == null && metric.max == null) return;

  const rawValue = resolvePath(event, metric.field);
  const n = rawValue != null ? numericValue(rawValue) : null;
  if (n === null) {
    violations.push({
      field: metric.field,
      message:
        `Metric '${metric.name}' field '${metric.field}' is missing or not numeric`,
      kind: 'missing_required_field',
    });
    return;
  }
  if (metric.min != null && n < metric.min) {
    violations.push({
      field: metric.field,
      message:
        `Metric '${metric.name}' value ${formatNumber(n)} is below ` +
        `minimum ${formatNumber(metric.min)} (field: '${metric.field}')`,
      kind: 'metric_range_violation',
    });
  }
  if (metric.max != null && n > metric.max) {
    violations.push({
      field: metric.field,
      message:
        `Metric '${metric.name}' value ${formatNumber(n)} exceeds ` +
        `maximum ${formatNumber(metric.max)} (field: '${metric.field}')`,
      kind: 'metric_range_violation',
    });
  }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

import type { FieldType } from './contract.js';

function typeMatches(ft: FieldType, value: unknown): boolean {
  switch (ft) {
    case 'string': return typeof value === 'string';
    // JSON booleans are typeof 'boolean', not 'number', so Number.isInteger
    // correctly rejects them without an extra boolean guard.
    case 'integer': return typeof value === 'number' && Number.isInteger(value);
    case 'float': return typeof value === 'number';
    case 'boolean': return typeof value === 'boolean';
    case 'object':
      return value !== null && typeof value === 'object' && !Array.isArray(value);
    case 'array': return Array.isArray(value);
    case 'any': return true;
  }
}

function numericValue(value: unknown): number | null {
  if (typeof value === 'number') return value;
  return null;
}

function resolvePath(value: unknown, path: string): unknown {
  let current = value;
  for (const key of path.split('.')) {
    if (current === null || typeof current !== 'object' || Array.isArray(current)) {
      return null;
    }
    current = (current as Record<string, unknown>)[key];
    if (current == null) return null;
  }
  return current;
}

function jsonTypeName(value: unknown): string {
  if (value === null) return 'null';
  if (typeof value === 'boolean') return 'boolean';
  if (typeof value === 'number') {
    return Number.isInteger(value) ? 'integer' : 'float';
  }
  if (typeof value === 'string') return 'string';
  if (Array.isArray(value)) return 'array';
  if (typeof value === 'object') return 'object';
  return typeof value;
}

function formatNumber(n: number): string {
  // Match Rust's Display: whole numbers render without a decimal point.
  // In JS, String(5.0) === "5" already, but be explicit for large integers.
  if (Number.isInteger(n)) return String(n);
  return String(n);
}

function deepEqual(a: unknown, b: unknown): boolean {
  return JSON.stringify(a) === JSON.stringify(b);
}

// Re-export ViolationKind type for convenience (imported by tests)
export type { ViolationKind };
