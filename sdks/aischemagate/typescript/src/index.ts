/**
 * AISchemaGate — alias over `@nightmoose/contractgate-sdk` for LLM output
 * validation, plus a dependency-light Vercel AI SDK guardrail.
 *
 * RFC-094 §4. The re-exported surface tracks `@nightmoose/contractgate-sdk`.
 * `aischemagateGuardrail` lives here: it calls `POST /egress/{id}` and does
 * not import the `ai` package.
 */

export * from '@nightmoose/contractgate-sdk';

export {
  LLMGate,
  LLMResponseError,
  MAX_RETRIES_CAP,
  extractJson,
} from '@nightmoose/contractgate-sdk/llm';
export type {
  LLMGateResult,
  LLMGateOptions,
  CompletionOptions,
  MessageOptions,
} from '@nightmoose/contractgate-sdk/llm';

export {
  AISG_MAX_REPAIR_ATTEMPTS,
  SEMANTIC_VIOLATION_KINDS,
  AiSchemaGateRejectedError,
  aischemagateGuardrail,
  validateStructuredOutput,
} from './ai.js';
export type {
  AiSchemaGateGuardrailOptions,
  AiSchemaGateMiddleware,
  GuardrailOutcome,
  LanguageModelCallParams,
  LanguageModelGenerateResult,
  LanguageModelLike,
  SemanticViolationKind,
} from './ai.js';

export const AISCHEMAGATE_VERSION = '0.4.0';
