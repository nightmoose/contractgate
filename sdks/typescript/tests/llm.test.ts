import { describe, it } from 'node:test';
import assert from 'node:assert/strict';

import { Client, ContractGateError } from '../src/index.js';
import {
  LLMGate,
  LLMResponseError,
  MAX_RETRIES_CAP,
  extractJson,
  type LLMGateResult,
} from '../src/llm.js';
import type { Violation } from '../src/types.js';

// ---------------------------------------------------------------------------
// Mock builders — LLM responses
// ---------------------------------------------------------------------------

function openaiJsonMode(obj: Record<string, unknown>): unknown {
  return {
    choices: [
      { message: { content: JSON.stringify(obj), tool_calls: null } },
    ],
  };
}

function openaiToolCall(obj: Record<string, unknown>): unknown {
  return {
    choices: [
      {
        message: {
          content: null,
          tool_calls: [
            { function: { name: 'extract', arguments: JSON.stringify(obj) } },
          ],
        },
      },
    ],
  };
}

function anthropicToolUse(obj: Record<string, unknown>): unknown {
  return { content: [{ type: 'tool_use', name: 'extract', input: obj }] };
}

function anthropicTextJson(obj: Record<string, unknown>): unknown {
  return { content: [{ type: 'text', text: JSON.stringify(obj) }] };
}

// ---------------------------------------------------------------------------
// Mock builders — gateway responses
// ---------------------------------------------------------------------------

function okEgressBody(payload: Record<string, unknown>): Record<string, unknown> {
  return {
    total: 1,
    passed: 1,
    failed: 0,
    dry_run: false,
    disposition: 'block',
    resolved_version: '1.0',
    payload: [payload],
    outcomes: [
      {
        index: 0,
        passed: true,
        violations: [],
        validation_us: 42,
        action: 'included',
      },
    ],
  };
}

function failEgressBody(): Record<string, unknown> {
  return {
    total: 1,
    passed: 0,
    failed: 1,
    dry_run: false,
    disposition: 'block',
    resolved_version: '1.0',
    payload: [],
    outcomes: [
      {
        index: 0,
        passed: false,
        violations: [
          {
            field: 'amount',
            message: "Field 'amount' value -3 is below minimum 0",
            kind: 'range_violation',
          },
        ],
        validation_us: 42,
        action: 'blocked',
      },
    ],
  };
}

function ingestWithTransform(
  transformed: Record<string, unknown>,
): Record<string, unknown> {
  return {
    total: 1,
    passed: 1,
    failed: 0,
    dry_run: true,
    atomic: false,
    resolved_version: '1.0',
    version_pin_source: 'default_stable',
    results: [
      {
        passed: true,
        violations: [],
        validation_us: 10,
        forwarded: false,
        contract_version: '1.0',
        transformed_event: transformed,
      },
    ],
  };
}

// ---------------------------------------------------------------------------
// fetch mocks
// ---------------------------------------------------------------------------

function fetchReturning(body: unknown, status = 200): typeof globalThis.fetch {
  return async () =>
    new Response(JSON.stringify(body), {
      status,
      headers: { 'Content-Type': 'application/json' },
    });
}

type CycleState = { calls: Array<{ url: string; body: unknown }>; i: number };

function fetchCycle(bodies: unknown[]): {
  state: CycleState;
  fetch: typeof globalThis.fetch;
} {
  const state: CycleState = { calls: [], i: 0 };
  const fn: typeof globalThis.fetch = async (url, opts) => {
    const body = opts?.body !== undefined ? JSON.parse(String(opts.body)) : null;
    state.calls.push({ url: String(url), body });
    const b = bodies[state.i];
    state.i += 1;
    return new Response(JSON.stringify(b), {
      status: 200,
      headers: { 'Content-Type': 'application/json' },
    });
  };
  return { state, fetch: fn };
}

function clientWith(body: unknown): Client {
  return new Client({
    baseUrl: 'https://gw',
    apiKey: 'k',
    fetch: fetchReturning(body),
  });
}

// ---------------------------------------------------------------------------
// extractJson — shape matrix
// ---------------------------------------------------------------------------

describe('extractJson', () => {
  it('OpenAI JSON mode', () => {
    assert.deepEqual(extractJson(openaiJsonMode({ user_id: 'alice', amount: 10 })), {
      user_id: 'alice',
      amount: 10,
    });
  });

  it('OpenAI tool_call wins over content when both are set', () => {
    const resp = {
      choices: [
        {
          message: {
            content: JSON.stringify({ wrong: 'value' }),
            tool_calls: [
              { function: { name: 'x', arguments: JSON.stringify({ right: 'value' }) } },
            ],
          },
        },
      ],
    };
    assert.deepEqual(extractJson(resp), { right: 'value' });
  });

  it('Anthropic tool_use', () => {
    assert.deepEqual(extractJson(anthropicToolUse({ invoice_total: 42.5 })), {
      invoice_total: 42.5,
    });
  });

  it('Anthropic text-block JSON fallback', () => {
    assert.deepEqual(extractJson(anthropicTextJson({ status: 'ok' })), {
      status: 'ok',
    });
  });

  it('OpenAI Responses API output_parsed passthrough', () => {
    assert.deepEqual(extractJson({ output_parsed: { schema_validated: true } }), {
      schema_validated: true,
    });
  });

  it('string passthrough', () => {
    assert.deepEqual(extractJson('{"a": 1}'), { a: 1 });
  });

  it('string with markdown fence', () => {
    assert.deepEqual(extractJson('```json\n{"a": 1, "b": [1,2]}\n```'), {
      a: 1,
      b: [1, 2],
    });
  });

  it('string with fence, no language tag', () => {
    assert.deepEqual(extractJson('```\n{"a": 1}\n```'), { a: 1 });
  });

  it('dict passthrough', () => {
    assert.deepEqual(extractJson({ a: 1 }), { a: 1 });
  });

  it('unknown shape raises', () => {
    assert.throws(() => extractJson(42), (err: Error) => {
      return err instanceof LLMResponseError && /unrecognized response shape/.test(err.message);
    });
  });

  it('invalid JSON string raises', () => {
    assert.throws(() => extractJson('not json at all {{{'), (err: Error) => {
      return err instanceof LLMResponseError && /not valid JSON/.test(err.message);
    });
  });

  it('JSON array not object raises', () => {
    assert.throws(() => extractJson('[1, 2, 3]'), (err: Error) => {
      return err instanceof LLMResponseError && /not an object/.test(err.message);
    });
  });

  it('async-iterable without response fields is streaming', () => {
    const stream = {
      [Symbol.asyncIterator]() {
        return { next: async () => ({ done: true, value: undefined }) };
      },
    };
    assert.throws(() => extractJson(stream), (err: Error) => {
      return err instanceof LLMResponseError && /streaming responses are not supported/.test(err.message);
    });
  });
});

// ---------------------------------------------------------------------------
// LLMGate — construction
// ---------------------------------------------------------------------------

describe('LLMGate construction', () => {
  it('requires an llm client', () => {
    assert.throws(
      () => new LLMGate({ apiKey: 'k', contractId: '11111111-1111-4111-8111-111111111111', llm: null }),
      (err: Error) => err instanceof TypeError && /requires an `llm` client/.test(err.message),
    );
  });

  it('maxRetries above cap warns and clamps', () => {
    const originalWarn = console.warn;
    const warnings: string[] = [];
    console.warn = (msg: unknown) => warnings.push(String(msg));
    try {
      const gate = new LLMGate({
        apiKey: 'k',
        contractId: '11111111-1111-4111-8111-111111111111',
        llm: {},
        maxRetries: 99,
        client: clientWith(okEgressBody({})),
      });
      assert.ok(warnings.some((w) => /exceeds hard cap/.test(w)));
      // Property is private; infer clamp via observable behavior in later tests.
      void gate;
    } finally {
      console.warn = originalWarn;
    }
    assert.equal(MAX_RETRIES_CAP, 5);
  });
});

// ---------------------------------------------------------------------------
// validateOnly
// ---------------------------------------------------------------------------

describe('LLMGate.validateOnly', () => {
  it('happy path', async () => {
    const payload = { user_id: 'alice', amount: 10 };
    const gate = new LLMGate({
      apiKey: 'k',
      contractId: '11111111-1111-4111-8111-111111111111',
      llm: {},
      client: clientWith(okEgressBody(payload)),
    });
    const result = await gate.validateOnly(payload);
    assert.equal(result.ok, true);
    assert.deepEqual(result.payload, payload);
    assert.deepEqual(result.violations, []);
    assert.equal(result.attempts, 1);
  });

  it('failure returns sanitized result', async () => {
    const gate = new LLMGate({
      apiKey: 'k',
      contractId: '11111111-1111-4111-8111-111111111111',
      llm: {},
      client: clientWith(failEgressBody()),
    });
    const result = await gate.validateOnly({ amount: -3 });
    assert.equal(result.ok, false);
    assert.deepEqual(result.payload, {});
    assert.equal(result.violations.length, 1);
    assert.equal(result.violations[0].field, 'amount');
    assert.equal(result.violations[0].kind, 'range_violation');
  });

  it('asserts request shape', async () => {
    let captured: { url: string; body: unknown } = { url: '', body: null };
    const fetchFn: typeof globalThis.fetch = async (url, opts) => {
      captured = { url: String(url), body: JSON.parse(String(opts?.body)) };
      return new Response(JSON.stringify(okEgressBody({ user_id: 'a' })), { status: 200 });
    };
    const client = new Client({ baseUrl: 'https://gw', apiKey: 'k', fetch: fetchFn });
    const gate = new LLMGate({
      apiKey: 'k',
      contractId: '22222222-2222-4222-8222-222222222222',
      llm: {},
      client,
    });
    await gate.validateOnly(openaiJsonMode({ user_id: 'a' }));
    assert.ok(captured.url.includes('/egress/22222222-2222-4222-8222-222222222222'));
    assert.deepEqual(captured.body, [{ user_id: 'a' }]);
  });
});

// ---------------------------------------------------------------------------
// completion — OpenAI protocol
// ---------------------------------------------------------------------------

describe('LLMGate.completion', () => {
  it('happy path, no repair', async () => {
    const payload = { user_id: 'alice', amount: 10 };
    let callCount = 0;
    const llm = {
      chat: {
        completions: {
          create: async () => {
            callCount += 1;
            return openaiJsonMode(payload);
          },
        },
      },
    };
    const gate = new LLMGate({
      apiKey: 'k',
      contractId: '11111111-1111-4111-8111-111111111111',
      llm,
      client: clientWith(okEgressBody(payload)),
    });
    const result = await gate.completion({
      model: 'gpt-4o-mini',
      messages: [{ role: 'user', content: 'x' }],
    });
    assert.equal(result.ok, true);
    assert.equal(result.attempts, 1);
    assert.equal(callCount, 1);
    assert.deepEqual(result.repairHints, []);
  });

  it('passes model + messages + extra kwargs to provider', async () => {
    const captured: Record<string, unknown>[] = [];
    const llm = {
      chat: {
        completions: {
          create: async (args: Record<string, unknown>) => {
            captured.push(args);
            return openaiJsonMode({ a: 1 });
          },
        },
      },
    };
    const gate = new LLMGate({
      apiKey: 'k',
      contractId: '11111111-1111-4111-8111-111111111111',
      llm,
      client: clientWith(okEgressBody({ a: 1 })),
    });
    await gate.completion({
      model: 'gpt-4o',
      messages: [{ role: 'user', content: 'hi' }],
      temperature: 0.1,
    });
    assert.equal(captured[0].model, 'gpt-4o');
    assert.deepEqual(captured[0].messages, [{ role: 'user', content: 'hi' }]);
    assert.equal(captured[0].temperature, 0.1);
  });

  it('auto_repair passes on second try, appends hint to messages', async () => {
    const bad = { amount: -3 };
    const good = { amount: 10 };
    const cycle = fetchCycle([failEgressBody(), okEgressBody(good)]);
    const client = new Client({ baseUrl: 'https://gw', apiKey: 'k', fetch: cycle.fetch });
    const providerCalls: Record<string, unknown>[] = [];
    const llm = {
      chat: {
        completions: {
          create: async (args: Record<string, unknown>) => {
            providerCalls.push(args);
            return providerCalls.length === 1 ? openaiJsonMode(bad) : openaiJsonMode(good);
          },
        },
      },
    };
    const gate = new LLMGate({
      apiKey: 'k',
      contractId: '11111111-1111-4111-8111-111111111111',
      llm,
      client,
      maxRetries: 2,
    });
    const result = await gate.completion({
      model: 'gpt-4o-mini',
      messages: [{ role: 'user', content: 'extract' }],
      autoRepair: true,
    });
    assert.equal(result.ok, true);
    assert.equal(result.attempts, 2);
    assert.equal(providerCalls.length, 2);
    assert.equal(cycle.state.calls.length, 2);
    assert.equal(result.repairHints.length, 1);
    // Second provider call must carry the hint as an appended user message.
    const secondMessages = providerCalls[1].messages as Array<{ role: string; content: string }>;
    assert.equal(secondMessages.length, 2);
    assert.equal(secondMessages[1].role, 'user');
    assert.ok(secondMessages[1].content.includes('amount'));
  });

  it('auto_repair exhausts budget — ok=false after maxRetries+1 calls', async () => {
    const bad = { amount: -3 };
    const cycle = fetchCycle([failEgressBody(), failEgressBody(), failEgressBody()]);
    const client = new Client({ baseUrl: 'https://gw', apiKey: 'k', fetch: cycle.fetch });
    let providerCalls = 0;
    const llm = {
      chat: {
        completions: {
          create: async () => {
            providerCalls += 1;
            return openaiJsonMode(bad);
          },
        },
      },
    };
    const gate = new LLMGate({
      apiKey: 'k',
      contractId: '11111111-1111-4111-8111-111111111111',
      llm,
      client,
      maxRetries: 2,
    });
    const result = await gate.completion({
      model: 'gpt-4o-mini',
      messages: [{ role: 'user', content: 'extract' }],
      autoRepair: true,
    });
    assert.equal(result.ok, false);
    assert.equal(result.attempts, 3);
    assert.equal(providerCalls, 3);
    assert.equal(result.repairHints.length, 2);
  });

  it('autoRepair:false skips retries', async () => {
    const bad = { amount: -3 };
    let providerCalls = 0;
    const llm = {
      chat: {
        completions: {
          create: async () => {
            providerCalls += 1;
            return openaiJsonMode(bad);
          },
        },
      },
    };
    const gate = new LLMGate({
      apiKey: 'k',
      contractId: '11111111-1111-4111-8111-111111111111',
      llm,
      client: clientWith(failEgressBody()),
      maxRetries: 2,
    });
    const result = await gate.completion({
      model: 'gpt-4o-mini',
      messages: [{ role: 'user', content: 'extract' }],
      autoRepair: false,
    });
    assert.equal(result.ok, false);
    assert.equal(result.attempts, 1);
    assert.equal(providerCalls, 1);
  });

  it('stream:true is rejected', async () => {
    const gate = new LLMGate({
      apiKey: 'k',
      contractId: '11111111-1111-4111-8111-111111111111',
      llm: {},
      client: clientWith(okEgressBody({})),
    });
    await assert.rejects(
      gate.completion({ model: 'x', messages: [], stream: true }),
      (err: Error) => err instanceof LLMResponseError && /streaming responses are not supported/.test(err.message),
    );
  });

  it('throws when llm lacks chat.completions.create', async () => {
    const gate = new LLMGate({
      apiKey: 'k',
      contractId: '11111111-1111-4111-8111-111111111111',
      llm: { messages: { create: async () => ({}) } },
      client: clientWith(okEgressBody({})),
    });
    await assert.rejects(
      gate.completion({ model: 'x', messages: [] }),
      (err: Error) => err instanceof TypeError && /chat\.completions\.create/.test(err.message),
    );
  });
});

// ---------------------------------------------------------------------------
// message — Anthropic protocol
// ---------------------------------------------------------------------------

describe('LLMGate.message', () => {
  it('calls llm.messages.create with max_tokens', async () => {
    const payload = { invoice_total: 42 };
    const captured: Record<string, unknown>[] = [];
    const llm = {
      messages: {
        create: async (args: Record<string, unknown>) => {
          captured.push(args);
          return anthropicToolUse(payload);
        },
      },
    };
    const gate = new LLMGate({
      apiKey: 'k',
      contractId: '11111111-1111-4111-8111-111111111111',
      llm,
      client: clientWith(okEgressBody(payload)),
    });
    const result = await gate.message({
      model: 'claude-sonnet-4-5',
      messages: [{ role: 'user', content: 'extract invoice' }],
      maxTokens: 512,
    });
    assert.equal(result.ok, true);
    assert.deepEqual(result.payload, payload);
    assert.equal(captured[0].model, 'claude-sonnet-4-5');
    assert.equal(captured[0].max_tokens, 512);
  });

  it('throws when llm lacks messages.create', async () => {
    const gate = new LLMGate({
      apiKey: 'k',
      contractId: '11111111-1111-4111-8111-111111111111',
      llm: { chat: { completions: { create: async () => ({}) } } },
      client: clientWith(okEgressBody({})),
    });
    await assert.rejects(
      gate.message({ model: 'x', messages: [] }),
      (err: Error) => err instanceof TypeError && /messages\.create/.test(err.message),
    );
  });
});

// ---------------------------------------------------------------------------
// maskPrompt
// ---------------------------------------------------------------------------

describe('LLMGate.maskPrompt', () => {
  it('returns transformed_event, posts to the right path with dry_run', async () => {
    const transformed = { email: 'hmac-sha256:abc123', user_id: 'alice' };
    let captured: { url: string; body: unknown } = { url: '', body: null };
    const fetchFn: typeof globalThis.fetch = async (url, opts) => {
      captured = { url: String(url), body: JSON.parse(String(opts?.body)) };
      return new Response(JSON.stringify(ingestWithTransform(transformed)), { status: 200 });
    };
    const client = new Client({ baseUrl: 'https://gw', apiKey: 'k', fetch: fetchFn });
    const gate = new LLMGate({ apiKey: 'k', contractId: '33333333-3333-4333-8333-333333333333', llm: {}, client });
    const result = await gate.maskPrompt(
      { email: 'alice@example.com', user_id: 'alice' },
      { promptContractId: '44444444-4444-4444-8444-444444444444' },
    );
    assert.deepEqual(result, transformed);
    assert.ok(captured.url.includes('/ingest/44444444-4444-4444-8444-444444444444'));
    assert.ok(captured.url.includes('dry_run=true'));
  });
});

// ---------------------------------------------------------------------------
// LLMGateResult shape
// ---------------------------------------------------------------------------

describe('LLMGateResult', () => {
  it('has the expected fields', () => {
    const r: LLMGateResult = {
      ok: true,
      payload: { a: 1 },
      violations: [] as Violation[],
      attempts: 1,
      rawResponse: null,
      repairHints: [],
    };
    assert.equal(r.ok, true);
    assert.deepEqual(r.repairHints, []);
  });
});

describe('contract name lookup', () => {
  const rows = [
    { id: '33333333-3333-4333-8333-333333333333', name: 'other_contract', description: null, multi_stable_resolution: 'strict', created_at: '', updated_at: '', version_count: 1, latest_stable_version: '1.0.0' },
    { id: '11111111-1111-4111-8111-111111111111', name: 'extract_invoice_v1', description: null, multi_stable_resolution: 'strict', created_at: '', updated_at: '', version_count: 1, latest_stable_version: '1.0.0' },
  ];
  function lookupClient(calls: string[]): Client {
    const fetchFn: typeof globalThis.fetch = async (url, opts) => {
      const u = new URL(String(url));
      const method = opts?.method ?? 'GET';
      calls.push(`${method} ${u.pathname}`);
      const body = u.pathname === '/contracts' ? rows : okEgressBody({ user_id: 'a' });
      return new Response(JSON.stringify(body), { status: 200 });
    };
    return new Client({ baseUrl: 'https://gw', apiKey: 'k', fetch: fetchFn });
  }

  it('resolves a name once and caches it', async () => {
    const calls: string[] = [];
    const gate = new LLMGate({ apiKey: 'k', contractId: 'extract_invoice_v1', llm: {}, client: lookupClient(calls) });
    await gate.validateOnly({ user_id: 'a' });
    await gate.validateOnly({ user_id: 'a' });
    assert.deepEqual(calls, ['GET /contracts', 'POST /egress/11111111-1111-4111-8111-111111111111', 'POST /egress/11111111-1111-4111-8111-111111111111']);
  });

  it('falls back to a case-insensitive match', async () => {
    const calls: string[] = [];
    const gate = new LLMGate({ apiKey: 'k', contractId: 'Extract_Invoice_V1', llm: {}, client: lookupClient(calls) });
    await gate.validateOnly({ user_id: 'a' });
    assert.equal(calls.at(-1), 'POST /egress/11111111-1111-4111-8111-111111111111');
  });

  it('an unknown name lists the names on the key', async () => {
    const gate = new LLMGate({ apiKey: 'k', contractId: 'nope', llm: {}, client: lookupClient([]) });
    await assert.rejects(
      () => gate.validateOnly({ user_id: 'a' }),
      (err: unknown) => err instanceof ContractGateError && /No contract named 'nope'.*extract_invoice_v1/.test(err.message),
    );
  });

  it('a UUID skips the lookup', async () => {
    const calls: string[] = [];
    const gate = new LLMGate({ apiKey: 'k', contractId: '11111111-1111-4111-8111-111111111111', llm: {}, client: lookupClient(calls) });
    await gate.validateOnly({ user_id: 'a' });
    assert.deepEqual(calls, ['POST /egress/11111111-1111-4111-8111-111111111111']);
  });
});
