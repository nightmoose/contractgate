import { describe, it } from 'node:test';
import assert from 'node:assert/strict';

import {
  AISchemaGateRepairError,
  buildRepairPrompt,
  createAISchemaGateMiddleware,
  validateOrRepairObject,
  type EgressClient,
  type StructuredViolation,
} from '../src/vercel-ai.ts';

interface EgressOpts {
  contractId: string;
  events: unknown[];
  disposition?: string;
  dryRun?: boolean;
}

function egressBody(opts: {
  ok: boolean;
  violations?: StructuredViolation[];
  payload?: Record<string, unknown> | null;
}): Record<string, unknown> {
  const violations = opts.violations ?? [];
  return {
    total: 1,
    passed: opts.ok ? 1 : 0,
    failed: opts.ok ? 0 : 1,
    dry_run: false,
    disposition: 'fail',
    resolved_version: '1.0.0',
    payload: opts.ok && opts.payload ? [opts.payload] : [],
    outcomes: [
      {
        index: 0,
        passed: opts.ok,
        violations,
        validation_us: 10,
        action: opts.ok ? 'pass' : 'fail',
      },
    ],
  };
}

function scripted(handler: (event: Record<string, unknown>, opts: EgressOpts) => unknown): {
  client: EgressClient;
  calls: EgressOpts[];
} {
  const calls: EgressOpts[] = [];
  const client: EgressClient = {
    async egress(opts) {
      const recorded = opts as EgressOpts;
      calls.push(recorded);
      const event = recorded.events[0] as Record<string, unknown>;
      const result = handler(event, recorded);
      if (result instanceof Error) throw result;
      return result;
    },
  };
  return { client, calls };
}

const rangeViolation: StructuredViolation = {
  field: 'amount',
  kind: 'range_violation',
  message: 'amount 150 is above the maximum',
  expected: '<= 100',
  received: 150,
  suggestion: 'Lower amount to 100 or less.',
};

const enumViolation: StructuredViolation = {
  field: 'currency',
  kind: 'enum_violation',
  message: 'currency must be one of USD, EUR, GBP',
  expected: 'one of [USD, EUR, GBP]',
  received: 'JPY',
};

const complianceViolation: StructuredViolation = {
  field: 'note',
  kind: 'undeclared_field',
  message: 'field note is not declared',
  suggestion: 'Remove note.',
};

describe('validateOrRepairObject', () => {
  it('returns the egress payload when the object passes', async () => {
    const { client, calls } = scripted(() =>
      egressBody({
        ok: true,
        payload: { currency: 'USD', amount: 10, email: '****' },
      }),
    );
    const result = await validateOrRepairObject(client, {
      contractId: 'invoice',
      maxRetries: 2,
      generate: async () => ({ currency: 'USD', amount: 10, email: 'a@b.co' }),
    });
    assert.equal(result.ok, true);
    assert.equal(result.attempts, 1);
    assert.deepEqual(result.repairPrompts, []);
    assert.equal(result.object['email'], '****');
    assert.equal(calls.length, 1);
    assert.equal(calls[0]!.contractId, 'invoice');
    assert.equal(calls[0]!.disposition, 'fail');
    assert.deepEqual(calls[0]!.events[0], { currency: 'USD', amount: 10, email: 'a@b.co' });
  });

  it('retries with semantic violations and accepts the repair', async () => {
    let n = 0;
    const { client } = scripted((event) => {
      n += 1;
      if (n === 1) {
        return egressBody({
          ok: false,
          violations: [rangeViolation, enumViolation, complianceViolation],
        });
      }
      assert.equal(event['currency'], 'USD');
      assert.equal(event['amount'], 40);
      assert.equal('note' in event, false);
      return egressBody({ ok: true, payload: event });
    });
    const seen: string[] = [];
    const result = await validateOrRepairObject(client, {
      contractId: 'invoice',
      maxRetries: 2,
      generate: async (ctx) => {
        if (ctx.attempt === 1) return { currency: 'JPY', amount: 150, note: 'hi' };
        seen.push(ctx.repairPrompt ?? '');
        return { currency: 'USD', amount: 40 };
      },
    });
    assert.equal(result.ok, true);
    assert.equal(result.attempts, 2);
    assert.equal(result.object['amount'], 40);
    assert.match(seen[0]!, /range_violation/);
    assert.match(seen[0]!, /enum_violation/);
    assert.match(seen[0]!, /compliance_mode/);
    assert.match(seen[0]!, /<= 100/);
    assert.match(seen[0]!, /amount/);
  });

  it('unwraps generateObject and reads a 422 egress body', async () => {
    let n = 0;
    const { client } = scripted(() => {
      n += 1;
      const body = egressBody({
        ok: n > 1,
        violations: n > 1 ? [] : [enumViolation],
        payload: n > 1 ? { currency: 'USD' } : null,
      });
      if (n === 1) {
        const err = new Error('rejected');
        Object.assign(err, { status: 422, body });
        throw err;
      }
      return body;
    });
    const result = await validateOrRepairObject(client, {
      contractId: 'invoice',
      maxRetries: 1,
      generate: async (ctx) => ({
        object: ctx.attempt === 1 ? { currency: 'JPY' } : { currency: 'USD' },
        finishReason: 'stop',
        usage: { totalTokens: 3 },
      }),
    });
    assert.equal(result.ok, true);
    assert.equal(result.attempts, 2);
    assert.equal(result.object['currency'], 'USD');
  });

  it('reports exhaustion without another provider call', async () => {
    let generates = 0;
    const { client, calls } = scripted(() =>
      egressBody({ ok: false, violations: [rangeViolation] }),
    );
    const result = await validateOrRepairObject(client, {
      contractId: 'invoice',
      maxRetries: 1,
      generate: async () => {
        generates += 1;
        return { amount: 150 };
      },
    });
    assert.equal(result.ok, false);
    assert.equal(result.attempts, 2);
    assert.equal(generates, 2);
    assert.equal(calls.length, 2);
    assert.equal(result.repairPrompts.length, 1);
    assert.equal(result.violations[0]!.kind, 'range_violation');
    assert.match(result.repairPrompts[0]!, /min, max, pattern, enum/);
  });
});

describe('createAISchemaGateMiddleware', () => {
  it('passes a clean generation through and keeps the sanitized JSON', async () => {
    const { client } = scripted(() =>
      egressBody({ ok: true, payload: { currency: 'USD', amount: 10 } }),
    );
    const mw = createAISchemaGateMiddleware({ client, contractId: 'invoice', maxRetries: 2 });
    let retried = false;
    const result = (await mw.wrapGenerate({
      doGenerate: async () => ({
        content: [{ type: 'text', text: '{"currency":"USD","amount":10,"extra":1}' }],
        finishReason: 'stop',
      }),
      params: { prompt: [{ role: 'user', content: 'extract' }] },
      model: {
        doGenerate: async () => {
          retried = true;
          return {};
        },
      },
    })) as { content: Array<{ type: string; text: string }> };
    assert.equal(retried, false);
    assert.deepEqual(JSON.parse(result.content[0]!.text), { currency: 'USD', amount: 10 });
  });

  it('feeds violations into a repair prompt and retries doGenerate', async () => {
    let n = 0;
    const { client } = scripted(() => {
      n += 1;
      if (n === 1) return egressBody({ ok: false, violations: [enumViolation, rangeViolation] });
      return egressBody({ ok: true, payload: { currency: 'EUR', amount: 5 } });
    });
    const mw = createAISchemaGateMiddleware({ client, contractId: 'invoice', maxRetries: 2, dryRun: true });
    const prompts: unknown[] = [];
    const result = (await mw.wrapGenerate({
      doGenerate: async () => ({
        content: [{ type: 'text', text: '{"currency":"JPY","amount":150}' }],
      }),
      params: { prompt: [{ role: 'user', content: [{ type: 'text', text: 'extract' }] }] },
      model: {
        doGenerate: async (params) => {
          prompts.push(params);
          return {
            content: [{ type: 'tool-call', toolName: 'emit', input: '{"currency":"EUR","amount":5}' }],
          };
        },
      },
    })) as { content: Array<{ type: string; input?: string; text?: string }> };
    assert.equal(n, 2);
    const sent = prompts[0] as { prompt: Array<{ content: Array<{ text: string }> }> };
    const repair = sent.prompt[1]!.content[0]!.text;
    assert.match(repair, /enum_violation/);
    assert.match(repair, /range_violation/);
    assert.match(repair, /JPY/);
    assert.equal(result.content[0]!.input, JSON.stringify({ currency: 'EUR', amount: 5 }));
  });

  it('throws when retries are exhausted', async () => {
    const { client } = scripted(() => egressBody({ ok: false, violations: [complianceViolation] }));
    const mw = createAISchemaGateMiddleware({ client, contractId: 'invoice', maxRetries: 1 });
    let calls = 0;
    await assert.rejects(
      () =>
        mw.wrapGenerate({
          doGenerate: async () => {
            calls += 1;
            return { text: '{"note":"nope"}' };
          },
          params: { prompt: [] },
          model: {
            doGenerate: async () => {
              calls += 1;
              return { text: '{"note":"still"}' };
            },
          },
        }),
      (err: unknown) => {
        assert.ok(err instanceof AISchemaGateRepairError);
        assert.equal(err.attempts, 2);
        assert.equal(err.violations[0]!.kind, 'undeclared_field');
        assert.match(err.repairPrompts[0]!, /compliance_mode/);
        return true;
      },
    );
    assert.equal(calls, 2);
  });
});

describe('buildRepairPrompt', () => {
  it('names the semantic constraint kinds', () => {
    const text = buildRepairPrompt([rangeViolation, enumViolation]);
    assert.match(text, /pattern/);
    assert.match(text, /enum/);
    assert.match(text, /range_violation/);
  });
});
