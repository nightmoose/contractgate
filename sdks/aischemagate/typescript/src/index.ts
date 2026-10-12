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

export const AISCHEMAGATE_VERSION = '0.1.0';
