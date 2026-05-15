/**
 * API client for the ContractGate Rust backend.
 * All functions throw on non-2xx responses.
 *
 * Shape is authoritative from `src/contract.rs` and `src/main.rs` under
 * RFC-002 (contract versioning) and RFC-004 (PII transforms).  The key
 * invariants every caller must respect:
 *
 *   1. Contracts have identity (name, description, policy).  YAML lives
 *      on `contract_versions`, one row per (contract, version) pair.
 *   2. A version is draft / stable / deprecated.  Only drafts can have
 *      their YAML edited (PATCH /contracts/:id/versions/:version).
 *      Promoting a draft freezes it forever.
 *   3. `updateContract` is a PATCH and is identity-only — name,
 *      description, resolution policy.  It will never touch YAML.
 *      Use `patchVersionYaml` (drafts only) or create a new draft via
 *      `createVersion` to ship YAML changes.
 */

import * as yaml from "js-yaml";
import { DEMO_MODE, DEMO_ORG_UUID } from "@/lib/demo";

const BASE = process.env.NEXT_PUBLIC_API_URL ?? "http://localhost:8080";
const API_KEY = process.env.NEXT_PUBLIC_API_KEY ?? "";

/**
 * The current user's org_id, set by OrgProvider once the Supabase session
 * resolves.  Sent as `x-org-id` on every Rust API call so the backend can
 * scope queries even when using the legacy env-var key (which carries no
 * org context of its own).  A DB-backed key always takes precedence on the
 * Rust side — this header is only the fallback.
 *
 * In demo mode this is pre-populated at module init so the first API call
 * (before any provider mounts) already carries the correct org header.
 */
let _apiOrgId: string | null = DEMO_MODE ? DEMO_ORG_UUID : null;

export function setApiOrgId(orgId: string): void {
  _apiOrgId = orgId;
}

/** Parse name + description out of a contract YAML string. */
function extractYamlMeta(
  yaml_content: string
): { name: string; description?: string } {
  try {
    const doc = yaml.load(yaml_content) as Record<string, unknown>;
    const name = typeof doc?.name === "string" ? doc.name : "";
    const description =
      typeof doc?.description === "string" ? doc.description : undefined;
    return { name, description };
  } catch {
    return { name: "" };
  }
}

/**
 * Extract a human-useful error message from a non-OK Response.  Tries the
 * JSON `{error}` shape the Rust API always emits, then falls back to the
 * raw text body (truncated for sanity), then to `statusText`.  Never
 * throws — even `.text()` errors collapse to `Request failed: <status>`.
 */
async function extractErrorMessage(res: Response): Promise<string> {
  try {
    const body = await res.clone().json();
    if (body && typeof body === "object" && typeof body.error === "string") {
      return body.error;
    }
  } catch {
    // not JSON — fall through
  }
  try {
    const text = (await res.text()).trim();
    if (text) {
      const snippet = text.length > 500 ? `${text.slice(0, 500)}…` : text;
      return `Request failed: ${res.status} ${res.statusText}: ${snippet}`;
    }
  } catch {
    // body already consumed or network read failed — fall through
  }
  return `Request failed: ${res.status} ${res.statusText || ""}`.trim();
}

async function apiFetch<T>(path: string, init?: RequestInit): Promise<T> {
  const headers: Record<string, string> = {
    "Content-Type": "application/json",
  };
  if (API_KEY) headers["x-api-key"] = API_KEY;
  if (_apiOrgId) headers["x-org-id"] = _apiOrgId;
  // Merge any caller-supplied headers (supports Headers, string[][], or plain object)
  if (init?.headers) {
    new Headers(init.headers).forEach((v, k) => {
      headers[k] = v;
    });
  }
  const res = await fetch(`${BASE}${path}`, { ...init, headers });
  // 207 Multi-Status is a valid success response from the ingest endpoint
  if (!res.ok && res.status !== 207) {
    // Try JSON first (the Rust API always returns `{error, status}` on
    // failure).  If that fails — e.g. the server returned an HTML error
    // page from a proxy, or an empty body — fall back to the raw text so
    // the thrown Error carries something more useful than `statusText`.
    const message = await extractErrorMessage(res);
    throw new Error(message);
  }
  // 204 No Content — typed as void by callers
  if (res.status === 204) return undefined as T;
  return res.json() as Promise<T>;
}

// ---------------------------------------------------------------------------
// Types (mirrors Rust structs in src/contract.rs)
// ---------------------------------------------------------------------------

/** Mirrors `enum MultiStableResolution { Strict, Fallback }`. */
export type MultiStableResolution = "strict" | "fallback";

/** Mirrors `enum VersionState { Draft, Stable, Deprecated }`. */
export type VersionState = "draft" | "stable" | "deprecated";

/**
 * Lightweight listing row for `GET /contracts`.  Identity-only — no YAML.
 * `latest_stable_version` is null when every version is still a draft or
 * every stable has been deprecated.
 */
export interface ContractSummary {
  id: string;
  name: string;
  multi_stable_resolution: MultiStableResolution;
  latest_stable_version: string | null;
  version_count: number;
}

/**
 * Full response for `GET /contracts/:id` / `POST /contracts` /
 * `PATCH /contracts/:id`.  Still identity-only — fetch YAML via
 * `getLatestStableVersion` or `getVersion`.
 */
export interface ContractResponse {
  id: string;
  name: string;
  description: string | null;
  multi_stable_resolution: MultiStableResolution;
  created_at: string;
  updated_at: string;
  version_count: number;
  latest_stable_version: string | null;
}

/** Source of a contract version's YAML content. */
export type ImportSource = "native" | "odcs" | "odcs_stripped";

/** One row for `GET /contracts/:id/versions`. */
export interface VersionSummary {
  version: string;
  state: VersionState;
  created_at: string;
  promoted_at: string | null;
  deprecated_at: string | null;
  /** Where the YAML originated. */
  import_source: ImportSource;
  /** True when the version needs human review before promotion (D-002). */
  requires_review: boolean;
}

/** Full response for a single version — includes YAML. */
export interface VersionResponse {
  id: string;
  contract_id: string;
  version: string;
  state: VersionState;
  yaml_content: string;
  created_at: string;
  promoted_at: string | null;
  deprecated_at: string | null;
  /** RFC-004: when true, undeclared inbound fields fail validation. */
  compliance_mode: boolean;
  /** Where the YAML originated. */
  import_source: ImportSource;
  /** True when the version needs human review before promotion (D-002). */
  requires_review: boolean;
}

/** Row in the append-only rename log for `GET /contracts/:id/name-history`. */
export interface NameHistoryEntry {
  id: string;
  contract_id: string;
  old_name: string;
  new_name: string;
  changed_at: string;
}

export interface Violation {
  field: string;
  message: string;
  kind: string;
}

// ---------------------------------------------------------------------------
// RFC-004: PII transforms — shape of the YAML the dashboard emits.
// ---------------------------------------------------------------------------

export type TransformKind = "mask" | "hash" | "drop" | "redact";
export type MaskStyle = "opaque" | "format_preserving";

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
 */
export interface PlaygroundResponse extends ValidationResult {
  transformed_event: unknown;
}

export interface IngestEventResult extends ValidationResult {
  forwarded: boolean;
  /** Contract version that actually accepted/rejected this event (RFC-002). */
  contract_version: string;
  /** Post-transform payload as it was written to storage (RFC-004). */
  transformed_event: unknown;
}

export interface BatchIngestResponse {
  total: number;
  passed: number;
  failed: number;
  dry_run: boolean;
  /** When true the backend treats every event in the batch atomically:
   *  any single failure causes the entire batch to fail and nothing is
   *  forwarded.  Optional for backwards compatibility with older deploys. */
  atomic?: boolean;
  resolved_version: string;
  version_pin_source: string;
  results: IngestEventResult[];
}

export interface IngestionStats {
  total_events: number;
  passed_events: number;
  failed_events: number;
  pass_rate: number;
  avg_validation_us: number;
  p50_validation_us: number;
  p95_validation_us: number;
  p99_validation_us: number;
}

export interface AuditEntry {
  id: string;
  contract_id: string;
  contract_version: string | null;
  passed: boolean;
  violation_count: number;
  violation_details: Violation[];
  /** Post-transform payload (RFC-004 §6) — PII already scrubbed. */
  raw_event: unknown;
  validation_us: number;
  source_ip: string | null;
  created_at: string;
}

// ---------------------------------------------------------------------------
// Quarantine — failed events held for inspection and optional replay.
// ---------------------------------------------------------------------------

/** A failed event held in the quarantine store for manual review / replay. */
export interface QuarantinedEvent {
  id: string;
  contract_id: string;
  contract_version: string | null;
  raw_event: unknown;
  violation_details: Violation[];
  violation_count: number;
  source_ip: string | null;
  quarantined_at: string;
  /** How many times this event has been replayed so far. */
  replay_count: number;
  last_replayed_at: string | null;
  /** Result of the most recent replay attempt, or null if never replayed. */
  last_replay_passed: boolean | null;
}

/** One outcome row written by the replay engine for a single event. */
export interface ReplayOutcome {
  event_id: string;
  version: string;
  passed: boolean;
  violations: Violation[];
  replayed_at: string;
}

/** Response body for `POST /quarantine/replay`. */
export interface ReplayResponse {
  replayed: number;
  outcomes: ReplayOutcome[];
}

// ---------------------------------------------------------------------------
// Contracts — identity-level CRUD
// ---------------------------------------------------------------------------

export const listContracts = () => apiFetch<ContractSummary[]>("/contracts");

export const getContract = (id: string) =>
  apiFetch<ContractResponse>(`/contracts/${id}`);

/**
 * Create a contract + its v1.0.0 draft in a single transactional call.
 * `name` and `description` are parsed out of the YAML (the server also
 * validates the YAML itself and requires `name` on the request body).
 */
export const createContract = (yaml_content: string) => {
  const { name, description } = extractYamlMeta(yaml_content);
  return apiFetch<ContractResponse>("/contracts", {
    method: "POST",
    body: JSON.stringify({ name, description, yaml_content }),
  });
};

/**
 * Identity-level metadata patch.  Does NOT touch YAML — that is immutable
 * once a version leaves draft.  Use `createVersion` to ship YAML changes.
 */
export const updateContract = (
  id: string,
  patch: {
    name?: string;
    description?: string;
    multi_stable_resolution?: MultiStableResolution;
  }
) =>
  apiFetch<ContractResponse>(`/contracts/${id}`, {
    method: "PATCH",
    body: JSON.stringify(patch),
  });

export const deleteContract = (id: string) =>
  apiFetch<void>(`/contracts/${id}`, { method: "DELETE" });

export const listNameHistory = (contractId: string) =>
  apiFetch<NameHistoryEntry[]>(`/contracts/${contractId}/name-history`);

// ---------------------------------------------------------------------------
// Versions — one row per (contract, version) pair.  YAML lives here.
// ---------------------------------------------------------------------------

export const listVersions = (contractId: string) =>
  apiFetch<VersionSummary[]>(`/contracts/${contractId}/versions`);

/**
 * Create a new DRAFT version.  `version` must be unique per contract
 * (server enforces).  The new version is always born in `draft` state;
 * promote it via `promoteVersion` to make it eligible for ingest.
 */
export const createVersion = (
  contractId: string,
  body: { version: string; yaml_content: string }
) =>
  apiFetch<VersionResponse>(`/contracts/${contractId}/versions`, {
    method: "POST",
    body: JSON.stringify(body),
  });

export const getVersion = (contractId: string, version: string) =>
  apiFetch<VersionResponse>(
    `/contracts/${contractId}/versions/${encodeURIComponent(version)}`
  );

export const getLatestStableVersion = (contractId: string) =>
  apiFetch<VersionResponse>(
    `/contracts/${contractId}/versions/latest-stable`
  );

/** Edit a draft version's YAML.  Fails server-side if the version is not draft. */
export const patchVersionYaml = (
  contractId: string,
  version: string,
  yaml_content: string
) =>
  apiFetch<VersionResponse>(
    `/contracts/${contractId}/versions/${encodeURIComponent(version)}`,
    { method: "PATCH", body: JSON.stringify({ yaml_content }) }
  );

/** Promote draft → stable.  Irreversible freeze of this version's YAML. */
export const promoteVersion = (contractId: string, version: string) =>
  apiFetch<VersionResponse>(
    `/contracts/${contractId}/versions/${encodeURIComponent(version)}/promote`,
    { method: "POST" }
  );

/** Mark a stable version as deprecated.  Ingest still works but new traffic
 *  resolves to the next-newest stable instead. */
export const deprecateVersion = (contractId: string, version: string) =>
  apiFetch<VersionResponse>(
    `/contracts/${contractId}/versions/${encodeURIComponent(version)}/deprecate`,
    { method: "POST" }
  );

/** Delete a version.  Server only allows this for drafts. */
export const deleteVersion = (contractId: string, version: string) =>
  apiFetch<void>(
    `/contracts/${contractId}/versions/${encodeURIComponent(version)}`,
    { method: "DELETE" }
  );

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
// Quarantine
// ---------------------------------------------------------------------------

export const listQuarantinedEvents = (params?: {
  contract_id?: string;
  limit?: number;
  offset?: number;
}) => {
  const qs = new URLSearchParams();
  if (params?.contract_id) qs.set("contract_id", params.contract_id);
  if (params?.limit != null) qs.set("limit", String(params.limit));
  if (params?.offset != null) qs.set("offset", String(params.offset));
  return apiFetch<QuarantinedEvent[]>(`/quarantine?${qs}`);
};

export const replayEvents = (
  eventIds: string[],
  opts?: { version?: string; contract_id?: string }
) =>
  apiFetch<ReplayResponse>("/quarantine/replay", {
    method: "POST",
    body: JSON.stringify({
      event_ids: eventIds,
      ...(opts?.version ? { version: opts.version } : {}),
      ...(opts?.contract_id ? { contract_id: opts.contract_id } : {}),
    }),
  });

export const getReplayHistory = (params?: {
  event_id?: string;
  limit?: number;
}) => {
  const qs = new URLSearchParams();
  if (params?.event_id) qs.set("event_id", params.event_id);
  if (params?.limit != null) qs.set("limit", String(params.limit));
  return apiFetch<ReplayOutcome[]>(`/quarantine/replay-history?${qs}`);
};

// ---------------------------------------------------------------------------
// Playground
// ---------------------------------------------------------------------------

export const playgroundValidate = (
  yaml_content: string,
  event: unknown,
  opts?: { atomic?: boolean }
) =>
  apiFetch<PlaygroundResponse>("/playground/validate", {
    method: "POST",
    body: JSON.stringify({
      yaml_content,
      event,
      ...(opts?.atomic != null ? { atomic: opts.atomic } : {}),
    }),
  });

// ---------------------------------------------------------------------------
// ODCS — import, export, approve-import, conformance
// ---------------------------------------------------------------------------

/** Response from `POST /contracts/import`. */
export interface OdcsImportResponse {
  id: string;
  version: string;
  import_source: ImportSource;
  requires_review: boolean;
}

/** Breakdown of which ODCS mandatory fields are present. */
export interface MandatoryFieldsDetail {
  api_version: boolean;
  kind: boolean;
  id: boolean;
  version: boolean;
  status: boolean;
}

/** Breakdown of which CG extensions are present. */
export interface ExtensionsDetail {
  x_contractgate_version: boolean;
  x_contractgate_ontology: boolean;
}

/** Four-dimensional ODCS v3.1.0 conformance report. */
export interface ConformanceReport {
  version: string;
  mandatory_fields_score: number;
  mandatory_fields_detail: MandatoryFieldsDetail;
  extensions_score: number;
  extensions_detail: ExtensionsDetail;
  round_trip_fidelity_score: number;
  round_trip_note: string;
  quality_coverage_pct: number;
  quality_covered_fields: number;
  total_fields: number;
  overall_score: number;
}

/**
 * Import an ODCS v3.1.0 YAML document, creating a new contract + draft version.
 * Returns the newly-created VersionResponse (or a minimal OdcsImportResponse shape).
 */
export const importOdcs = (
  odcs_yaml: string,
  name_override?: string
) =>
  apiFetch<VersionResponse>("/contracts/import", {
    method: "POST",
    body: JSON.stringify({
      odcs_yaml,
      ...(name_override ? { name_override } : {}),
    }),
  });

/**
 * Export a contract version as ODCS v3.1.0 YAML.  Returns raw YAML text.
 */
export const exportOdcs = async (
  contractId: string,
  version: string
): Promise<string> => {
  const headers: Record<string, string> = {};
  if (API_KEY) headers["x-api-key"] = API_KEY;
  if (_apiOrgId) headers["x-org-id"] = _apiOrgId;
  const res = await fetch(
    `${BASE}/contracts/${contractId}/versions/${encodeURIComponent(version)}/export`,
    { headers }
  );
  if (!res.ok) {
    const message = await extractErrorMessage(res);
    throw new Error(message);
  }
  return res.text();
};

/**
 * Clear the `requires_review` flag on a stripped ODCS import draft,
 * allowing it to be promoted.
 */
export const approveImport = (contractId: string, version: string) =>
  apiFetch<VersionResponse>(
    `/contracts/${contractId}/versions/${encodeURIComponent(version)}/approve-import`,
    { method: "POST" }
  );

/** Fetch the ODCS conformance report for a specific version. */
export const getConformanceReport = (contractId: string, version: string) =>
  apiFetch<ConformanceReport>(
    `/contracts/${contractId}/versions/${encodeURIComponent(version)}/odcs-conformance`
  );

// ---------------------------------------------------------------------------
// Kafka Ingress (RFC-025)
// ---------------------------------------------------------------------------

export interface KafkaIngressConfig {
  id: string;
  contract_id: string;
  enabled: boolean;
  bootstrap_servers: string;
  sasl_username: string;
  /** Only present immediately after enabling — shown once. */
  sasl_password?: string;
  topic_raw: string;
  topic_clean: string;
  topic_quarantine: string;
  partition_count: number;
  created_at: string;
}

export const getKafkaIngress = (contractId: string) =>
  apiFetch<KafkaIngressConfig>(`/contracts/${contractId}/kafka-ingress`);

export const enableKafkaIngress = (contractId: string) =>
  apiFetch<KafkaIngressConfig>(`/contracts/${contractId}/kafka-ingress/enable`, {
    method: "POST",
  });

export const disableKafkaIngress = (contractId: string) =>
  apiFetch<void>(`/contracts/${contractId}/kafka-ingress/disable`, {
    method: "DELETE",
  });

// ---------------------------------------------------------------------------
// Kinesis Ingress (RFC-026)
// ---------------------------------------------------------------------------

export interface KinesisIngressConfig {
  id: string;
  contract_id: string;
  enabled: boolean;
  aws_region: string;
  stream_raw: string;
  stream_clean: string;
  stream_quarantine: string;
  raw_stream_arn: string | null;
  clean_stream_arn: string | null;
  quarantine_stream_arn: string | null;
  iam_access_key_id: string | null;
  /** Only present immediately after enabling or credential rotation — shown once. */
  iam_secret_access_key?: string;
  shard_count: number;
  created_at: string;
}

export const getKinesisIngress = (contractId: string) =>
  apiFetch<KinesisIngressConfig>(`/contracts/${contractId}/kinesis-ingress`);

export const enableKinesisIngress = (contractId: string) =>
  apiFetch<KinesisIngressConfig>(
    `/contracts/${contractId}/kinesis-ingress/enable`,
    { method: "POST" }
  );

export const disableKinesisIngress = (contractId: string) =>
  apiFetch<void>(`/contracts/${contractId}/kinesis-ingress/disable`, {
    method: "DELETE",
  });

export const rotateKinesisCredentials = (contractId: string) =>
  apiFetch<KinesisIngressConfig>(
    `/contracts/${contractId}/kinesis-ingress/rotate-credentials`,
    { method: "POST" }
  );

// ---------------------------------------------------------------------------
// Egress validation (RFC-029)
// ---------------------------------------------------------------------------

/**
 * How the egress endpoint handles failing records.
 *
 * - `block` (default) — drop failing records from the response payload.
 * - `fail`  — any failure rejects the entire response (422).
 * - `tag`   — all records pass through; failures are flagged in outcomes.
 */
export type EgressDisposition = "block" | "fail" | "tag";

/** Per-record outcome returned by `POST /egress/{contractId}`. */
export interface EgressOutcome {
  /** Zero-based index of this record in the original payload. */
  index: number;
  passed: boolean;
  violations: Violation[];
  validation_us: number;
  /**
   * What happened to this record:
   * - `"included"` — passed, present in `payload`
   * - `"blocked"`  — failed, dropped from `payload` (block mode)
   * - `"rejected"` — part of a wholesale rejection (fail mode)
   * - `"tagged"`   — failed but present in `payload` with flag (tag mode)
   */
  action: "included" | "blocked" | "rejected" | "tagged";
}

/** Response from `POST /egress/{contractId}`. */
export interface EgressResponse {
  total: number;
  passed: number;
  failed: number;
  dry_run: boolean;
  disposition: EgressDisposition;
  resolved_version: string;
  /**
   * Cleaned / filtered / annotated payload:
   * - block: only passing records
   * - fail: empty when any record fails
   * - tag: all records
   */
  payload: unknown[];
  /** One entry per input record. */
  outcomes: EgressOutcome[];
}

/**
 * Validate an outbound payload against a named contract.
 *
 * Mirrors `ingestEvent` but for the egress path.  The `disposition` parameter
 * controls what happens to failing records (default: `block`).
 *
 * Returns a 207 Multi-Status on partial failure (block/tag) or 422 on full
 * rejection (fail mode or all records failed).  `apiFetch` treats 207 as
 * success, so callers always receive the `EgressResponse` body and can inspect
 * `failed` / `outcomes` to determine what was blocked or tagged.
 */
export const egressValidate = (
  contractId: string,
  payload: unknown,
  opts: {
    disposition?: EgressDisposition;
    dryRun?: boolean;
    version?: string;
  } = {}
) => {
  const qs = new URLSearchParams();
  if (opts.disposition) qs.set("disposition", opts.disposition);
  if (opts.dryRun) qs.set("dry_run", "true");
  const qstr = qs.toString() ? `?${qs}` : "";

  const path = opts.version
    ? `/egress/${contractId}@${encodeURIComponent(opts.version)}${qstr}`
    : `/egress/${contractId}${qstr}`;

  return apiFetch<EgressResponse>(path, {
    method: "POST",
    body: JSON.stringify(payload),
  });
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/**
 * Suggest the next semver for a new draft, bumping the patch component.
 * Accepts `1.0`, `1.0.0`, `2.3.4` and similar; falls back to `1.0.0`
 * if it can't parse.  Used by the Edit modal when the user wants to ship
 * a YAML change as a fresh draft.
 */
export function suggestNextVersion(current: string | null): string {
  if (!current) return "1.0.0";
  const parts = current.trim().split(".");
  const nums = parts.map((p) => parseInt(p, 10));
  if (nums.some((n) => !Number.isFinite(n))) return `${current}.1`;
  while (nums.length < 3) nums.push(0);
  nums[2] += 1;
  return nums.slice(0, 3).join(".");
}
