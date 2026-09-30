// Walks tests/conformance/*.json, parses the contract YAML in each fixture,
// runs the local TypeScript validator, and asserts the result matches expected.
// The same fixture corpus drives equivalent suites in Rust and Python.
import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, readdirSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { Contract } from '../src/contract.js';
import type { ViolationKind } from '../src/types.js';

const __dirname = dirname(fileURLToPath(import.meta.url));
const CONFORMANCE_DIR = join(__dirname, '..', '..', '..', 'tests', 'conformance');

interface ExpectedViolation {
  field: string;
  kind: ViolationKind;
}

interface ConformanceFixture {
  name: string;
  contract_yaml: string;
  event: unknown;
  expected: {
    passed: boolean;
    violations: ExpectedViolation[];
  };
}

describe('conformance corpus', () => {
  let fixtures: string[];
  try {
    fixtures = readdirSync(CONFORMANCE_DIR).filter((f) => f.endsWith('.json')).sort();
  } catch {
    it('corpus directory not found — skip', () => {
      assert.fail(`${CONFORMANCE_DIR} does not exist`);
    });
    fixtures = [];
  }

  for (const file of fixtures) {
    it(file.replace('.json', ''), () => {
      const raw = readFileSync(join(CONFORMANCE_DIR, file), 'utf8');
      const fixture = JSON.parse(raw) as ConformanceFixture;

      const compiled = Contract.fromYaml(fixture.contract_yaml).compile();
      const result = compiled.validate(fixture.event);

      assert.equal(
        result.passed,
        fixture.expected.passed,
        `passed mismatch: got ${result.passed}, want ${fixture.expected.passed}\n` +
          `violations: ${JSON.stringify(result.violations, null, 2)}`,
      );

      // Check every expected violation is present (field + kind).
      for (const expected of fixture.expected.violations) {
        const found = result.violations.some(
          (v) => v.field === expected.field && v.kind === expected.kind,
        );
        assert.ok(
          found,
          `Expected violation {field: '${expected.field}', kind: '${expected.kind}'} not found.\n` +
            `Actual violations: ${JSON.stringify(result.violations)}`,
        );
      }

      // Violation count must match.
      assert.equal(
        result.violations.length,
        fixture.expected.violations.length,
        `Violation count mismatch: got ${result.violations.length}, ` +
          `want ${fixture.expected.violations.length}\n` +
          `Actual: ${JSON.stringify(result.violations)}`,
      );
    });
  }
});
