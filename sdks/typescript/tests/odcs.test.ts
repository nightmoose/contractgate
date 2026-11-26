import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { Contract } from '../src/contract.js';
import { ContractCompileError } from '../src/errors.js';

const load = (n: string) => readFileSync(fileURLToPath(new URL(`./${n}`, import.meta.url)), 'utf8');

describe('ODCS input', () => {
  it('parses a minimal ODCS contract with no ontology and validates events', () => {
    const c = Contract.fromYaml(load('odcs_minimal.yaml')).compile();
    assert.equal(c.contract.name, 'orders');
    assert.equal(c.contract.description, 'Minimal orders contract');
    assert.deepEqual(
      c.contract.entities.map((e) => e.name),
      ['order_id', 'amount', 'status'],
    );
    assert.equal(c.contract.entities[1]!.fieldType, 'float');
    assert.ok(c.validate({ order_id: 'a', amount: 2.5, status: 'open' }).passed);
    assert.ok(!c.validate({ order_id: 'a', amount: -1, status: 'open' }).passed);
    assert.ok(!c.validate({ order_id: 'a', amount: 1, status: 'nope' }).passed);
  });

  it('parses the official full example (map description, v3.2 enum, primaryKey)', () => {
    const c = Contract.fromYaml(load('odcs_full_example.yaml')).compile();
    assert.equal(c.contract.description, 'Views built on top of the seller tables.');
    const by = Object.fromEntries(c.contract.entities.map((e) => [e.name, e]));
    assert.deepEqual(by['rcvr_cntry_code']!.allowedValues, ['US', 'CA', 'MX']);
    assert.equal(by['rcvr_id']!.required, true);
    assert.ok(c.validate({ rcvr_id: 'x', rcvr_cntry_code: 'US' }).passed);
    assert.ok(!c.validate({ rcvr_id: 'x', rcvr_cntry_code: 'ZZ' }).passed);
  });

  it('reads physicalType when logicalType is absent and ignores unknown fields', () => {
    const c = Contract.fromYaml(`
apiVersion: v3.1.0
kind: DataContract
version: 2
name: t
futureKey: {a: 1}
schema:
  - name: t
    properties:
      - name: id
        physicalType: BIGINT
        primaryKey: true
      - name: price
        physicalType: DECIMAL(10,2)
      - name: attrs
        logicalType: map
`).compile();
    const types = c.contract.entities.map((e) => e.fieldType);
    assert.deepEqual(types, ['integer', 'float', 'object']);
    assert.equal(c.contract.version, '2');
  });

  it('keeps ontology optional for native contracts', () => {
    const c = Contract.fromYaml('version: "1.0"\nname: bare\n').compile();
    assert.deepEqual(c.contract.entities, []);
  });

  it('native contracts still need a mapping ontology when one is given', () => {
    assert.throws(
      () => Contract.fromYaml('version: "1.0"\nname: x\nontology: 5\n'),
      /ontology must be a mapping/,
    );
  });

  it('gives clear errors for unsupported ODCS', () => {
    assert.throws(
      () => Contract.fromYaml('apiVersion: v3.1.0\nkind: DataContract\nname: n\nversion: 1.0.0\n'),
      (e: unknown) => e instanceof ContractCompileError && /no 'schema' array/.test(e.message),
    );
    assert.throws(
      () =>
        Contract.fromYaml(
          'apiVersion: v2.2.2\nkind: DataContract\nname: n\nversion: 1\nschema: []\n',
        ),
      /unsupported ODCS apiVersion/,
    );
    assert.throws(
      () =>
        Contract.fromYaml(
          'apiVersion: v3.1.0\nname: n\nversion: 1\nschema:\n  - name: t\n',
        ),
      /no properties/,
    );
  });
});
