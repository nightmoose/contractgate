/**
 * AISchemaGate — alias over `@nightmoose/contractgate-sdk` for LLM output
 * validation.
 *
 * RFC-094 §4. Pure re-export surface. No behavior of its own. Breaking
 * changes are shipped in `@nightmoose/contractgate-sdk`; this alias
 * tracks the pinned floor via its dependency range.
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

export const AISCHEMAGATE_VERSION = '0.4.0';

export {
  AISCHEMAGATE_MAX_RETRIES,
  AISchemaGateRepairError,
  buildRepairPrompt,
  createAISchemaGateMiddleware,
  validateOrRepairObject,
} from './vercel-ai.js';
export type {
  AISchemaGateMiddlewareOptions,
  EgressClient,
  RepairContext,
  StructuredViolation,
  ValidateOrRepairOptions,
  ValidateOrRepairResult,
} from './vercel-ai.js';
