import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { Contract } from '../src/contract.js';

const BASE_YAML = `
version: "1.0"
name: "user_events"
ontology:
  entities:
    - name: user_id
      type: string
      required: true
      pattern: "^[a-zA-Z0-9_-]+$"
      min_length: 3
      max_length: 50
    - name: event_type
      type: string
      required: true
      enum: ["click", "view", "purchase", "login"]
    - name: timestamp
      type: integer
      required: true
      min: 0
    - name: amount
      type: number
      required: false
      min: 0
`;

const COMPLIANCE_YAML = `
version: "1.0"
name: "compliance_test"
compliance_mode: true
ontology:
  entities:
    - name: user_id
      type: string
      required: true
    - name: event_type
      type: string
      required: true
`;

function compiled(yaml = BASE_YAML) {
  return Contract.fromYaml(yaml).compile();
}

// ---------------------------------------------------------------------------
// Happy path
// ---------------------------------------------------------------------------

describe('validator', () => {
  it('valid event passes', () => {
    const r = compiled().validate({ user_id: 'alice_01', event_type: 'click', timestamp: 1712000000 });
    assert.ok(r.passed, JSON.stringify(r.violations));
  });

  it('valid event with optional amount passes', () => {
    const r = compiled().validate({
      user_id: 'alice_01', event_type: 'purchase', timestamp: 1712000000, amount: 99.99,
    });
    assert.ok(r.passed, JSON.stringify(r.violations));
  });

  // ── MISSING_REQUIRED_FIELD ───────────────────────────────────────────────

  it('missing required field — user_id', () => {
    const r = compiled().validate({ event_type: 'click', timestamp: 1712000000 });
    assert.ok(!r.passed);
    assert.ok(r.violations.some((v) => v.kind === 'missing_required_field' && v.field === 'user_id'));
  });

  it('missing required field — timestamp', () => {
    const r = compiled().validate({ user_id: 'alice_01', event_type: 'click' });
    assert.ok(!r.passed);
    assert.ok(r.violations.some((v) => v.kind === 'missing_required_field' && v.field === 'timestamp'));
  });

  // ── TYPE_MISMATCH ────────────────────────────────────────────────────────

  it('type mismatch — timestamp is a string', () => {
    const r = compiled().validate({ user_id: 'alice_01', event_type: 'click', timestamp: 'not-a-number' });
    assert.ok(!r.passed);
    assert.ok(r.violations.some((v) => v.kind === 'type_mismatch' && v.field === 'timestamp'));
  });

  it('type mismatch — user_id is an integer', () => {
    const r = compiled().validate({ user_id: 42, event_type: 'click', timestamp: 1712000000 });
    assert.ok(!r.passed);
    assert.ok(r.violations.some((v) => v.kind === 'type_mismatch' && v.field === 'user_id'));
  });

  it('boolean rejected as integer', () => {
    const r = compiled().validate({ user_id: 'alice_01', event_type: 'click', timestamp: true });
    assert.ok(!r.passed);
    assert.ok(r.violations.some((v) => v.kind === 'type_mismatch' && v.field === 'timestamp'));
  });

  it('non-object event returns type_mismatch on <root>', () => {
    const r = compiled().validate(['not', 'an', 'object']);
    assert.ok(!r.passed);
    assert.equal(r.violations[0]?.field, '<root>');
    assert.equal(r.violations[0]?.kind, 'type_mismatch');
  });

  // ── PATTERN_MISMATCH ─────────────────────────────────────────────────────

  it('pattern mismatch — user_id has spaces', () => {
    const r = compiled().validate({ user_id: 'alice 01', event_type: 'click', timestamp: 1712000000 });
    assert.ok(!r.passed);
    assert.ok(r.violations.some((v) => v.kind === 'pattern_mismatch' && v.field === 'user_id'));
  });

  it('pattern mismatch — user_id has special chars', () => {
    const r = compiled().validate({ user_id: 'alice!@#', event_type: 'click', timestamp: 1712000000 });
    assert.ok(!r.passed);
    assert.ok(r.violations.some((v) => v.kind === 'pattern_mismatch'));
  });

  // ── ENUM_VIOLATION ───────────────────────────────────────────────────────

  it('enum violation — event_type not in allowed set', () => {
    const r = compiled().validate({ user_id: 'alice_01', event_type: 'delete', timestamp: 1712000000 });
    assert.ok(!r.passed);
    assert.ok(r.violations.some((v) => v.kind === 'enum_violation' && v.field === 'event_type'));
  });

  // ── RANGE_VIOLATION ──────────────────────────────────────────────────────

  it('range violation — timestamp below min 0', () => {
    const r = compiled().validate({ user_id: 'alice_01', event_type: 'click', timestamp: -1 });
    assert.ok(!r.passed);
    assert.ok(r.violations.some((v) => v.kind === 'range_violation' && v.field === 'timestamp'));
  });

  it('range violation — amount below min 0', () => {
    const r = compiled().validate({
      user_id: 'alice_01', event_type: 'purchase', timestamp: 1712000000, amount: -1,
    });
    assert.ok(!r.passed);
    assert.ok(r.violations.some((v) => v.kind === 'range_violation' && v.field === 'amount'));
  });

  // ── LENGTH_VIOLATION ─────────────────────────────────────────────────────

  it('length violation — user_id below min_length', () => {
    const r = compiled().validate({ user_id: 'ab', event_type: 'click', timestamp: 1712000000 });
    assert.ok(!r.passed);
    assert.ok(r.violations.some((v) => v.kind === 'length_violation' && v.field === 'user_id'));
  });

  it('length violation — user_id above max_length', () => {
    const r = compiled().validate({
      user_id: 'a'.repeat(51), event_type: 'click', timestamp: 1712000000,
    });
    assert.ok(!r.passed);
    assert.ok(r.violations.some((v) => v.kind === 'length_violation' && v.field === 'user_id'));
  });

  // ── METRIC_RANGE_VIOLATION ───────────────────────────────────────────────

  it('metric range violation — exceeds max', () => {
    const yaml = `
version: "1.0"
name: "x"
ontology:
  entities:
    - name: latency
      type: float
metrics:
  - name: latency_ms
    field: latency
    max: 500
`;
    const r = Contract.fromYaml(yaml).compile().validate({ latency: 999 });
    assert.ok(!r.passed);
    assert.ok(r.violations.some((v) => v.kind === 'metric_range_violation'));
  });

  it('metric missing field — reports missing_required_field', () => {
    const yaml = `
version: "1.0"
name: "x"
ontology:
  entities:
    - name: latency
      type: float
      required: false
metrics:
  - name: latency_ms
    field: latency
    max: 500
`;
    const r = Contract.fromYaml(yaml).compile().validate({});
    assert.ok(!r.passed);
    assert.ok(r.violations.some((v) => v.kind === 'missing_required_field'));
  });

  // ── COMPLIANCE MODE ──────────────────────────────────────────────────────

  it('compliance mode — undeclared field is rejected', () => {
    const r = compiled(COMPLIANCE_YAML).validate({ user_id: 'x', event_type: 'click', stray: 1 });
    assert.ok(!r.passed);
    assert.ok(r.violations.some((v) => v.kind === 'undeclared_field' && v.field === 'stray'));
  });

  it('compliance mode — declared only passes', () => {
    const r = compiled(COMPLIANCE_YAML).validate({ user_id: 'x', event_type: 'click' });
    assert.ok(r.passed, JSON.stringify(r.violations));
  });

  it('compliance mode — missing field violation appears before undeclared', () => {
    const r = compiled(COMPLIANCE_YAML).validate({ user_id: 'x', stray: 1 });
    assert.ok(!r.passed);
    const kinds = r.violations.map((v) => v.kind);
    const missingIdx = kinds.indexOf('missing_required_field');
    const undeclaredIdx = kinds.indexOf('undeclared_field');
    assert.ok(missingIdx < undeclaredIdx, `missing at ${missingIdx}, undeclared at ${undeclaredIdx}`);
  });

  // ── NESTED / ARRAY ───────────────────────────────────────────────────────

  it('nested object — dotted field path', () => {
    const yaml = `
version: "1.0"
name: "x"
ontology:
  entities:
    - name: user
      type: object
      properties:
        - name: address
          type: object
          properties:
            - name: zip
              type: string
              pattern: "^[0-9]{5}$"
`;
    const r = Contract.fromYaml(yaml).compile().validate({ user: { address: { zip: 'abc' } } });
    assert.ok(!r.passed);
    assert.ok(r.violations.some((v) => v.field === 'user.address.zip'));
  });

  it('array item — indexed path on type mismatch', () => {
    const yaml = `
version: "1.0"
name: "x"
ontology:
  entities:
    - name: tags
      type: array
      items:
        name: tag
        type: string
`;
    const r = Contract.fromYaml(yaml).compile().validate({ tags: ['ok', 42, 'good'] });
    assert.ok(!r.passed);
    assert.ok(r.violations.some((v) => v.field === 'tags[1]' && v.kind === 'type_mismatch'));
  });
});
