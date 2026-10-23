/**
 * AISchemaGate helpers for Vercel AI SDK structured-output flows.
 *
 * `createAISchemaGateMiddleware` matches the `LanguageModelV4Middleware`
 * shape (`specificationVersion: 'v4'`, `wrapGenerate`) using structural
 * types, so this package does not depend on the `ai` package at runtime.
 * `validateOrRepairObject` wraps `generateObject`, `generateText` with an
 * object output, or any async function that returns JSON.
 *
 * Validation is `client.egress` — `POST /egress/{contract_id}`. There is
 * no separate HTTP `client.validate`; the in-process `validate()` helper
 * on a compiled contract is a different call.
 */

import {
  extractJson,
  LLMResponseError,
  MAX_RETRIES_CAP,
} from '@nightmoose/contractgate-sdk/llm';

/** Hard cap, shared with `LLMGate`. Higher values are clamped. */
export const AISCHEMAGATE_MAX_RETRIES = MAX_RETRIES_CAP;

export interface StructuredViolation {
  field: string;
  message: string;
  kind: string;
  expected?: string;
  received?: unknown;
  suggestion?: string;
}

/**
 * Anything with `egress()`. The `Client` class from
 * `@nightmoose/aischemagate` satisfies this.
 */
export interface EgressClient {
  egress(opts: {
    contractId: string;
    events: unknown;
    version?: string;
    disposition?: string;
    dryRun?: boolean;
    timeoutMs?: number;
  }): Promise<unknown>;
}

export interface AISchemaGateMiddlewareOptions {
  client: EgressClient;
  contractId: string;
  /** Repair attempts after the first generation. Default 2. Capped at 5. */
  maxRetries?: number;
  version?: string;
  /** Default `fail`, so one bad object rejects the call instead of being dropped. */
  disposition?: 'block' | 'fail' | 'tag';
  /** Default false. Set true to skip audit and quarantine writes during repair. */
  dryRun?: boolean;
}

export interface RepairContext {
  /** 1-based. The first call has no `repairPrompt`. */
  attempt: number;
  repairPrompt?: string;
  violations: StructuredViolation[];
}

export interface ValidateOrRepairOptions {
  contractId: string;
  /**
   * Return JSON text, a plain object, a Vercel AI SDK `generateObject`
   * result (`{ object, finishReason }`), or a `generateText` result with
   * `{ output, text }`.
   */
  generate: (ctx: RepairContext) => Promise<unknown>;
  maxRetries?: number;
  version?: string;
  disposition?: 'block' | 'fail' | 'tag';
  dryRun?: boolean;
}

export interface ValidateOrRepairResult {
  ok: boolean;
  /** Egress-sanitized object when `ok` is true; otherwise the last candidate. */
  object: Record<string, unknown>;
  violations: StructuredViolation[];
  attempts: number;
  repairPrompts: string[];
}

/**
 * Thrown by the middleware when every repair attempt still violates the
 * contract. `validateOrRepairObject` returns `{ ok: false }` instead.
 */
export class AISchemaGateRepairError extends Error {
  readonly violations: StructuredViolation[];
  readonly attempts: number;
  readonly payload: Record<string, unknown> | null;
  readonly repairPrompts: string[];

  constructor(
    message: string,
    details: {
      violations: StructuredViolation[];
      attempts: number;
      payload: Record<string, unknown> | null;
      repairPrompts: string[];
    },
  ) {
    super(message);
    this.name = 'AISchemaGateRepairError';
    this.violations = details.violations;
    this.attempts = details.attempts;
    this.payload = details.payload;
    this.repairPrompts = details.repairPrompts;
  }
}

interface EgressCall {
  contractId: string;
  version?: string;
  disposition?: 'block' | 'fail' | 'tag';
  dryRun?: boolean;
}

interface Slot {
  kind: 'legacy-text' | 'text' | 'tool-input' | 'tool-args';
  index: number;
}

function clampRetries(n: number | undefined): number {
  const raw = n ?? 2;
  if (!Number.isFinite(raw) || raw < 0) return 0;
  return Math.min(Math.trunc(raw), AISCHEMAGATE_MAX_RETRIES);
}

function isRecord(v: unknown): v is Record<string, unknown> {
  return v !== null && typeof v === 'object' && !Array.isArray(v);
}

function optString(v: unknown): string | undefined {
  return typeof v === 'string' && v.length > 0 ? v : undefined;
}

function violationFrom(raw: unknown): StructuredViolation | null {
  if (!isRecord(raw)) return null;
  return {
    field: String(raw['field'] ?? ''),
    message: String(raw['message'] ?? ''),
    kind: String(raw['kind'] ?? ''),
    expected: optString(raw['expected']),
    received: 'received' in raw ? raw['received'] : undefined,
    suggestion: optString(raw['suggestion']),
  };
}

function isEgressBody(v: unknown): v is Record<string, unknown> {
  return isRecord(v) && 'failed' in v && ('outcomes' in v || 'payload' in v);
}

function violationsOf(body: Record<string, unknown>): StructuredViolation[] {
  const outcomes = Array.isArray(body['outcomes']) ? body['outcomes'] : [];
  const out: StructuredViolation[] = [];
  for (const outcome of outcomes) {
    if (!isRecord(outcome)) continue;
    const list = Array.isArray(outcome['violations']) ? outcome['violations'] : [];
    for (const item of list) {
      const parsed = violationFrom(item);
      if (parsed) out.push(parsed);
    }
  }
  return out;
}

function parseEgress(raw: unknown): { ok: boolean; violations: StructuredViolation[]; payload: Record<string, unknown> | null } {
  if (!isEgressBody(raw)) {
    throw new TypeError(
      'AISchemaGate egress response is missing failed/outcomes. Expected POST /egress/{contract_id}.',
    );
  }
  const failed = typeof raw['failed'] === 'number' ? raw['failed'] : 0;
  const violations = violationsOf(raw);
  const payloadList = Array.isArray(raw['payload']) ? raw['payload'] : [];
  const first = payloadList.find(isRecord) ?? null;
  const ok = failed === 0 && violations.length === 0;
  return { ok, violations, payload: first };
}

async function callEgress(
  client: EgressClient,
  call: EgressCall,
  object: Record<string, unknown>,
): Promise<{ ok: boolean; violations: StructuredViolation[]; payload: Record<string, unknown> | null }> {
  const opts = {
    contractId: call.contractId,
    events: [object],
    version: call.version,
    disposition: call.disposition ?? 'fail',
    dryRun: call.dryRun,
  };
  try {
    return parseEgress(await client.egress(opts));
  } catch (err) {
    if (isRecord(err) && err['status'] === 422 && isEgressBody(err['body'])) {
      return parseEgress(err['body']);
    }
    throw err;
  }
}

/**
 * Repair text fed back to the model. Includes field, kind, message, and
 * the gateway's expected / received / suggestion fields when present, so
 * min, max, pattern, enum, and compliance_mode failures are explicit.
 */
export function buildRepairPrompt(violations: StructuredViolation[]): string {
  const lines = [
    'The previous JSON failed AISchemaGate validation. The contract enforces semantic constraints (min, max, pattern, enum, and compliance_mode for undeclared fields).',
    'Fix every violation and return ONLY the corrected JSON object. No prose and no markdown fence.',
  ];
  for (const v of violations) {
    const bits = [
      `field ${JSON.stringify(v.field)}`,
      `kind ${v.kind || 'violation'}`,
      v.message,
    ];
    if (v.expected) bits.push(`expected ${v.expected}`);
    if (v.received !== undefined) bits.push(`received ${JSON.stringify(v.received)}`);
    if (v.suggestion) bits.push(`suggestion: ${v.suggestion}`);
    if (v.kind === 'undeclared_field' || v.kind === 'unknown_field') {
      bits.push('compliance_mode rejects this field; remove it');
    }
    lines.push(`- ${bits.join(' — ')}`);
  }
  return lines.join('\n');
}

function asObject(value: unknown): Record<string, unknown> | null {
  if (typeof value === 'string') {
    try {
      const parsed = extractJson(value);
      return isRecord(parsed) ? parsed : null;
    } catch (err) {
      if (err instanceof LLMResponseError) return null;
      throw err;
    }
  }
  if (isRecord(value)) return value;
  return null;
}

/** Unwrap `generateObject` / `generateText({ output })` results. */
function unwrapGenerated(value: unknown): unknown {
  if (!isRecord(value)) return value;
  if (
    isRecord(value['object']) &&
    ('finishReason' in value || 'usage' in value || 'response' in value)
  ) {
    return value['object'];
  }
  if (
    isRecord(value['output']) &&
    ('finishReason' in value || 'usage' in value || 'text' in value)
  ) {
    return value['output'];
  }
  return value;
}

function notJsonViolation(): StructuredViolation {
  return {
    field: '$',
    kind: 'type_mismatch',
    message: 'The model response was not a JSON object.',
    suggestion: 'Return a single JSON object and no surrounding prose.',
  };
}

interface Candidate {
  object: Record<string, unknown> | null;
  slot: Slot | null;
  violation: StructuredViolation | null;
}

function candidateFromGenerated(value: unknown): Candidate {
  const object = asObject(unwrapGenerated(value));
  if (!object) return { object: null, slot: null, violation: notJsonViolation() };
  return { object, slot: null, violation: null };
}

function candidateFromModelResult(result: unknown): Candidate {
  if (typeof result === 'string') {
    const object = asObject(result);
    return object
      ? { object, slot: null, violation: null }
      : { object: null, slot: null, violation: notJsonViolation() };
  }
  if (!isRecord(result)) {
    return { object: null, slot: null, violation: notJsonViolation() };
  }
  if (typeof result['text'] === 'string') {
    const object = asObject(result['text']);
    if (object) return { object, slot: { kind: 'legacy-text', index: 0 }, violation: null };
  }
  const content = result['content'];
  if (Array.isArray(content)) {
    for (let index = 0; index < content.length; index++) {
      const part = content[index];
      if (!isRecord(part)) continue;
      if (part['type'] === 'text' && typeof part['text'] === 'string') {
        const object = asObject(part['text']);
        if (object) return { object, slot: { kind: 'text', index }, violation: null };
      }
      if (part['type'] === 'tool-call') {
        if (typeof part['input'] === 'string') {
          const object = asObject(part['input']);
          if (object) return { object, slot: { kind: 'tool-input', index }, violation: null };
        } else if (isRecord(part['input'])) {
          return { object: part['input'], slot: { kind: 'tool-input', index }, violation: null };
        }
        if (isRecord(part['args'])) {
          return { object: part['args'], slot: { kind: 'tool-args', index }, violation: null };
        }
      }
    }
  }
  return { object: null, slot: null, violation: notJsonViolation() };
}

function applyPayload(result: unknown, payload: Record<string, unknown>, slot: Slot | null): unknown {
  if (!isRecord(result) || slot === null) return result;
  const json = JSON.stringify(payload);
  if (slot.kind === 'legacy-text') {
    return { ...result, text: json };
  }
  if (!Array.isArray(result['content'])) return result;
  const content = result['content'].map((part, index) => {
    if (index !== slot.index || !isRecord(part)) return part;
    if (slot.kind === 'text') return { ...part, text: json };
    if (slot.kind === 'tool-input') {
      return typeof part['input'] === 'string' ? { ...part, input: json } : { ...part, input: payload };
    }
    if (slot.kind === 'tool-args') return { ...part, args: payload };
    return part;
  });
  return { ...result, content };
}

function withRepairMessage(params: unknown, prompt: string): Record<string, unknown> {
  const base = isRecord(params) ? { ...params } : {};
  const existing = Array.isArray(base['prompt']) ? [...base['prompt']] : [];
  const usesParts = existing.some((message) => isRecord(message) && Array.isArray(message['content']));
  existing.push(
    usesParts
      ? { role: 'user', content: [{ type: 'text', text: prompt }] }
      : { role: 'user', content: prompt },
  );
  base['prompt'] = existing;
  return base;
}

interface LoopOutcome {
  ok: boolean;
  object: Record<string, unknown> | null;
  violations: StructuredViolation[];
  attempts: number;
  repairPrompts: string[];
  slot: Slot | null;
  raw: unknown;
}

async function repairLoop(
  client: EgressClient,
  call: EgressCall,
  maxRetries: number,
  produce: (ctx: RepairContext, params: unknown) => Promise<{ raw: unknown; candidate: Candidate }>,
  initialParams: unknown,
): Promise<LoopOutcome> {
  const repairPrompts: string[] = [];
  let violations: StructuredViolation[] = [];
  let repairPrompt: string | undefined;
  let params = initialParams;
  let last: LoopOutcome = {
    ok: false,
    object: null,
    violations: [],
    attempts: 0,
    repairPrompts,
    slot: null,
    raw: null,
  };

  const total = maxRetries + 1;
  for (let attempt = 1; attempt <= total; attempt++) {
    const ctx: RepairContext = { attempt, repairPrompt, violations };
    const { raw, candidate } = await produce(ctx, params);
    if (candidate.violation || !candidate.object) {
      violations = [candidate.violation ?? notJsonViolation()];
      last = {
        ok: false,
        object: null,
        violations,
        attempts: attempt,
        repairPrompts,
        slot: candidate.slot,
        raw,
      };
    } else {
      const egress = await callEgress(client, call, candidate.object);
      const object = egress.ok && egress.payload ? egress.payload : candidate.object;
      violations = egress.violations;
      last = {
        ok: egress.ok,
        object,
        violations,
        attempts: attempt,
        repairPrompts,
        slot: candidate.slot,
        raw: egress.ok ? applyPayload(raw, object, candidate.slot) : raw,
      };
      if (egress.ok) return last;
    }
    if (attempt >= total) return last;
    repairPrompt = buildRepairPrompt(violations);
    repairPrompts.push(repairPrompt);
    params = withRepairMessage(params, repairPrompt);
  }
  return last;
}

/**
 * `LanguageModelV4Middleware`-compatible guard. On success the generated
 * text or tool-call JSON is replaced with the egress-sanitized payload.
 * On exhaustion throws `AISchemaGateRepairError`.
 */
export function createAISchemaGateMiddleware(options: AISchemaGateMiddlewareOptions): {
  specificationVersion: 'v4';
  wrapGenerate: (options: {
    doGenerate: () => PromiseLike<unknown>;
    doStream?: () => PromiseLike<unknown>;
    params?: unknown;
    model?: { doGenerate?: (params: unknown) => PromiseLike<unknown> };
  }) => Promise<unknown>;
} {
  const maxRetries = clampRetries(options.maxRetries);
  const call: EgressCall = {
    contractId: options.contractId,
    version: options.version,
    disposition: options.disposition,
    dryRun: options.dryRun,
  };

  return {
    specificationVersion: 'v4',
    async wrapGenerate(mw) {
      let params = mw.params;
      const outcome = await repairLoop(
        options.client,
        call,
        maxRetries,
        async (_ctx, current) => {
          const raw =
            current === mw.params
              ? await mw.doGenerate()
              : await retryGenerate(mw.model, current);
          return { raw, candidate: candidateFromModelResult(raw) };
        },
        params,
      );
      if (!outcome.ok || !outcome.object) {
        throw new AISchemaGateRepairError(
          `AISchemaGate rejected the model output after ${outcome.attempts} attempt(s).`,
          {
            violations: outcome.violations,
            attempts: outcome.attempts,
            payload: outcome.object,
            repairPrompts: outcome.repairPrompts,
          },
        );
      }
      return outcome.raw;
    },
  };
}

async function retryGenerate(
  model: { doGenerate?: (params: unknown) => PromiseLike<unknown> } | undefined,
  params: unknown,
): Promise<unknown> {
  if (typeof model?.doGenerate !== 'function') {
    throw new TypeError(
      'AISchemaGate repair needs model.doGenerate on wrapGenerate. Pass the language model through the middleware options.',
    );
  }
  return model.doGenerate(params);
}

/**
 * Validate JSON from any async generator. Retries up to `maxRetries` by
 * passing a repair prompt that quotes the structured violations.
 * Returns `{ ok: false }` when the retries are exhausted.
 */
export async function validateOrRepairObject(
  client: EgressClient,
  options: ValidateOrRepairOptions,
): Promise<ValidateOrRepairResult> {
  const maxRetries = clampRetries(options.maxRetries);
  const call: EgressCall = {
    contractId: options.contractId,
    version: options.version,
    disposition: options.disposition,
    dryRun: options.dryRun,
  };
  const outcome = await repairLoop(
    client,
    call,
    maxRetries,
    async (ctx) => {
      const raw = await options.generate(ctx);
      return { raw, candidate: candidateFromGenerated(raw) };
    },
    undefined,
  );
  return {
    ok: outcome.ok,
    object: outcome.object ?? {},
    violations: outcome.violations,
    attempts: outcome.attempts,
    repairPrompts: outcome.repairPrompts,
  };
}
