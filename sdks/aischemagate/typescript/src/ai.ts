/**
 * AISchemaGate guardrail for the Vercel AI SDK.
 *
 * Dependency-light on purpose: this module does not import `ai`. The object
 * returned by `aischemagateGuardrail` is a `LanguageModelV4Middleware`
 * (`specificationVersion: 'v4'`, `wrapGenerate`). Pass it to
 * `wrapLanguageModel` from the `ai` package.
 *
 * Generated JSON is checked with `POST /egress/{id}`. When `autoRepair` is
 * set, a bounded re-prompt runs only for semantic violations: `min` / `max`
 * (`range_violation`), `pattern` (`pattern_mismatch`), `enum`
 * (`enum_violation`), and `compliance_mode` (`undeclared_field`). The text
 * (or tool-call input) written back is the gateway payload, which has
 * already applied declared transforms.
 */

import {
  Client,
  ValidationFailedError,
  type ClientOptions,
  type EgressResponse,
  type Violation,
} from '@nightmoose/contractgate-sdk';

export const AISG_MAX_REPAIR_ATTEMPTS = 5;

/** Violation kinds produced by semantic contract rules, not structural type checks. */
export const SEMANTIC_VIOLATION_KINDS = [
  'range_violation',
  'pattern_mismatch',
  'enum_violation',
  'undeclared_field',
] as const;

export type SemanticViolationKind = (typeof SEMANTIC_VIOLATION_KINDS)[number];

const SEMANTIC_KIND_SET = new Set<string>(SEMANTIC_VIOLATION_KINDS);
const UUID_RE = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
const DEFAULT_BASE_URL = 'https://api.datacontractgate.com';
const FENCE = '```';

export class AiSchemaGateRejectedError extends Error {
  readonly violations: Violation[];
  readonly attempts: number;
  /** Last payload submitted to egress. Not safe to forward when `ok` is false. */
  readonly payload: Record<string, unknown>;

  constructor(message: string, details: { violations: Violation[]; attempts: number; payload: Record<string, unknown> }) {
    super(message);
    this.name = 'AiSchemaGateRejectedError';
    this.violations = details.violations;
    this.attempts = details.attempts;
    this.payload = details.payload;
  }
}

export interface GuardrailOutcome {
  ok: boolean;
  /** Egress payload when `ok` is true. Last submitted object when it is not. */
  payload: Record<string, unknown>;
  violations: Violation[];
  attempts: number;
}

export interface AiSchemaGateGuardrailOptions {
  apiKey: string;
  /** Deployed contract UUID, or a contract name resolved with `GET /contracts`. */
  contractId: string;
  baseUrl?: string;
  /** Inject a `Client` (tests, shared pool). `apiKey` is still required by the options type and ignored when `client` is set. */
  client?: Client;
  version?: string;
  disposition?: 'block' | 'fail' | 'tag';
  /** Default false, matching `LLMGate`: a live egress call. Set true to skip audit and quarantine. */
  dryRun?: boolean;
  /**
   * Re-prompt while semantic rules fail. Each retry counts against
   * `maxRetries` (default 2 extra calls, hard cap 5).
   */
  autoRepair?: boolean;
  maxRetries?: number;
  /** Defaults to `SEMANTIC_VIOLATION_KINDS`. */
  repairOn?: readonly string[];
  /**
   * Middleware throws `AiSchemaGateRejectedError` when the payload still
   * fails. `validateStructuredOutput` returns the outcome instead.
   */
  throwIfRejected?: boolean;
  repairPrompt?: (violations: Violation[]) => string;
  clientOptions?: Partial<Omit<ClientOptions, 'baseUrl' | 'apiKey'>>;
}

export interface LanguageModelGenerateResult {
  content: unknown[];
  [key: string]: unknown;
}

export interface LanguageModelCallParams {
  prompt?: unknown;
  [key: string]: unknown;
}

export interface LanguageModelLike {
  doGenerate: (params: LanguageModelCallParams) => PromiseLike<LanguageModelGenerateResult>;
}

/**
 * Structural `LanguageModelV4Middleware`. Assignable to the AI SDK type
 * without a runtime dependency on `ai`.
 */
export interface AiSchemaGateMiddleware {
  specificationVersion: 'v4';
  wrapGenerate: (options: {
    doGenerate: () => PromiseLike<LanguageModelGenerateResult>;
    doStream: () => PromiseLike<unknown>;
    params: LanguageModelCallParams;
    model: LanguageModelLike;
  }) => Promise<LanguageModelGenerateResult>;
}

type RepairFn = (
  payload: Record<string, unknown>,
  violations: Violation[],
) => Promise<Record<string, unknown>>;

function isRecord(v: unknown): v is Record<string, unknown> {
  return v !== null && typeof v === 'object' && !Array.isArray(v);
}

function clampRetries(n: number): number {
  if (n < 0) return 0;
  if (n > AISG_MAX_REPAIR_ATTEMPTS) {
    console.warn(
      `[aischemagate] maxRetries=${n} exceeds hard cap ${AISG_MAX_REPAIR_ATTEMPTS}; clamping to ${AISG_MAX_REPAIR_ATTEMPTS}`,
    );
    return AISG_MAX_REPAIR_ATTEMPTS;
  }
  return Math.trunc(n);
}

function stripFence(s: string): string {
  const trimmed = s.trim();
  if (trimmed.length < 2 * FENCE.length || !trimmed.startsWith(FENCE) || !trimmed.endsWith(FENCE)) {
    return trimmed;
  }
  let inner = trimmed.slice(FENCE.length, -FENCE.length);
  if (inner.slice(0, 4).toLowerCase() === 'json') inner = inner.slice(4);
  return inner.trim();
}

function parseJsonObject(text: string): Record<string, unknown> | null {
  try {
    const value: unknown = JSON.parse(stripFence(text));
    return isRecord(value) ? value : null;
  } catch {
    return null;
  }
}

function isEgressBody(v: unknown): boolean {
  return isRecord(v) && Array.isArray(v['outcomes']) && 'failed' in v;
}

function egressFromBody(body: unknown): EgressResponse {
  const r = isRecord(body) ? body : {};
  const outcomesRaw = Array.isArray(r['outcomes']) ? r['outcomes'] : [];
  const outcomes = outcomesRaw.map((item, index) => {
    const o = isRecord(item) ? item : {};
    const violationsRaw = Array.isArray(o['violations']) ? o['violations'] : [];
    const violations: Violation[] = violationsRaw.map((entry) => {
      const rec = isRecord(entry) ? entry : {};
      return {
        field: String(rec['field'] ?? ''),
        message: String(rec['message'] ?? ''),
        kind: String(rec['kind'] ?? '') as Violation['kind'],
      };
    });
    return {
      index: typeof o['index'] === 'number' ? o['index'] : index,
      passed: o['passed'] === true,
      violations,
      validation_us: typeof o['validation_us'] === 'number' ? o['validation_us'] : 0,
      action: String(o['action'] ?? ''),
    };
  });
  return {
    total: typeof r['total'] === 'number' ? r['total'] : outcomes.length,
    passed: typeof r['passed'] === 'number' ? r['passed'] : 0,
    failed: typeof r['failed'] === 'number' ? r['failed'] : 0,
    dry_run: r['dry_run'] === true,
    disposition: String(r['disposition'] ?? 'block'),
    resolved_version: String(r['resolved_version'] ?? ''),
    payload: Array.isArray(r['payload']) ? r['payload'] : [],
    outcomes,
  };
}

function defaultRepairPrompt(violations: Violation[]): string {
  const lines = [
    'The previous JSON failed semantic contract validation. Fix every violation and return ONLY the corrected JSON object. No prose and no markdown fence.',
    'Rules that failed are value constraints (minimum, maximum, pattern, enum, or compliance_mode), not the JSON shape.',
  ];
  for (const v of violations) {
    lines.push(`- field '${v.field}' (${v.kind}): ${v.message}`);
  }
  return lines.join('\n');
}

function semanticHit(violations: Violation[], kinds: ReadonlySet<string>): boolean {
  return violations.some((v) => kinds.has(v.kind));
}

function outcomeFromEgress(
  egress: EgressResponse,
  submitted: Record<string, unknown>,
  attempts: number,
): GuardrailOutcome {
  const first = egress.outcomes[0];
  const violations = first !== undefined ? [...first.violations] : [];
  const ok = egress.failed === 0;
  let payload = submitted;
  if (ok && egress.payload.length > 0 && isRecord(egress.payload[0])) {
    payload = egress.payload[0];
  }
  return { ok, payload, violations, attempts };
}

class GateSession {
  readonly #client: Client;
  readonly #contractId: string;
  readonly #version?: string;
  readonly #disposition?: 'block' | 'fail' | 'tag';
  readonly #dryRun: boolean;
  readonly #autoRepair: boolean;
  readonly #maxRetries: number;
  readonly #repairOn: ReadonlySet<string>;
  readonly #repairPrompt: (violations: Violation[]) => string;
  readonly #resolved = new Map<string, string>();

  constructor(opts: AiSchemaGateGuardrailOptions) {
    this.#client =
      opts.client ??
      new Client({
        baseUrl: opts.baseUrl ?? DEFAULT_BASE_URL,
        apiKey: opts.apiKey,
        ...(opts.clientOptions ?? {}),
      });
    this.#contractId = opts.contractId;
    this.#version = opts.version;
    this.#disposition = opts.disposition;
    this.#dryRun = opts.dryRun ?? false;
    this.#autoRepair = opts.autoRepair ?? false;
    this.#maxRetries = clampRetries(opts.maxRetries ?? 2);
    this.#repairOn = new Set(opts.repairOn ?? SEMANTIC_VIOLATION_KINDS);
    this.#repairPrompt = opts.repairPrompt ?? defaultRepairPrompt;
  }

  async resolve(contractId: string): Promise<string> {
    if (UUID_RE.test(contractId)) return contractId;
    let id = this.#resolved.get(contractId);
    if (id === undefined) {
      const rows = await this.#client.listContracts();
      const match =
        rows.find((r) => r.name === contractId) ??
        rows.find((r) => r.name.toLowerCase() === contractId.toLowerCase());
      if (match === undefined) {
        const names = rows.map((r) => r.name).sort().join(', ') || 'none';
        throw new AiSchemaGateRejectedError(
          `No contract named '${contractId}' on this API key. Pass the contract UUID or one of: ${names}.`,
          { violations: [], attempts: 0, payload: {} },
        );
      }
      id = match.id;
      this.#resolved.set(contractId, id);
    }
    return id;
  }

  async egress(payload: Record<string, unknown>): Promise<EgressResponse> {
    try {
      return await this.#client.egress({
        contractId: await this.resolve(this.#contractId),
        events: [payload],
        version: this.#version,
        disposition: this.#disposition,
        dryRun: this.#dryRun,
      });
    } catch (err) {
      if (err instanceof ValidationFailedError && isEgressBody(err.body)) {
        return egressFromBody(err.body);
      }
      throw err;
    }
  }

  async run(initial: Record<string, unknown>, repair?: RepairFn): Promise<GuardrailOutcome> {
    let current = initial;
    const total = this.#autoRepair && repair !== undefined ? this.#maxRetries + 1 : 1;
    let last: GuardrailOutcome = {
      ok: false,
      payload: current,
      violations: [],
      attempts: 0,
    };
    for (let attempt = 1; attempt <= total; attempt++) {
      const egress = await this.egress(current);
      last = outcomeFromEgress(egress, current, attempt);
      if (last.ok) return last;
      const canRepair =
        this.#autoRepair &&
        repair !== undefined &&
        attempt < total &&
        semanticHit(last.violations, this.#repairOn);
      if (!canRepair) return last;
      current = await repair(current, last.violations);
    }
    return last;
  }

  repairHint(violations: Violation[]): string {
    return this.#repairPrompt(violations);
  }
}

interface ExtractedJson {
  payload: Record<string, unknown>;
  write: (result: LanguageModelGenerateResult, payload: Record<string, unknown>) => LanguageModelGenerateResult;
}

function textOf(part: Record<string, unknown>): string | null {
  return typeof part['text'] === 'string' ? part['text'] : null;
}

function toolPayload(part: Record<string, unknown>): Record<string, unknown> | null {
  if (part['type'] !== 'tool-call') return null;
  for (const key of ['input', 'args'] as const) {
    const raw = part[key];
    if (typeof raw === 'string') {
      const parsed = parseJsonObject(raw);
      if (parsed !== null) return parsed;
    } else if (isRecord(raw)) {
      return raw;
    }
  }
  return null;
}

function extractGeneratedJson(result: LanguageModelGenerateResult): ExtractedJson | null {
  const content = result.content;
  const textIndexes: number[] = [];
  const chunks: string[] = [];
  content.forEach((part, index) => {
    if (!isRecord(part) || part['type'] !== 'text') return;
    const text = textOf(part);
    if (text !== null && text.trim().length > 0) {
      textIndexes.push(index);
      chunks.push(text);
    }
  });
  const joined = parseJsonObject(chunks.join(''));
  if (joined !== null && textIndexes.length > 0) {
    const first = textIndexes[0]!;
    return {
      payload: joined,
      write: (source, payload) => {
        const json = JSON.stringify(payload);
        const next = source.content.map((part, index) => {
          if (!textIndexes.includes(index) || !isRecord(part)) return part;
          return { ...part, text: index === first ? json : '' };
        });
        return { ...source, content: next };
      },
    };
  }

  for (let index = 0; index < content.length; index++) {
    const part = content[index];
    if (!isRecord(part)) continue;
    const payload = toolPayload(part);
    if (payload === null) continue;
    return {
      payload,
      write: (source, nextPayload) => {
        const json = JSON.stringify(nextPayload);
        const next = source.content.map((item, i) => {
          if (i !== index || !isRecord(item)) return item;
          const copy: Record<string, unknown> = { ...item };
          if ('input' in copy) copy['input'] = typeof copy['input'] === 'string' || copy['input'] === undefined ? json : nextPayload;
          if ('args' in copy) copy['args'] = typeof copy['args'] === 'string' || copy['args'] === undefined ? json : nextPayload;
          if (!('input' in copy) && !('args' in copy)) copy['input'] = json;
          return copy;
        });
        return { ...source, content: next };
      },
    };
  }
  return null;
}

function appendRepairMessage(params: LanguageModelCallParams, hint: string): LanguageModelCallParams | null {
  if (!Array.isArray(params.prompt)) return null;
  const useString = params.prompt.some((message) => isRecord(message) && typeof message['content'] === 'string');
  const message = useString
    ? { role: 'user', content: hint }
    : { role: 'user', content: [{ type: 'text', text: hint }] };
  return { ...params, prompt: [...params.prompt, message] };
}

function reject(outcome: GuardrailOutcome): never {
  const summary = outcome.violations.map((v) => `${v.field}: ${v.message}`).join('; ') || 'validation failed';
  throw new AiSchemaGateRejectedError(
    `AISchemaGate rejected the structured output after ${outcome.attempts} attempt(s): ${summary}`,
    outcome,
  );
}

/**
 * Validate one structured object with `POST /egress/{id}`.
 *
 * `repair` is called only when a semantic rule fails and `autoRepair` is
 * set. The returned payload on success is the egress payload.
 */
export async function validateStructuredOutput(
  opts: AiSchemaGateGuardrailOptions,
  payload: Record<string, unknown>,
  repair?: RepairFn,
): Promise<GuardrailOutcome> {
  const session = new GateSession(opts);
  const outcome = await session.run(payload, repair);
  if (!outcome.ok && opts.throwIfRejected) reject(outcome);
  return outcome;
}

/**
 * `LanguageModelV4Middleware` that validates `wrapGenerate` JSON output
 * against an AISchemaGate contract.
 *
 * Structured output (`generateText` + `Output.object`) lands in text
 * content. Tool-call `input` / `args` are used when the text is not a JSON
 * object. Streaming is unchanged: use a non-streaming generate call.
 */
export function aischemagateGuardrail(opts: AiSchemaGateGuardrailOptions): AiSchemaGateMiddleware {
  const session = new GateSession(opts);
  const throwIfRejected = opts.throwIfRejected !== false;

  return {
    specificationVersion: 'v4',
    wrapGenerate: async ({ doGenerate, params, model }) => {
      let currentParams = params;
      let result = await doGenerate();
      const extracted = extractGeneratedJson(result);
      if (extracted === null) {
        const outcome: GuardrailOutcome = {
          ok: false,
          payload: {},
          violations: [],
          attempts: 1,
        };
        if (throwIfRejected) {
          throw new AiSchemaGateRejectedError(
            'AISchemaGate could not find a JSON object in the model result. Use structured output or a tool call that returns JSON.',
            outcome,
          );
        }
        return result;
      }

      const repair: RepairFn = async (_payload, violations) => {
        const hint = session.repairHint(violations);
        const next = appendRepairMessage(currentParams, hint);
        if (next === null) {
          throw new AiSchemaGateRejectedError(
            'AISchemaGate could not append a repair prompt. params.prompt must be an array of messages.',
            { violations, attempts: 1, payload: extracted.payload },
          );
        }
        currentParams = next;
        result = await model.doGenerate(next);
        const again = extractGeneratedJson(result);
        if (again === null) {
          throw new AiSchemaGateRejectedError(
            'AISchemaGate repair response was not a JSON object.',
            { violations, attempts: 1, payload: {} },
          );
        }
        return again.payload;
      };

      const outcome = await session.run(extracted.payload, opts.autoRepair ? repair : undefined);
      if (!outcome.ok && throwIfRejected) reject(outcome);
      if (!outcome.ok) return result;
      const finalExtract = extractGeneratedJson(result) ?? extracted;
      return finalExtract.write(result, outcome.payload);
    },
  };
}
