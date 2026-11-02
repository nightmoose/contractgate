/** Thin HTTP client for the DataContractGate gateway. No validation logic. */

export const DEFAULT_BASE_URL = "https://api.datacontractgate.com";
const USER_AGENT = "contractgate-mcp/0.1.0";

export class GatewayError extends Error {
  readonly status: number;
  readonly body: string;

  constructor(status: number, body: string) {
    super(`DataContractGate HTTP ${status}: ${body}`);
    this.name = "GatewayError";
    this.status = status;
    this.body = body;
  }
}

export class ConfigError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "ConfigError";
  }
}

export type FetchFn = typeof fetch;

export class Gateway {
  readonly baseUrl: string;
  readonly apiKey: string;
  private readonly fetchFn: FetchFn;

  constructor(opts: { baseUrl?: string; apiKey: string; fetch?: FetchFn }) {
    this.baseUrl = (opts.baseUrl ?? DEFAULT_BASE_URL).replace(/\/+$/, "");
    this.apiKey = opts.apiKey;
    this.fetchFn = opts.fetch ?? globalThis.fetch;
  }

  static fromEnv(env: NodeJS.ProcessEnv = process.env, fetchFn?: FetchFn): Gateway {
    const apiKey = env.CONTRACTGATE_API_KEY?.trim();
    if (!apiKey) {
      throw new ConfigError(
        "CONTRACTGATE_API_KEY is not set. Create a key at https://app.datacontractgate.com/account and export it. Never inline the key in MCP config.",
      );
    }
    return new Gateway({
      baseUrl: env.CONTRACTGATE_BASE_URL?.trim() || DEFAULT_BASE_URL,
      apiKey,
      fetch: fetchFn,
    });
  }

  infer(body: { name: string; samples: unknown[]; description?: string }) {
    return this.request("POST", "/contracts/infer", { body });
  }

  ingest(
    contractId: string,
    events: unknown,
    opts: { dryRun: boolean; onFail?: "quarantine" | "drop" },
  ) {
    const q = new URLSearchParams();
    if (opts.dryRun) q.set("dry_run", "true");
    if (opts.onFail) q.set("on_fail", opts.onFail);
    const qs = q.toString();
    // 207 mixed / 422 all-failed are data results, not transport errors.
    return this.request("POST", `/v1/ingest/${encodeURIComponent(contractId)}${qs ? `?${qs}` : ""}`, {
      body: events,
      ok: [200, 207, 422],
    });
  }

  egress(
    contractId: string,
    events: unknown,
    opts: { dryRun: boolean; disposition?: string; version?: string; onFail?: "quarantine" | "drop" },
  ) {
    const q = new URLSearchParams();
    if (opts.dryRun) q.set("dry_run", "true");
    if (opts.disposition) q.set("disposition", opts.disposition);
    if (opts.onFail) q.set("on_fail", opts.onFail);
    // 207 mixed / 422 all-failed are data results, not transport errors.
    return this.request("POST", `/egress/${encodeURIComponent(contractId)}${q.toString() ? `?${q.toString()}` : ""}`, {
      body: events,
      ok: [200, 207, 422],
    });
  }

  playground(yamlContent: string, event: unknown) {
    return this.request("POST", "/playground/validate", {
      body: { yaml_content: yamlContent, event },
    });
  }

  deploy(body: {
    name: string;
    yaml_content: string;
    source?: string;
    deployed_by?: string;
  }) {
    return this.request("POST", "/contracts/deploy", { body });
  }

  quarantine(opts: { contractId?: string; limit?: number; offset?: number }) {
    const q = new URLSearchParams();
    if (opts.contractId) q.set("contract_id", opts.contractId);
    if (opts.limit != null) q.set("limit", String(opts.limit));
    if (opts.offset != null) q.set("offset", String(opts.offset));
    const qs = q.toString();
    return this.request("GET", `/quarantine${qs ? `?${qs}` : ""}`);
  }

  listContracts() {
    return this.request("GET", "/contracts");
  }

  dropCounts(contractId: string, hours?: number) {
    const q = new URLSearchParams();
    if (hours != null) q.set("hours", String(hours));
    const qs = q.toString();
    return this.request(
      "GET",
      `/contracts/${encodeURIComponent(contractId)}/drop-counts${qs ? `?${qs}` : ""}`,
    );
  }

  private async request(
    method: string,
    path: string,
    opts: { body?: unknown; ok?: number[] } = {},
  ): Promise<unknown> {
    const headers: Record<string, string> = {
      Accept: "application/json",
      "X-Api-Key": this.apiKey,
      "User-Agent": USER_AGENT,
    };
    const init: RequestInit = { method, headers };
    if (opts.body !== undefined) {
      headers["Content-Type"] = "application/json";
      init.body = JSON.stringify(opts.body);
    }
    const res = await this.fetchFn(`${this.baseUrl}${path}`, init);
    const text = await res.text();
    const allowed = opts.ok;
    const success = allowed
      ? allowed.includes(res.status)
      : res.status >= 200 && res.status < 300;
    if (!success) {
      throw new GatewayError(res.status, planRequiredText(text) || text || res.statusText);
    }
    return decode(text);
  }
}

function planRequiredText(text: string): string | null {
  try {
    const body = JSON.parse(text) as { error?: unknown; message?: unknown };
    if (body.error === "plan_required" && typeof body.message === "string" && body.message) {
      return body.message;
    }
  } catch {
    return null;
  }
  return null;
}

function decode(text: string): unknown {
  if (!text) return null;
  try {
    return JSON.parse(text);
  } catch {
    return text;
  }
}
