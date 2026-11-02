import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { Client, AuthError, BadRequestError, NotFoundError, PlanRequiredError, ServerError, ValidationFailedError } from '../src/index.js';

function mockFetch(status: number, body: unknown): typeof globalThis.fetch {
  return async (_url, _opts) => {
    return new Response(JSON.stringify(body), {
      status,
      headers: { 'Content-Type': 'application/json' },
    });
  };
}

const CONTRACT_ID = '00000000-0000-0000-0000-000000000001';

function client(status: number, body: unknown) {
  return new Client({
    baseUrl: 'http://localhost:8080',
    apiKey: 'cg_test_key',
    fetch: mockFetch(status, body),
  });
}

// ---------------------------------------------------------------------------
// Ingest
// ---------------------------------------------------------------------------

describe('Client', () => {
  it('ingest — passes response through correctly', async () => {
    const mockBody = {
      total: 1, passed: 1, failed: 0, dry_run: false, atomic: false,
      resolved_version: '1.0.0', version_pin_source: 'default',
      results: [{
        passed: true, violations: [], validation_us: 100,
        forwarded: true, contract_version: '1.0.0', transformed_event: null,
      }],
    };
    const c = client(200, mockBody);
    const r = await c.ingest({ contractId: CONTRACT_ID, events: [{ user_id: 'x' }] });
    assert.equal(r.total, 1);
    assert.equal(r.passed, 1);
    assert.equal(r.results.length, 1);
    assert.ok(r.results[0].passed);
  });

  it('ingest — sends x-api-key header', async () => {
    let capturedHeaders: Record<string, string> = {};
    const c = new Client({
      baseUrl: 'http://localhost:8080',
      apiKey: 'cg_test_key',
      fetch: async (_url, opts) => {
        capturedHeaders = opts?.headers as Record<string, string> ?? {};
        return new Response(JSON.stringify({
          total: 0, passed: 0, failed: 0, dry_run: false, atomic: false,
          resolved_version: '1', version_pin_source: 'default', results: [],
        }), { status: 200 });
      },
    });
    await c.ingest({ contractId: CONTRACT_ID, events: [] });
    assert.equal(capturedHeaders['x-api-key'], 'cg_test_key');
  });

  it('ingest — posts to /v1/ingest with version as a query param', async () => {
    const urls: URL[] = [];
    const c = new Client({
      baseUrl: 'http://localhost:8080',
      apiKey: 'cg_key',
      fetch: async (url) => {
        urls.push(new URL(String(url)));
        return new Response(JSON.stringify({
          total: 0, passed: 0, failed: 0, dry_run: false, atomic: false,
          resolved_version: '2.0.0', version_pin_source: 'query_param', results: [],
        }), { status: 200 });
      },
    });
    await c.ingest({ contractId: CONTRACT_ID, events: [], version: '2.0.0' });
    await c.ingest({ contractId: `${CONTRACT_ID}@1.2.0`, events: [] });
    assert.equal(urls[0].pathname, `/v1/ingest/${CONTRACT_ID}`);
    assert.equal(urls[0].searchParams.get('version'), '2.0.0');
    assert.equal(urls[1].pathname, `/v1/ingest/${CONTRACT_ID}`);
    assert.equal(urls[1].searchParams.get('version'), '1.2.0');
  });

  it('ingest — all-failed 422 resolves with per-event results', async () => {
    const c = client(422, {
      total: 1, passed: 0, failed: 1, dry_run: false, atomic: false,
      resolved_version: '1.0.0', version_pin_source: 'default_stable',
      results: [{
        index: 0, passed: false, validation_us: 9, forwarded: false,
        contract_version: '1.0.0', quarantine_id: 'q-1', transformed_event: {},
        violations: [{ field: 'amount', message: 'below min', kind: 'range_violation' }],
      }],
    });
    const r = await c.ingest({ contractId: CONTRACT_ID, events: [{}] });
    assert.equal(r.failed, 1);
    assert.equal(r.results[0].index, 0);
    assert.equal(r.results[0].quarantine_id, 'q-1');
    assert.equal(r.results[0].violations[0].field, 'amount');
  });

  it('ingest — a 422 without results still throws', async () => {
    const c = client(422, { error: 'idempotency key reused with a different body', status: 422 });
    await assert.rejects(() => c.ingest({ contractId: CONTRACT_ID, events: [] }), ValidationFailedError);
  });

  // ── Error mapping ────────────────────────────────────────────────────────

  it('maps 401 → AuthError', async () => {
    const c = client(401, { error: 'Invalid API key' });
    await assert.rejects(
      () => c.ingest({ contractId: CONTRACT_ID, events: [] }),
      (e) => e instanceof AuthError,
    );
  });

  it('maps 400 → BadRequestError', async () => {
    const c = client(400, { error: 'bad input' });
    await assert.rejects(
      () => c.ingest({ contractId: CONTRACT_ID, events: [] }),
      (e) => e instanceof BadRequestError,
    );
  });

  it('maps 404 → NotFoundError', async () => {
    const c = client(404, { error: 'not found' });
    await assert.rejects(
      () => c.getContract(CONTRACT_ID),
      (e) => e instanceof NotFoundError,
    );
  });

  it('maps 500 → ServerError', async () => {
    const c = client(500, { error: 'internal' });
    await assert.rejects(
      () => c.stats(),
      (e) => e instanceof ServerError,
    );
  });

  // ── List / get ───────────────────────────────────────────────────────────

  it('listContracts — returns array', async () => {
    const c = client(200, [
      { id: 'abc', name: 'test', description: null, multi_stable_resolution: 'strict',
        created_at: 'x', updated_at: 'x', version_count: 1, latest_stable_version: null },
    ]);
    const contracts = await c.listContracts();
    assert.equal(contracts.length, 1);
    assert.equal(contracts[0].id, 'abc');
  });

  it('audit — forwards limit/offset as query params', async () => {
    let capturedUrl = '';
    const c = new Client({
      baseUrl: 'http://localhost:8080',
      fetch: async (url, _opts) => {
        capturedUrl = url as string;
        return new Response(JSON.stringify([]), { status: 200 });
      },
    });
    await c.audit({ limit: 10, offset: 20 });
    assert.ok(capturedUrl.includes('limit=10'), capturedUrl);
    assert.ok(capturedUrl.includes('offset=20'), capturedUrl);
  });

  it('ingest — plan_required is PlanRequiredError', async () => {
    const c = client(403, {
      error: 'plan_required',
      message: 'Replay is included with Pro.',
      feature: 'quarantine_replay',
    });
    await assert.rejects(() => c.ingest({ contractId: CONTRACT_ID, events: [{}] }), (err: unknown) => {
      assert.ok(err instanceof PlanRequiredError);
      assert.match((err as Error).message, /included with Pro/);
      return true;
    });
  });
});
