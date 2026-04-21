/**
 * API client for the ContractGate Rust backend.
 * All functions throw on non-2xx responses.
 */

import * as yaml from "js-yaml";

const BASE = process.env.NEXT_PUBLIC_API_URL ?? "http://localhost:3001";
const API_KEY = process.env.NEXT_PUBLIC_API_KEY ?? "";

/** Parse name + description out of a contract YAML string. */
function extractYamlMeta(yaml_content: string): { name: string; description?: string } {
  try {
    const doc = yaml.load(yaml_content) as Record<string, unknown>;
    const name = typeof doc?.name === "string" ? doc.name : "";
    const description = typeof doc?.description === "string" ? doc.description : undefined;
    return { name, description };
  } catch {
    return { name: "" };
  }
}

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

// ---------------------------------------------------------------------------
// RFC-004: PII transforms
//
// These describe the YAML the dashboard emits into `ontology.entities[].transform`
// and the contract-level `compliance_mode` flag.  They are NOT on any response
// payload directly — the server round-trips them through `yaml_content`.
// ---------------------------------------------------------------------------

/** Which transform to apply to a string field before it reaches durable storage. */
export type TransformKind = "mask" | "hash" | "drop" | "redact";

/** Sub-style for `kind: "mask"`.  Ignored for every other kind. */
export type MaskStyle = "opaque" | "format_preserving";

/**
 * Per-field PII transform declaration.  Only valid on string fields — the
 * server rejects contracts at compile time that attach a transform to a
 * non-string entity.  `style` is only meaningful when `kind === "mask"`
 * and defaults to `"opaque"` when omitted.
 */
export interface Transform {
  kind: TransformKind;
  style?: MaskStyle;
}

export interface ValidationResult {
  passed: boolean;
  violations: Violation[];
  validation_us: number;
}

/**
 * Playground response — extends `ValidationResult` with the post-transform
 * payload that *would* be persisted if this YAML were saved and ingested
 * against.  Always populated; if the contract declares no transforms and
 * `compliance_mode` is off, this is byte-for-byte identical to the
 * request body.
 *
 * Note: the Playground has no per-contract salt (the contract isn't
 * saved yet), so `hash` and `format_preserving` mask outputs here are
 * illustrative — production ingest uses the real per-contract salt.
 */
export interface PlaygroundResponse extends ValidationResult {
  transformed_event: unknown;
}

export interface IngestEventResult extends ValidationResult {
  forwarded: boolean;
  /**
   * The contract version that actually produced the decision for this event.
   * Under `multi_stable_resolution = fallback` this may differ from the
   * request's resolved version when a fallback stable accepted the event.
   * Always set by the backend (RFC-002).
   */
  contract_version: string;
  /**
   * RFC-004 echo: the post-transform payload as it was written to
   * `audit_log` / `quarantine_events` / the forward destination.  If the
   * matching contract version declares no transforms this is byte-for-byte
   * identical to the request body; otherwise values have already been
   * masked / hashed / dropped / redacted.  Callers that need the raw
   * request body must track it themselves — it does not survive to any
   * persisted row.
   */
  transformed_event: unknown;
}

export interface BatchIngestResponse {
  total: number;
  passed: number;
  failed: number;
  /** true when the request was sent with ?dry_run=true — no data was persisted */
  dry_run: boolean;
  /** true when the request was sent with ?atomic=true */
  atomic: boolean;
  /**
   * The version the request was dispatched against before any fallback retry.
   * Mirrors what got logged server-side (RFC-002).
   */
  resolved_version: string;
  /** Where the resolved version came from: "header", "path", "default_stable", or "pinned_deprecated". */
  version_pin_source: string;
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
  /**
   * The exact contract version that produced this decision (RFC-002).
   * Null only for legacy rows written before RFC-002 shipped; every row
   * written on or after the versioning rollout has this populated.
   */
  contract_version: string | null;
  passed: boolean;
  violation_count: number;
  violation_details: Violation[];
  /**
   * The post-transform payload as it was written to `audit_log.raw_event`
   * (RFC-004 §6).  This is ALREADY scrubbed — masks, hashes, drops, and
   * redactions have been applied before the row landed.  If the matching
   * contract version declared no transforms, this is byte-for-byte
   * identical to the request body; otherwise values are post-transform.
   * Raw PII never reaches this field.
   */
  raw_event: unknown;
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

export const createContract = (yaml_content: string) => {
  const { name, description } = extractYamlMeta(yaml_content);
  return apiFetch<ContractResponse>("/contracts", {
    method: "POST",
    body: JSON.stringify({ name, description, yaml_content }),
  });
};

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
  apiFetch<PlaygroundResponse>("/playground/validate", {
    method: "POST",
    body: JSON.stringify({ yaml_content, event }),
  });
