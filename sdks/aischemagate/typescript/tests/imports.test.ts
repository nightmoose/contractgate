import { describe, it } from 'node:test';
import assert from 'node:assert/strict';

import {
  AISCHEMAGATE_VERSION,
  AiSchemaGateRejectedError,
  Client,
  LLMGate,
  LLMResponseError,
  MAX_RETRIES_CAP,
  aischemagateGuardrail,
  extractJson,
  type LLMGateResult,
  type Violation,
} from '../src/index.js';

describe('@nightmoose/aischemagate', () => {
  it('ships its own version', () => {
    assert.equal(AISCHEMAGATE_VERSION, '0.4.1');
  });

  it('re-exports LLMGate class', () => {
    assert.equal(typeof LLMGate, 'function');
    assert.equal(LLMGate.name, 'LLMGate');
  });

  it('re-exports LLMResponseError', () => {
    assert.equal(typeof LLMResponseError, 'function');
    const err = new LLMResponseError('x');
    assert.ok(err instanceof Error);
  });

  it('re-exports Client (contractgate core)', () => {
    assert.equal(typeof Client, 'function');
  });

  it('re-exports extractJson helper', () => {
    assert.equal(typeof extractJson, 'function');
    assert.deepEqual(extractJson('{"a": 1}'), { a: 1 });
  });

  it('re-exports MAX_RETRIES_CAP constant', () => {
    assert.equal(MAX_RETRIES_CAP, 5);
  });

  it('re-exports the AI SDK guardrail', () => {
    assert.equal(typeof aischemagateGuardrail, 'function');
    assert.equal(aischemagateGuardrail.name, 'aischemagateGuardrail');
    const mw = aischemagateGuardrail({ apiKey: 'k', contractId: 'c' });
    assert.equal(mw.specificationVersion, 'v4');
    assert.equal(typeof AiSchemaGateRejectedError, 'function');
  });

  it('LLMGateResult / Violation types compile through the alias', () => {
    // Pure type-level check: this file must typecheck for the aliases to work.
    const result: LLMGateResult = {
      ok: true,
      payload: { a: 1 },
      violations: [] as Violation[],
      attempts: 1,
      rawResponse: null,
      repairHints: [],
    };
    assert.equal(result.ok, true);
  });

  it('LLMGate needs an llm only for completion()', async () => {
    const gate = new LLMGate({ apiKey: 'k', contractId: 'c' });
    await assert.rejects(
      gate.completion({ model: 'm', messages: [] }),
      (err: Error) => err instanceof TypeError && /need an `llm` client/.test(err.message),
    );
  });
});
