import { ConnectionError, raiseForStatus } from './errors.js';
import {
  auditEntryFromJson,
  batchIngestResponseFromJson,
  contractResponseFromJson,
  egressResponseFromJson,
  expectList,
  ingestionStatsFromJson,
  versionResponseFromJson,
  versionSummaryFromJson,
} from './types.js';
import type {
  AuditEntry,
  BatchIngestResponse,
  ContractResponse,
  EgressResponse,
  IngestionStats,
  VersionResponse,
  VersionSummary,
} from './types.js';

const DEFAULT_TIMEOUT_MS = 30_000;
const SDK_VERSION = '0.2.0';
const USER_AGENT = `contractgate-typescript/${SDK_VERSION}`;

// ---------------------------------------------------------------------------
// Options
// ---------------------------------------------------------------------------

export interface ClientOptions {
  baseUrl: string;
  apiKey?: string;
  orgId?: string;
  timeoutMs?: number;
  /** Override the global fetch (useful for tests). */
  fetch?: typeof globalThis.fetch;
}

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

export class Client {
  private readonly baseUrl: string;
  private readonly apiKey?: string;
  private readonly orgId?: string;
  private readonly timeoutMs: number;
  private readonly fetchFn: typeof globalThis.fetch;

  constructor(opts: ClientOptions) {
    this.baseUrl = opts.baseUrl.replace(/\/$/, '');
    this.apiKey = opts.apiKey;
    this.orgId = opts.orgId;
    this.timeoutMs = opts.timeoutMs ?? DEFAULT_TIMEOUT_MS;
    this.fetchFn = opts.fetch ?? globalThis.fetch;
  }

  // ── Ingest ────────────────────────────────────────────────────────────────

  async ingest(opts: {
    contractId: string;
    events: unknown;
    version?: string;
    dryRun?: boolean;
    atomic?: boolean;
    timeoutMs?: number;
  }): Promise<BatchIngestResponse> {
    const params: Record<string, string> = {};
    if (opts.dryRun) params['dry_run'] = 'true';
    if (opts.atomic) params['atomic'] = 'true';

    const extra: Record<string, string> = {};
    if (opts.version) extra['X-Contract-Version'] = opts.version;

    const body = await this.request({
      method: 'POST',
      path: `/ingest/${opts.contractId}`,
      params,
      body: opts.events,
      extraHeaders: extra,
      timeoutMs: opts.timeoutMs,
    });
    return batchIngestResponseFromJson(body);
  }

  // ── Egress ────────────────────────────────────────────────────────────────

  async egress(opts: {
    contractId: string;
    events: unknown;
    version?: string;
    disposition?: string;
    dryRun?: boolean;
    timeoutMs?: number;
  }): Promise<EgressResponse> {
    const params: Record<string, string> = {};
    if (opts.dryRun) params['dry_run'] = 'true';
    if (opts.disposition) params['disposition'] = opts.disposition;

    const extra: Record<string, string> = {};
    if (opts.version) extra['X-Contract-Version'] = opts.version;

    const body = await this.request({
      method: 'POST',
      path: `/egress/${opts.contractId}`,
      params,
      body: opts.events,
      extraHeaders: extra,
      timeoutMs: opts.timeoutMs,
    });
    return egressResponseFromJson(body);
  }

  // ── Audit ─────────────────────────────────────────────────────────────────

  async audit(opts: {
    contractId?: string;
    limit?: number;
    offset?: number;
    timeoutMs?: number;
  } = {}): Promise<AuditEntry[]> {
    const params: Record<string, string> = {
      limit: String(opts.limit ?? 50),
      offset: String(opts.offset ?? 0),
    };
    if (opts.contractId) params['contract_id'] = opts.contractId;

    const body = await this.request({ method: 'GET', path: '/audit', params, timeoutMs: opts.timeoutMs });
    return expectList(body).map(auditEntryFromJson);
  }

  async stats(opts: { timeoutMs?: number } = {}): Promise<IngestionStats> {
    const body = await this.request({ method: 'GET', path: '/stats', timeoutMs: opts.timeoutMs });
    return ingestionStatsFromJson(body);
  }

  // ── Contract reads ────────────────────────────────────────────────────────

  async getContract(contractId: string, opts: { timeoutMs?: number } = {}): Promise<ContractResponse> {
    const body = await this.request({
      method: 'GET',
      path: `/contracts/${contractId}`,
      timeoutMs: opts.timeoutMs,
    });
    return contractResponseFromJson(body);
  }

  async listContracts(opts: { timeoutMs?: number } = {}): Promise<ContractResponse[]> {
    const body = await this.request({ method: 'GET', path: '/contracts', timeoutMs: opts.timeoutMs });
    return expectList(body).map(contractResponseFromJson);
  }

  async listVersions(contractId: string, opts: { timeoutMs?: number } = {}): Promise<VersionSummary[]> {
    const body = await this.request({
      method: 'GET',
      path: `/contracts/${contractId}/versions`,
      timeoutMs: opts.timeoutMs,
    });
    return expectList(body).map(versionSummaryFromJson);
  }

  async getVersion(
    contractId: string,
    version: string,
    opts: { timeoutMs?: number } = {},
  ): Promise<VersionResponse> {
    const body = await this.request({
      method: 'GET',
      path: `/contracts/${contractId}/versions/${version}`,
      timeoutMs: opts.timeoutMs,
    });
    return versionResponseFromJson(body);
  }

  async getLatestStable(contractId: string, opts: { timeoutMs?: number } = {}): Promise<VersionResponse> {
    const body = await this.request({
      method: 'GET',
      path: `/contracts/${contractId}/versions/latest-stable`,
      timeoutMs: opts.timeoutMs,
    });
    return versionResponseFromJson(body);
  }

  // ── Playground ────────────────────────────────────────────────────────────

  async playgroundValidate(opts: {
    yamlContent: string;
    event: unknown;
    timeoutMs?: number;
  }): Promise<unknown> {
    return this.request({
      method: 'POST',
      path: '/playground/validate',
      body: { yaml_content: opts.yamlContent, event: opts.event },
      timeoutMs: opts.timeoutMs,
    });
  }

  // ── Internal ──────────────────────────────────────────────────────────────

  private buildUrl(path: string, params?: Record<string, string>): string {
    const url = new URL(`${this.baseUrl}${path}`);
    if (params) {
      for (const [k, v] of Object.entries(params)) {
        url.searchParams.set(k, v);
      }
    }
    return url.toString();
  }

  private buildHeaders(extra?: Record<string, string>): Record<string, string> {
    const h: Record<string, string> = {
      'User-Agent': USER_AGENT,
      'Accept': 'application/json',
    };
    if (this.apiKey) h['x-api-key'] = this.apiKey;
    if (this.orgId) h['x-org-id'] = this.orgId;
    if (extra) Object.assign(h, extra);
    return h;
  }

  private async request(opts: {
    method: string;
    path: string;
    params?: Record<string, string>;
    body?: unknown;
    extraHeaders?: Record<string, string>;
    timeoutMs?: number;
  }): Promise<unknown> {
    const url = this.buildUrl(opts.path, opts.params);
    const headers = this.buildHeaders(opts.extraHeaders);
    let bodyStr: string | undefined;
    if (opts.body !== undefined) {
      headers['Content-Type'] = 'application/json';
      bodyStr = JSON.stringify(opts.body);
    }

    const timeoutMs = opts.timeoutMs ?? this.timeoutMs;
    let response: Response;
    try {
      response = await this.fetchFn(url, {
        method: opts.method,
        headers,
        body: bodyStr,
        signal: AbortSignal.timeout(timeoutMs),
      });
    } catch (e) {
      throw new ConnectionError(String(e));
    }

    const text = await response.text();
    let parsed: unknown = null;
    if (text) {
      try {
        parsed = JSON.parse(text);
      } catch {
        parsed = text;
      }
    }

    raiseForStatus(response.status, parsed);
    return parsed;
  }
}
