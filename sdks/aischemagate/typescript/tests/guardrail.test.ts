import { describe, it } from 'node:test';
import assert from 'node:assert/strict';

import { Client } from '@nightmoose/contractgate-sdk';
import {
  AISG_MAX_REPAIR_ATTEMPTS,
  AiSchemaGateRejectedError,
  SEMANTIC_VIOLATION_KINDS,
  aischemagateGuardrail,
  validateStructuredOutput,
  type LanguageModelCallParams,
  type LanguageModelGenerateResult,
} from '../src/ai.ts';

const CONTRACT = '11111111-1111-4111-8111-111111111111';

function jsonResponse(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'Content-Type': 'application/json' },
  });
}

function passBody(payload: Record<string, unknown>): unknown {
  return {
    total: 1,
    passed: 1,
    failed: 0,
    dry_run: false,
    disposition: 'block',
    resolved_version: '1.0.0',
    payload: [payload],
    outcomes: [{ index: 0, passed: true, violations: [], validation_us: 12, action: 'passed' }],
  };
}

function failBody(violations: Array<{ field: string; kind: string; message: string }>): unknown {
  return {
    total: 1,
    passed: 0,
    failed: 1,
    dry_run: false,
    disposition: 'block',
    resolved_version: '1.0.0',
    payload: [],
    outcomes: [
      { index: 0, passed: false, violations, validation_us: 12, action: 'blocked' },
    ],
  };
}

function clientWith(handler: (req: Request) => Response | Promise<Response>): Client {
  const fetchImpl: typeof fetch = async (input, init) => handler(new Request(input, init));
  return new Client({ baseUrl: 'https://gw.test', apiKey: 'cg_live_test', fetch: fetchImpl });
}

function textResult(text: string): LanguageModelGenerateResult {
  return { content: [{ type: 'text', text }], finishReason: 'stop' };
}

describe('aischemagateGuardrail', () => {
  it('is a LanguageModelV4 middleware and names the semantic kinds', () => {
    const mw = aischemagateGuardrail({
      apiKey: 'cg_live_test',
      contractId: CONTRACT,
      client: clientWith(() => jsonResponse(500, { error: 'unused' })),
    });
    assert.equal(mw.specificationVersion, 'v4');
    assert.equal(typeof mw.wrapGenerate, 'function');
    assert.deepEqual(SEMANTIC_VIOLATION_KINDS, [
      'range_violation',
      'pattern_mismatch',
      'enum_violation',
      'undeclared_field',
    ]);
    assert.equal(AISG_MAX_REPAIR_ATTEMPTS, 5);
  });

  it('posts generated JSON to egress and returns the gateway payload', async () => {
    let seen: { url: string; body: unknown; key: string | null } | undefined;
    const client = clientWith(async (req) => {
      seen = {
        url: req.url,
        body: await req.json(),
        key: req.headers.get('x-api-key'),
      };
      return jsonResponse(200, passBody({ total: 12.5, currency: 'USD', email: '****' }));
    });
    const mw = aischemagateGuardrail({ apiKey: 'cg_live_test', contractId: CONTRACT, client });
    const result = await mw.wrapGenerate({
      doGenerate: async () => textResult('```json\n{"total":12.5,"currency":"USD","email":"a@b.co"}\n```'),
      doStream: async () => {
        throw new Error('stream unused');
      },
      params: { prompt: [] },
      model: {
        doGenerate: () => {
          throw new Error('repair unused');
        },
      },
    });
    assert.equal(seen?.key, 'cg_live_test');
    assert.equal(new URL(seen!.url).pathname, `/egress/${CONTRACT}`);
    assert.deepEqual(seen?.body, [{ total: 12.5, currency: 'USD', email: 'a@b.co' }]);
    const text = (result.content[0] as { text: string }).text;
    assert.deepEqual(JSON.parse(text), { total: 12.5, currency: 'USD', email: '****' });
  });

  it('re-prompts once when an enum and a minimum fail, then returns the repair', async () => {
    const bodies = [
      failBody([
        { field: 'currency', kind: 'enum_violation', message: 'value usd not in allowed set' },
        { field: 'total', kind: 'range_violation', message: 'value -5 is below minimum 0' },
      ]),
      passBody({ currency: 'USD', total: 10 }),
    ];
    let calls = 0;
    const client = clientWith(async () => {
      const body = bodies[calls];
      calls += 1;
      return jsonResponse(calls === 1 ? 422 : 200, body);
    });
    const prompts: unknown[] = [];
    let modelCalls = 0;
    const mw = aischemagateGuardrail({
      apiKey: 'cg_live_test',
      contractId: CONTRACT,
      client,
      autoRepair: true,
      maxRetries: 2,
    });
    const result = await mw.wrapGenerate({
      doGenerate: async () => textResult('{"currency":"usd","total":-5}'),
      doStream: async () => {
        throw new Error('stream unused');
      },
      params: {
        prompt: [{ role: 'user', content: [{ type: 'text', text: 'Extract the invoice.' }] }],
      },
      model: {
        async doGenerate(params: LanguageModelCallParams) {
          modelCalls += 1;
          prompts.push(params.prompt);
          return textResult('{"currency":"USD","total":10}');
        },
      },
    });
    assert.equal(calls, 2);
    assert.equal(modelCalls, 1);
    const prompt = prompts[0] as Array<{ content: Array<{ text: string }> }>;
    assert.equal(prompt.length, 2);
    assert.match(prompt[1]!.content[0]!.text, /currency/);
    assert.match(prompt[1]!.content[0]!.text, /enum_violation/);
    assert.match(prompt[1]!.content[0]!.text, /range_violation/);
    assert.deepEqual(JSON.parse((result.content[0] as { text: string }).text), {
      currency: 'USD',
      total: 10,
    });
  });

  it('does not re-prompt for a structural type mismatch', async () => {
    let egressCalls = 0;
    const client = clientWith(() => {
      egressCalls += 1;
      return jsonResponse(
        422,
        failBody([{ field: 'total', kind: 'type_mismatch', message: 'expected number, got string' }]),
      );
    });
    const mw = aischemagateGuardrail({
      apiKey: 'cg_live_test',
      contractId: CONTRACT,
      client,
      autoRepair: true,
    });
    await assert.rejects(
      () =>
        mw.wrapGenerate({
          doGenerate: async () => textResult('{"total":"10"}'),
          doStream: async () => {
            throw new Error('stream unused');
          },
          params: { prompt: [{ role: 'user', content: 'extract' }] },
          model: {
            doGenerate: () => {
              throw new Error('structural failures are not repaired');
            },
          },
        }),
      (err: unknown) => {
        assert.ok(err instanceof AiSchemaGateRejectedError);
        assert.equal(err.attempts, 1);
        assert.equal(err.violations[0]?.kind, 'type_mismatch');
        return true;
      },
    );
    assert.equal(egressCalls, 1);
  });

  it('stops after the capped number of semantic repair attempts', async () => {
    let egressCalls = 0;
    const client = clientWith(() => {
      egressCalls += 1;
      return jsonResponse(
        422,
        failBody([{ field: 'status', kind: 'enum_violation', message: 'not in enum' }]),
      );
    });
    let modelCalls = 0;
    const mw = aischemagateGuardrail({
      apiKey: 'cg_live_test',
      contractId: CONTRACT,
      client,
      autoRepair: true,
      maxRetries: 99,
    });
    await assert.rejects(
      () =>
        mw.wrapGenerate({
          doGenerate: async () => textResult('{"status":"nope"}'),
          doStream: async () => {
            throw new Error('stream unused');
          },
          params: { prompt: [{ role: 'user', content: [{ type: 'text', text: 'go' }] }] },
          model: {
            async doGenerate() {
              modelCalls += 1;
              return textResult('{"status":"still-nope"}');
            },
          },
        }),
      (err: unknown) => err instanceof AiSchemaGateRejectedError && err.attempts === AISG_MAX_REPAIR_ATTEMPTS + 1,
    );
    assert.equal(egressCalls, AISG_MAX_REPAIR_ATTEMPTS + 1);
    assert.equal(modelCalls, AISG_MAX_REPAIR_ATTEMPTS);
  });

  it('rejects malformed JSON without calling egress or starting a repair', async () => {
    let egressCalls = 0;
    const client = clientWith(() => {
      egressCalls += 1;
      return jsonResponse(500, { error: 'egress must not run' });
    });
    let modelCalls = 0;
    const mw = aischemagateGuardrail({
      apiKey: 'cg_live_test',
      contractId: CONTRACT,
      client,
      autoRepair: true,
      maxRetries: 2,
    });
    await assert.rejects(
      () =>
        mw.wrapGenerate({
          doGenerate: async () => textResult('{not json'),
          doStream: async () => {
            throw new Error('stream unused');
          },
          params: { prompt: [{ role: 'user', content: [{ type: 'text', text: 'extract' }] }] },
          model: {
            async doGenerate() {
              modelCalls += 1;
              return textResult('{"currency":"USD"}');
            },
          },
        }),
      (err: unknown) => {
        assert.ok(err instanceof AiSchemaGateRejectedError);
        assert.match(err.message, /JSON object/);
        assert.equal(err.attempts, 1);
        assert.deepEqual(err.violations, []);
        return true;
      },
    );
    assert.equal(egressCalls, 0);
    assert.equal(modelCalls, 0);
  });

  it('rejects a JSON array and a malformed repair response', async () => {
    let egressCalls = 0;
    const client = clientWith(() => {
      egressCalls += 1;
      return jsonResponse(
        422,
        failBody([{ field: 'currency', kind: 'enum_violation', message: 'not in enum' }]),
      );
    });
    const mw = aischemagateGuardrail({
      apiKey: 'cg_live_test',
      contractId: CONTRACT,
      client,
      autoRepair: true,
      maxRetries: 2,
    });
    await assert.rejects(
      () =>
        mw.wrapGenerate({
          doGenerate: async () => textResult('["USD"]'),
          doStream: async () => {
            throw new Error('stream unused');
          },
          params: { prompt: [] },
          model: { doGenerate: () => Promise.reject(new Error('unused')) },
        }),
      (err: unknown) => err instanceof AiSchemaGateRejectedError && /JSON object/.test(err.message),
    );
    assert.equal(egressCalls, 0);

    let repairs = 0;
    const repairing = aischemagateGuardrail({
      apiKey: 'cg_live_test',
      contractId: CONTRACT,
      client,
      autoRepair: true,
      maxRetries: 2,
    });
    await assert.rejects(
      () =>
        repairing.wrapGenerate({
          doGenerate: async () => textResult('{"currency":"usd"}'),
          doStream: async () => {
            throw new Error('stream unused');
          },
          params: { prompt: [{ role: 'user', content: [{ type: 'text', text: 'extract' }] }] },
          model: {
            async doGenerate() {
              repairs += 1;
              return textResult('still not json');
            },
          },
        }),
      (err: unknown) =>
        err instanceof AiSchemaGateRejectedError && /repair response was not a JSON object/.test(err.message),
    );
    assert.equal(egressCalls, 1);
    assert.equal(repairs, 1);
  });

  it('validates tool-call input when the text is not JSON', async () => {
    const client = clientWith(async (req) => {
      assert.deepEqual(await req.json(), [{ city: 'Paris' }]);
      return jsonResponse(200, passBody({ city: 'Paris', compliance_mode: true }));
    });
    const mw = aischemagateGuardrail({ apiKey: 'cg_live_test', contractId: CONTRACT, client });
    const result = await mw.wrapGenerate({
      doGenerate: async () => ({
        content: [
          { type: 'text', text: 'booking' },
          { type: 'tool-call', toolName: 'book', input: '{"city":"Paris"}' },
        ],
      }),
      doStream: async () => {
        throw new Error('stream unused');
      },
      params: { prompt: [] },
      model: { doGenerate: () => Promise.reject(new Error('unused')) },
    });
    const tool = result.content[1] as { input: string };
    assert.deepEqual(JSON.parse(tool.input), { city: 'Paris', compliance_mode: true });
  });
});

describe('validateStructuredOutput', () => {
  it('repairs semantic violations through the caller callback and returns the egress payload', async () => {
    const bodies = [
      failBody([
        { field: 'code', kind: 'pattern_mismatch', message: 'must match ^[A-Z]{2}$' },
        { field: 'note', kind: 'undeclared_field', message: 'compliance_mode rejects note' },
      ]),
      passBody({ code: 'US' }),
    ];
    let calls = 0;
    const client = clientWith(() => {
      const body = bodies[calls];
      calls += 1;
      return jsonResponse(calls === 1 ? 422 : 200, body);
    });
    let repairs = 0;
    const outcome = await validateStructuredOutput(
      { apiKey: 'cg_live_test', contractId: CONTRACT, client, autoRepair: true, maxRetries: 1 },
      { code: 'usa', note: 'extra' },
      async () => {
        repairs += 1;
        return { code: 'US' };
      },
    );
    assert.equal(repairs, 1);
    assert.equal(outcome.ok, true);
    assert.equal(outcome.attempts, 2);
    assert.deepEqual(outcome.payload, { code: 'US' });
  });

  it('returns a failed outcome without throwing when throwIfRejected is unset', async () => {
    const client = clientWith(() =>
      jsonResponse(422, failBody([{ field: 'n', kind: 'range_violation', message: 'min 0' }])),
    );
    const outcome = await validateStructuredOutput(
      { apiKey: 'k', contractId: CONTRACT, client, autoRepair: true },
      { n: -1 },
    );
    assert.equal(outcome.ok, false);
    assert.equal(outcome.attempts, 1);
    assert.equal(outcome.violations[0]?.kind, 'range_violation');
  });
});
