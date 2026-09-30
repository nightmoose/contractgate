export type ViolationKind =
  | 'missing_required_field'
  | 'type_mismatch'
  | 'pattern_mismatch'
  | 'enum_violation'
  | 'range_violation'
  | 'length_violation'
  | 'metric_range_violation'
  | 'unknown_field'
  | 'undeclared_field';

export interface Violation {
  field: string;
  message: string;
  kind: ViolationKind;
}

export interface ValidationResult {
  passed: boolean;
  violations: Violation[];
  validation_us: number;
}

// ---------------------------------------------------------------------------
// Ingest
// ---------------------------------------------------------------------------

export interface IngestEventResult {
  passed: boolean;
  violations: Violation[];
  validation_us: number;
  forwarded: boolean;
  contract_version: string;
  transformed_event: unknown;
}

export interface BatchIngestResponse {
  total: number;
  passed: number;
  failed: number;
  dry_run: boolean;
  atomic: boolean;
  resolved_version: string;
  version_pin_source: string;
  results: IngestEventResult[];
}

// ---------------------------------------------------------------------------
// Egress
// ---------------------------------------------------------------------------

export interface EgressOutcome {
  index: number;
  passed: boolean;
  violations: Violation[];
  validation_us: number;
  action: string;
}

export interface EgressResponse {
  total: number;
  passed: number;
  failed: number;
  dry_run: boolean;
  disposition: string;
  resolved_version: string;
  payload: unknown[];
  outcomes: EgressOutcome[];
}

// ---------------------------------------------------------------------------
// Contract / version
// ---------------------------------------------------------------------------

export interface ContractResponse {
  id: string;
  name: string;
  description: string | null;
  multi_stable_resolution: string;
  created_at: string;
  updated_at: string;
  version_count: number;
  latest_stable_version: string | null;
}

export interface VersionResponse {
  id: string;
  contract_id: string;
  version: string;
  state: string;
  yaml_content: string;
  created_at: string;
  promoted_at: string | null;
  deprecated_at: string | null;
  compliance_mode: boolean;
}

export interface VersionSummary {
  version: string;
  state: string;
  created_at: string;
  promoted_at: string | null;
  deprecated_at: string | null;
}

// ---------------------------------------------------------------------------
// Audit
// ---------------------------------------------------------------------------

export interface AuditEntry {
  id: string;
  contract_id: string;
  contract_version: string | null;
  passed: boolean;
  violation_count: number;
  violation_details: unknown;
  raw_event: unknown;
  validation_us: number;
  source_ip: string | null;
  created_at: string;
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

// ---------------------------------------------------------------------------
// JSON deserialization helpers
// ---------------------------------------------------------------------------

type Raw = Record<string, unknown>;

function toRaw(v: unknown): Raw {
  if (v === null || typeof v !== 'object' || Array.isArray(v)) {
    throw new Error(`Expected object, got ${JSON.stringify(v)}`);
  }
  return v as Raw;
}

function optStr(v: unknown): string | null {
  return typeof v === 'string' ? v : null;
}

function toBool(v: unknown, fallback = false): boolean {
  return typeof v === 'boolean' ? v : fallback;
}

function toInt(v: unknown, fallback = 0): number {
  return typeof v === 'number' ? Math.trunc(v) : fallback;
}

export function violationFromJson(raw: unknown): Violation {
  const r = toRaw(raw);
  return {
    field: String(r['field'] ?? ''),
    message: String(r['message'] ?? ''),
    kind: String(r['kind'] ?? '') as ViolationKind,
  };
}

export function validationResultFromJson(raw: unknown): ValidationResult {
  const r = toRaw(raw);
  const vList = Array.isArray(r['violations']) ? r['violations'] : [];
  return {
    passed: toBool(r['passed']),
    violations: vList.map(violationFromJson),
    validation_us: toInt(r['validation_us']),
  };
}

export function ingestEventResultFromJson(raw: unknown): IngestEventResult {
  const r = toRaw(raw);
  const vList = Array.isArray(r['violations']) ? r['violations'] : [];
  return {
    passed: toBool(r['passed']),
    violations: vList.map(violationFromJson),
    validation_us: toInt(r['validation_us']),
    forwarded: toBool(r['forwarded']),
    contract_version: String(r['contract_version'] ?? ''),
    transformed_event: r['transformed_event'] ?? null,
  };
}

export function batchIngestResponseFromJson(raw: unknown): BatchIngestResponse {
  const r = toRaw(raw);
  const rList = Array.isArray(r['results']) ? r['results'] : [];
  return {
    total: toInt(r['total']),
    passed: toInt(r['passed']),
    failed: toInt(r['failed']),
    dry_run: toBool(r['dry_run']),
    atomic: toBool(r['atomic']),
    resolved_version: String(r['resolved_version'] ?? ''),
    version_pin_source: String(r['version_pin_source'] ?? ''),
    results: rList.map(ingestEventResultFromJson),
  };
}

export function egressOutcomeFromJson(raw: unknown): EgressOutcome {
  const r = toRaw(raw);
  const vList = Array.isArray(r['violations']) ? r['violations'] : [];
  return {
    index: toInt(r['index']),
    passed: toBool(r['passed']),
    violations: vList.map(violationFromJson),
    validation_us: toInt(r['validation_us']),
    action: String(r['action'] ?? ''),
  };
}

export function egressResponseFromJson(raw: unknown): EgressResponse {
  const r = toRaw(raw);
  const oList = Array.isArray(r['outcomes']) ? r['outcomes'] : [];
  return {
    total: toInt(r['total']),
    passed: toInt(r['passed']),
    failed: toInt(r['failed']),
    dry_run: toBool(r['dry_run']),
    disposition: String(r['disposition'] ?? 'block'),
    resolved_version: String(r['resolved_version'] ?? ''),
    payload: Array.isArray(r['payload']) ? r['payload'] : [],
    outcomes: oList.map(egressOutcomeFromJson),
  };
}

export function contractResponseFromJson(raw: unknown): ContractResponse {
  const r = toRaw(raw);
  return {
    id: String(r['id'] ?? ''),
    name: String(r['name'] ?? ''),
    description: optStr(r['description']),
    multi_stable_resolution: String(r['multi_stable_resolution'] ?? 'strict'),
    created_at: String(r['created_at'] ?? ''),
    updated_at: String(r['updated_at'] ?? ''),
    version_count: toInt(r['version_count']),
    latest_stable_version: optStr(r['latest_stable_version']),
  };
}

export function versionResponseFromJson(raw: unknown): VersionResponse {
  const r = toRaw(raw);
  return {
    id: String(r['id'] ?? ''),
    contract_id: String(r['contract_id'] ?? ''),
    version: String(r['version'] ?? ''),
    state: String(r['state'] ?? ''),
    yaml_content: String(r['yaml_content'] ?? ''),
    created_at: String(r['created_at'] ?? ''),
    promoted_at: optStr(r['promoted_at']),
    deprecated_at: optStr(r['deprecated_at']),
    compliance_mode: toBool(r['compliance_mode']),
  };
}

export function versionSummaryFromJson(raw: unknown): VersionSummary {
  const r = toRaw(raw);
  return {
    version: String(r['version'] ?? ''),
    state: String(r['state'] ?? ''),
    created_at: String(r['created_at'] ?? ''),
    promoted_at: optStr(r['promoted_at']),
    deprecated_at: optStr(r['deprecated_at']),
  };
}

export function auditEntryFromJson(raw: unknown): AuditEntry {
  const r = toRaw(raw);
  return {
    id: String(r['id'] ?? ''),
    contract_id: String(r['contract_id'] ?? ''),
    contract_version: optStr(r['contract_version']),
    passed: toBool(r['passed']),
    violation_count: toInt(r['violation_count']),
    violation_details: r['violation_details'] ?? null,
    raw_event: r['raw_event'] ?? null,
    validation_us: toInt(r['validation_us']),
    source_ip: optStr(r['source_ip']),
    created_at: String(r['created_at'] ?? ''),
  };
}

export function ingestionStatsFromJson(raw: unknown): IngestionStats {
  const r = toRaw(raw);
  return {
    total_events: toInt(r['total_events']),
    passed_events: toInt(r['passed_events']),
    failed_events: toInt(r['failed_events']),
    pass_rate: typeof r['pass_rate'] === 'number' ? r['pass_rate'] : 0,
    avg_validation_us: typeof r['avg_validation_us'] === 'number' ? r['avg_validation_us'] : 0,
    p50_validation_us: toInt(r['p50_validation_us']),
    p95_validation_us: toInt(r['p95_validation_us']),
    p99_validation_us: toInt(r['p99_validation_us']),
  };
}

export function expectList(body: unknown): unknown[] {
  if (body === null || body === undefined) return [];
  if (Array.isArray(body)) return body;
  throw new Error(`Expected list response, got ${typeof body}`);
}
