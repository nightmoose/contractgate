/**
 * LLMGate — thin wrapper over existing egress validation for LLM outputs.
 *
 * See `docs/rfcs/094-sdk-llm-output-guard.md`. All validation goes through
 * `client.egress({ contractId, events })`; this module adds response-shape
 * extraction and a bounded auto-repair re-prompt loop. No engine code, no
 * new endpoint. The caller brings their own LLM client (OpenAI, Anthropic,
 * Ollama, vLLM) and binds it on the gate.
 *
 * The TS SDK is async-only (there is no sync `Client`), so a single
 * `LLMGate` class covers both sync and async use. All methods return
 * promises.
 */

import { Client, type ClientOptions } from './client.js';
import { ContractGateError, ValidationFailedError } from './errors.js';
import { egressResponseFromJson, type ContractResponse, type EgressResponse, type Violation } from './types.js';

const UUID_RE = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

/**
 * Return the id of the contract named `name`. The gateway's egress and ingest
 * routes take a contract UUID, but the contract's YAML `name` is what people
 * know, so a name is accepted and looked up once on the key's contract list.
 */
function matchContract(rows: ContractResponse[], name: string): string {
  const match =
    rows.find((r) => r.name === name) ??
    rows.find((r) => r.name.toLowerCase() === name.toLowerCase());
  if (match === undefined) {
    const names = rows.map((r) => r.name).sort().join(', ') || 'none';
    throw new ContractGateError(
      `No contract named '${name}' on this API key. Pass the contract's UUID or one of these names: ${names}.`,
    );
  }
  return match.id;
}

export const MAX_RETRIES_CAP = 5;
const DEFAULT_MAX_RETRIES = 2;
const DEFAULT_BASE_URL = 'https://api.datacontractgate.com';
const FENCE = '```';

/**
 * Remove one outer ```json ... ``` fence from `s` (already trimmed). Plain
 * string checks instead of a regex: LLM output is untrusted, and the
 * lazy-match regex this replaces backtracks polynomially on input like a
 * fence followed by many spaces (CodeQL js/polynomial-redos).
 */
function stripFence(s: string): string {
  if (s.length < 2 * FENCE.length || !s.startsWith(FENCE) || !s.endsWith(FENCE)) return s;
  let inner = s.slice(FENCE.length, -FENCE.length);
  if (inner.slice(0, 4).toLowerCase() === 'json') inner = inner.slice(4);
  return inner.trim();
}

// ---------------------------------------------------------------------------
// Errors + result shape
// ---------------------------------------------------------------------------

export class LLMResponseError extends ContractGateError {}

export interface LLMGateResult {
  ok: boolean;
  /** Egress-sanitized payload — forward this downstream, never the raw response. */
  payload: Record<string, unknown>;
  violations: Violation[];
  attempts: number;
  rawResponse: unknown;
  repairHints: string[];
}

// ---------------------------------------------------------------------------
// extractJson — shape matrix
// ---------------------------------------------------------------------------

/**
 * Pull a JSON object out of common LLM response shapes.
 *
 * Supports OpenAI chat `choices[0].message.content` (JSON mode) and
 * `tool_calls[0].function.arguments`, OpenAI Responses API `output_parsed`,
 * Anthropic `content[*].type === 'tool_use'` → `.input` and the text-block
 * fallback, raw JSON strings (with markdown-fence tolerance), and plain
 * object passthrough.
 *
 * Streaming iterables raise `LLMResponseError`. Unrecognizable shapes raise.
 */
export function extractJson(response: unknown): Record<string, unknown> {
  if (isStreaming(response)) {
    throw new LLMResponseError(
      'streaming responses are not supported in v1 — pass stream=false ' +
        'or call validateOnly(response) with a non-streaming response',
    );
  }

  const openai = tryOpenaiChat(response);
  if (openai !== null) return openai;

  const anthropic = tryAnthropicMessage(response);
  if (anthropic !== null) return anthropic;

  const openaiResponses = tryOpenaiResponses(response);
  if (openaiResponses !== null) return openaiResponses;

  if (typeof response === 'string') {
    return jsonLoadsFenceTolerant(response);
  }

  if (isPlainObject(response)) {
    return response;
  }

  const typeLabel = response === null ? 'null' : typeof response;
  throw new LLMResponseError(
    `unrecognized response shape (type=${typeLabel}); ` +
      'pass an object, a JSON string, or a response from OpenAI / Anthropic',
  );
}

/** `llm` is only needed to call a provider; `validateOnly` works without it. */
function requireLlm<T>(llm: T | null): T {
  if (llm === null) {
    throw new TypeError(
      'completion() and message() need an `llm` client (OpenAI / Anthropic / Ollama / vLLM). ' +
        'Example: new LLMGate({ ..., llm: new OpenAI() }). validateOnly() works without one.',
    );
  }
  return llm;
}

function isEgressBody(v: unknown): boolean {
  return isPlainObject(v) && 'outcomes' in v && 'failed' in v;
}

function isPlainObject(v: unknown): v is Record<string, unknown> {
  return v !== null && typeof v === 'object' && !Array.isArray(v);
}

function isStreaming(response: unknown): boolean {
  if (response === null || response === undefined) return false;
  if (typeof response !== 'object') return false;
  if (Array.isArray(response)) return false;
  // Non-stream responses expose one of these terminal fields.
  const rec = response as Record<string, unknown>;
  if ('choices' in rec || 'content' in rec || 'output_parsed' in rec) return false;
  const asyncIt = (response as { [Symbol.asyncIterator]?: unknown })[Symbol.asyncIterator];
  const syncIt = (response as { [Symbol.iterator]?: unknown })[Symbol.iterator];
  return typeof asyncIt === 'function' || typeof syncIt === 'function';
}

function getField(obj: unknown, key: string): unknown {
  if (obj === null || obj === undefined) return undefined;
  if (typeof obj !== 'object') return undefined;
  return (obj as Record<string, unknown>)[key];
}

function tryOpenaiChat(response: unknown): Record<string, unknown> | null {
  const choices = getField(response, 'choices');
  if (!Array.isArray(choices) || choices.length === 0) return null;
  const msg = getField(choices[0], 'message');
  if (msg === undefined || msg === null) return null;

  // Tool call takes precedence over message content — structured args are
  // more trustworthy than free-form JSON mode output.
  const toolCalls = getField(msg, 'tool_calls');
  if (Array.isArray(toolCalls) && toolCalls.length > 0) {
    const fn = getField(toolCalls[0], 'function');
    const args = getField(fn, 'arguments');
    if (typeof args === 'string') return jsonLoadsFenceTolerant(args);
    if (isPlainObject(args)) return args;
  }

  const content = getField(msg, 'content');
  if (typeof content === 'string' && content.trim().length > 0) {
    return jsonLoadsFenceTolerant(content);
  }
  return null;
}

function tryAnthropicMessage(response: unknown): Record<string, unknown> | null {
  const content = getField(response, 'content');
  if (!Array.isArray(content) || content.length === 0) return null;
  for (const block of content) {
    if (getField(block, 'type') === 'tool_use') {
      const input = getField(block, 'input');
      if (isPlainObject(input)) return input;
    }
  }
  // Fallback: text block containing a JSON string.
  for (const block of content) {
    if (getField(block, 'type') === 'text') {
      const text = getField(block, 'text');
      if (typeof text === 'string' && text.trim().length > 0) {
        try {
          return jsonLoadsFenceTolerant(text);
        } catch {
          continue;
        }
      }
    }
  }
  return null;
}

function tryOpenaiResponses(response: unknown): Record<string, unknown> | null {
  const out = getField(response, 'output_parsed');
  return isPlainObject(out) ? out : null;
}

function jsonLoadsFenceTolerant(s: string): Record<string, unknown> {
  const stripped = stripFence(s.trim());
  let obj: unknown;
  try {
    obj = JSON.parse(stripped);
  } catch (err) {
    throw new LLMResponseError(
      `response is not valid JSON: ${err instanceof Error ? err.message : String(err)}`,
    );
  }
  if (!isPlainObject(obj)) {
    const kind = Array.isArray(obj) ? 'array' : typeof obj;
    throw new LLMResponseError(`response JSON is not an object (got ${kind})`);
  }
  return obj;
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

function clampRetries(n: number): number {
  if (n < 0) return 0;
  if (n > MAX_RETRIES_CAP) {
    // eslint-disable-next-line no-console
    console.warn(
      `[contractgate] maxRetries=${n} exceeds hard cap ${MAX_RETRIES_CAP}; clamping to ${MAX_RETRIES_CAP}`,
    );
    return MAX_RETRIES_CAP;
  }
  return Math.trunc(n);
}

function defaultRepairPrompt(violations: Violation[]): string {
  const lines = [
    'The previous JSON was rejected by schema validation. Fix the ' +
      'violations below and return ONLY the corrected JSON object. No prose, ' +
      'no markdown fence.',
  ];
  for (const v of violations) {
    lines.push(`- field '${v.field}': ${v.message}`);
  }
  return lines.join('\n');
}

function resultFromEgress(
  egress: EgressResponse,
  rawResponse: unknown,
  attempts: number,
  hints: string[],
): LLMGateResult {
  const first = egress.outcomes[0];
  const violations = first !== undefined ? [...first.violations] : [];
  let payloadRecord: Record<string, unknown> = {};
  if (egress.payload.length > 0 && isPlainObject(egress.payload[0])) {
    payloadRecord = egress.payload[0];
  }
  return {
    ok: egress.failed === 0,
    payload: payloadRecord,
    violations,
    attempts,
    rawResponse,
    repairHints: [...hints],
  };
}

function appendUserMessage(messages: unknown[], hint: string): unknown[] {
  return [...messages, { role: 'user', content: hint }];
}

// ---------------------------------------------------------------------------
// LLMGate
// ---------------------------------------------------------------------------

export interface LLMGateOptions {
  apiKey: string;
  contractId: string;
  /**
   * Caller-bound LLM client (OpenAI / Anthropic / Ollama / vLLM). Needed for
   * `completion` and `message`; `validateOnly` works without one.
   */
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  llm?: any;
  baseUrl?: string;
  version?: string;
  disposition?: 'block' | 'fail' | 'tag';
  /** Default 2 (3 total provider calls). Hard-capped at 5; higher warns + clamps. */
  maxRetries?: number;
  repairPrompt?: (violations: Violation[]) => string;
  /** Inject an existing `Client` (useful for tests or shared connection pooling). */
  client?: Client;
  /** Forwarded to the auto-constructed `Client`; ignored when `client` is injected. */
  clientOptions?: Partial<Omit<ClientOptions, 'baseUrl' | 'apiKey'>>;
}

export interface CompletionOptions {
  model: string;
  messages: unknown[];
  autoRepair?: boolean;
  /** OpenAI chat params forwarded unchanged (temperature, response_format, tools, ...). */
  [key: string]: unknown;
}

export interface MessageOptions {
  model: string;
  messages: unknown[];
  maxTokens?: number;
  autoRepair?: boolean;
  /** Anthropic params forwarded unchanged (system, temperature, tools, tool_choice, ...). */
  [key: string]: unknown;
}

type ProviderCall = (messages: unknown[]) => Promise<unknown>;

/**
 * Validate LLM structured output against a deployed contract.
 *
 * Construct once per contract. Call `completion` for OpenAI-protocol
 * clients (OpenAI, vLLM, Ollama OpenAI-compat), `message` for Anthropic,
 * or `validateOnly` for a response you already have.
 */
export class LLMGate {
  readonly #llm: {
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    chat?: { completions?: { create?: (...args: any[]) => Promise<unknown> } };
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    messages?: { create?: (...args: any[]) => Promise<unknown> };
  } | null;
  readonly #contractId: string;
  readonly #version?: string;
  readonly #disposition?: 'block' | 'fail' | 'tag';
  readonly #maxRetries: number;
  readonly #repairPrompt: (violations: Violation[]) => string;
  readonly #client: Client;
  readonly #resolved = new Map<string, string>();

  constructor(opts: LLMGateOptions) {
    this.#llm = opts.llm ?? null;
    this.#contractId = opts.contractId;
    this.#version = opts.version;
    this.#disposition = opts.disposition;
    this.#maxRetries = clampRetries(opts.maxRetries ?? DEFAULT_MAX_RETRIES);
    this.#repairPrompt = opts.repairPrompt ?? defaultRepairPrompt;
    this.#client =
      opts.client ??
      new Client({
        baseUrl: opts.baseUrl ?? DEFAULT_BASE_URL,
        apiKey: opts.apiKey,
        ...(opts.clientOptions ?? {}),
      });
  }

  /**
   * OpenAI-protocol wrapper. Calls `llm.chat.completions.create(...)`.
   * Works unchanged for vLLM and OpenAI-compatible Ollama endpoints.
   */
  async completion(opts: CompletionOptions): Promise<LLMGateResult> {
    const { model, messages, autoRepair = false, stream, ...rest } = opts;
    if (stream === true) {
      throw new LLMResponseError(
        'streaming responses are not supported in v1 — pass stream=false',
      );
    }
    const llm = requireLlm(this.#llm);
    const createFn = llm.chat?.completions?.create;
    if (typeof createFn !== 'function') {
      throw new TypeError(
        'bound `llm` client is missing chat.completions.create — expected an ' +
          'OpenAI-protocol client. For Anthropic, call `.message()` instead.',
      );
    }
    const chatCompletions = llm.chat!.completions!;
    const call: ProviderCall = (current) =>
      createFn.call(chatCompletions, { model, messages: current, ...rest });
    return this.#runLoop(messages, call, autoRepair);
  }

  /**
   * Anthropic-protocol wrapper. Calls `llm.messages.create(...)`.
   */
  async message(opts: MessageOptions): Promise<LLMGateResult> {
    const { model, messages, maxTokens = 1024, autoRepair = false, stream, ...rest } = opts;
    if (stream === true) {
      throw new LLMResponseError(
        'streaming responses are not supported in v1 — pass stream=false',
      );
    }
    const llm = requireLlm(this.#llm);
    const createFn = llm.messages?.create;
    if (typeof createFn !== 'function') {
      throw new TypeError(
        'bound `llm` client is missing messages.create — expected an ' +
          'Anthropic-protocol client. For OpenAI, call `.completion()` instead.',
      );
    }
    const anthropicMessages = llm.messages!;
    const call: ProviderCall = (current) =>
      createFn.call(anthropicMessages, {
        model,
        messages: current,
        max_tokens: maxTokens,
        ...rest,
      });
    return this.#runLoop(messages, call, autoRepair);
  }

  async #resolve(contractId: string): Promise<string> {
    if (UUID_RE.test(contractId)) return contractId;
    let id = this.#resolved.get(contractId);
    if (id === undefined) {
      id = matchContract(await this.#client.listContracts(), contractId);
      this.#resolved.set(contractId, id);
    }
    return id;
  }

  /**
   * The gateway answers 422 when every event in the batch fails, which is
   * always the case for a gate's single-event batch. The body is still a full
   * egress response with per-field violations, so a failed validation is a
   * result (`ok: false`), not an exception. Any other 422 (e.g. a deprecated
   * version pin) has no `outcomes` and is rethrown.
   */
  async #egress(payload: Record<string, unknown>, dryRun?: boolean): Promise<EgressResponse> {
    try {
      return await this.#client.egress({
        contractId: await this.#resolve(this.#contractId),
        events: [payload],
        version: this.#version,
        disposition: this.#disposition,
        dryRun,
      });
    } catch (err) {
      if (err instanceof ValidationFailedError && isEgressBody(err.body)) {
        return egressResponseFromJson(err.body);
      }
      throw err;
    }
  }

  /**
   * Validate a response the caller already produced (no provider call).
   */
  async validateOnly(
    response: unknown,
    opts: { dryRun?: boolean } = {},
  ): Promise<LLMGateResult> {
    const payload = extractJson(response);
    const egress = await this.#egress(payload, opts.dryRun);
    return resultFromEgress(egress, response, 1, []);
  }

  /**
   * Return the RFC-004-transformed form of a prompt payload. One-liner over
   * `client.ingest({ dryRun: true })`.
   */
  async maskPrompt(
    payload: Record<string, unknown>,
    opts: { promptContractId: string },
  ): Promise<Record<string, unknown>> {
    const resp = await this.#client.ingest({
      contractId: await this.#resolve(opts.promptContractId),
      events: [payload],
      dryRun: true,
    });
    const first = resp.results[0];
    if (first !== undefined && isPlainObject(first.transformed_event)) {
      return first.transformed_event;
    }
    return payload;
  }

  async #runLoop(
    messages: unknown[],
    call: ProviderCall,
    autoRepair: boolean,
  ): Promise<LLMGateResult> {
    let current = [...messages];
    const hints: string[] = [];
    let last: LLMGateResult | null = null;
    const totalAttempts = autoRepair ? this.#maxRetries + 1 : 1;

    for (let attempt = 1; attempt <= totalAttempts; attempt++) {
      const resp = await call(current);
      let payload: Record<string, unknown>;
      try {
        payload = extractJson(resp);
      } catch (err) {
        if (!autoRepair || attempt >= totalAttempts) {
          throw err;
        }
        const hint =
          'The previous response was not valid JSON. Return ONLY a JSON ' +
          'object matching the required schema. No prose, no markdown fence.';
        hints.push(hint);
        current = appendUserMessage(current, hint);
        continue;
      }

      const egress = await this.#egress(payload);
      last = resultFromEgress(egress, resp, attempt, hints);

      if (egress.failed === 0 || !autoRepair || attempt >= totalAttempts) {
        return last;
      }
      const hint = this.#repairPrompt(last.violations);
      hints.push(hint);
      current = appendUserMessage(current, hint);
    }
    // Unreachable: the loop above always returns on the last iteration.
    if (last === null) {
      throw new Error('internal: loop exited without producing a result');
    }
    return last;
  }
}
