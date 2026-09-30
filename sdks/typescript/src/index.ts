export { Client } from './client.js';
export type { ClientOptions } from './client.js';

export { Contract } from './contract.js';
export type {
  CompiledContract,
  ContractData,
  FieldDefinition,
  FieldType,
  GlossaryEntry,
  MaskStyle,
  MetricDefinition,
  MetricType,
  Transform,
  TransformKind,
} from './contract.js';

export {
  AuthError,
  BadRequestError,
  ConflictError,
  ConnectionError,
  ContractCompileError,
  ContractGateError,
  HTTPError,
  NotFoundError,
  ServerError,
  ValidationFailedError,
} from './errors.js';

export type {
  AuditEntry,
  BatchIngestResponse,
  ContractResponse,
  EgressOutcome,
  EgressResponse,
  IngestionStats,
  IngestEventResult,
  ValidationResult,
  Violation,
  ViolationKind,
  VersionResponse,
  VersionSummary,
} from './types.js';

export { validate } from './validator.js';
