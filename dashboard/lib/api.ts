/**
 * API client for the ContractGate Rust backend.
 * All functions throw on non-2xx responses.
 */

const BASE = process.env.NEXT_PUBLIC_API_URL ?? "http://localhost:3001";
const API_KEY = process.env.NEXT_PUBLIC_API_KEY ?? "";

async function apiFetch<T>(path: string, init?: RequestInit): Promise<T> {
  const headers: Record<string, string> = { "Content-Type": "application/json" };
  if (API_KEY) headers["x-api-key"] = API_KEY;
  // Merge any caller-supplied headers (supports Headers, string[][], or plain object)
  if (init?.headers) {
    new Headers(init.headers).forEach((v, k) => { headers[k] = v; });
  }
  const res = await fetch(`${BASE}${path}`, { ...init, headers });
  // 207 Multi-Status is a valid success response from the ingest endpoint
  if (!res.ok && res.status !== 207) {
    const body = await res.json().catch(() => ({ error: res.statusText }));
    throw new Error(body?.error ?? `Request failed: ${res.status}`);
  }
  return res.json() as Promise<T>;
}

// ---------------------------------------------------------------------------
// Types (mirrors Rust structs)
// ---------------------------------------------------------------------------

export interface ContractSummary {
  id: string;
  name: string;
  version: string;
  active: boolean;
}

export interface ContractResponse extends ContractSummary {
  created_at: string;
  updated_at: string;
  /** Raw YAML definition — included in all create/get/update responses */
  yaml_content: string;
}

export interface Violation {
  field: string;
  message: string;
  kind: string;
}

export interface ValidationResult {
  passed: boolean;
  violations: Violation[];
  validation_us: number;
}

export interface IngestEventResult extends ValidationResult {
  forwarded: boolean;
}

export interface BatchIngestResponse {
  total: number;
  passed: number;
  failed: number;
  /** true when the request was sent with ?dry_run=true — no data was persisted */
  dry_run: boolean;
  results: IngestEventResult[];
}

export interface IngestionStats {
  total_events: number;
  passed_events: number;
  failed_events: number;
  pass_rate: number;
  avg_validation_us: number;
  /** Median validation latency in microseconds */
  p50_validation_us: number;
  /** 95th-percentile validation latency in microseconds */
  p95_validation_us: number;
  /** 99th-percentile validation latency in microseconds (target: <15 000 µs) */
  p99_validation_us: number;
}

export interface AuditEntry {
  id: string;
  contract_id: string;
  passed: boolean;
  violation_count: number;
  violation_details: Violation[];
  validation_us: number;
  source_ip: string | null;
  created_at: string;
}

// ---------------------------------------------------------------------------
// Contracts
// ---------------------------------------------------------------------------

export const listContracts = () => apiFetch<ContractSummary[]>("/contracts");

export const getContract = (id: string) =>
  apiFetch<ContractResponse>(`/contracts/${id}`);

export const createContract = (yaml_content: string) =>
  apiFetch<ContractResponse>("/contracts", {
    method: "POST",
    body: JSON.stringify({ yaml_content }),
  });

export const updateContract = (
  id: string,
  patch: { active?: boolean; yaml_content?: string }
) =>
  apiFetch<ContractResponse>(`/contracts/${id}`, {
    method: "PUT",
    body: JSON.stringify(patch),
  });

export const deleteContract = (id: string) =>
  apiFetch<void>(`/contracts/${id}`, { method: "DELETE" });

// ---------------------------------------------------------------------------
// Ingestion
// ---------------------------------------------------------------------------

export const ingestEvent = (
  contractId: string,
  event: unknown,
  opts: { dryRun?: boolean } = {}
) => {
  const qs = opts.dryRun ? "?dry_run=true" : "";
  return apiFetch<BatchIngestResponse>(`/ingest/${contractId}${qs}`, {
    method: "POST",
    body: JSON.stringify(event),
  });
};

export const getContractStats = (contractId: string) =>
  apiFetch<IngestionStats>(`/ingest/${contractId}/stats`);

export const getGlobalStats = () => apiFetch<IngestionStats>("/stats");

// ---------------------------------------------------------------------------
// Audit log
// ---------------------------------------------------------------------------

export const getAuditLog = (params?: {
  contract_id?: string;
  limit?: number;
  offset?: number;
}) => {
  const qs = new URLSearchParams();
  if (params?.contract_id) qs.set("contract_id", params.contract_id);
  if (params?.limit != null) qs.set("limit", String(params.limit));
  if (params?.offset != null) qs.set("offset", String(params.offset));
  return apiFetch<AuditEntry[]>(`/audit?${qs}`);
};

// ---------------------------------------------------------------------------
// Playground
// ---------------------------------------------------------------------------

export const playgroundValidate = (yaml_content: string, event: unknown) =>
  apiFetch<ValidationResult>("/playground/validate", {
    method: "POST",
    body: JSON.stringify({ yaml_content, event }),
  });
