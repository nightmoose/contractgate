import yaml from 'js-yaml';
import { ContractCompileError } from './errors.js';
import type { ValidationResult } from './types.js';
// validate is imported at runtime; validator.ts uses only "import type" from this
// module so there is no cycle in the compiled JavaScript.
import { validate as runValidate } from './validator.js';

// ---------------------------------------------------------------------------
// FieldType
// ---------------------------------------------------------------------------

export type FieldType = 'string' | 'integer' | 'float' | 'boolean' | 'object' | 'array' | 'any';

const FIELD_TYPE_DISPLAY: Record<FieldType, string> = {
  string: 'String',
  integer: 'Integer',
  float: 'Float',
  boolean: 'Boolean',
  object: 'Object',
  array: 'Array',
  any: 'Any',
};

export function fieldTypeDisplay(ft: FieldType): string {
  return FIELD_TYPE_DISPLAY[ft];
}

function parseFieldType(raw: unknown): FieldType {
  if (typeof raw !== 'string') {
    throw new ContractCompileError(`field type must be a string, got ${typeof raw}`);
  }
  const normalized = raw.toLowerCase() === 'number' ? 'float' : raw.toLowerCase();
  const valid: FieldType[] = ['string', 'integer', 'float', 'boolean', 'object', 'array', 'any'];
  if (!valid.includes(normalized as FieldType)) {
    throw new ContractCompileError(`unknown field type: ${JSON.stringify(raw)}`);
  }
  return normalized as FieldType;
}

// ---------------------------------------------------------------------------
// Transform (RFC-004 — declared only, not run locally)
// ---------------------------------------------------------------------------

export type TransformKind = 'mask' | 'hash' | 'drop' | 'redact';
export type MaskStyle = 'opaque' | 'format_preserving';

export interface Transform {
  kind: TransformKind;
  style?: MaskStyle;
}

function parseTransform(raw: unknown): Transform {
  if (raw === null || typeof raw !== 'object' || Array.isArray(raw)) {
    throw new ContractCompileError('transform must be a mapping');
  }
  const r = raw as Record<string, unknown>;
  const kindRaw = r['kind'];
  if (typeof kindRaw !== 'string') {
    throw new ContractCompileError('transform.kind is required and must be a string');
  }
  const validKinds: TransformKind[] = ['mask', 'hash', 'drop', 'redact'];
  const kind = kindRaw.toLowerCase() as TransformKind;
  if (!validKinds.includes(kind)) {
    throw new ContractCompileError(`unknown transform kind: ${JSON.stringify(kindRaw)}`);
  }
  let style: MaskStyle | undefined;
  if (r['style'] != null) {
    const validStyles: MaskStyle[] = ['opaque', 'format_preserving'];
    const s = String(r['style']).toLowerCase() as MaskStyle;
    if (!validStyles.includes(s)) {
      throw new ContractCompileError(`unknown mask style: ${JSON.stringify(r['style'])}`);
    }
    style = s;
  }
  return { kind, style };
}

// ---------------------------------------------------------------------------
// FieldDefinition
// ---------------------------------------------------------------------------

export interface FieldDefinition {
  name: string;
  fieldType: FieldType;
  required: boolean;
  pattern?: string;
  allowedValues?: unknown[];
  min?: number;
  max?: number;
  minLength?: number;
  maxLength?: number;
  properties?: FieldDefinition[];
  items?: FieldDefinition;
  transform?: Transform;
}

function parseFieldDefinition(raw: unknown): FieldDefinition {
  if (raw === null || typeof raw !== 'object' || Array.isArray(raw)) {
    throw new ContractCompileError(`entity must be a mapping, got ${typeof raw}`);
  }
  const r = raw as Record<string, unknown>;
  const name = r['name'];
  if (typeof name !== 'string' || !name) {
    throw new ContractCompileError('entity.name is required');
  }
  const fieldType = parseFieldType(r['type']);

  let properties: FieldDefinition[] | undefined;
  if (r['properties'] != null) {
    if (!Array.isArray(r['properties'])) {
      throw new ContractCompileError(`${name}.properties must be a list`);
    }
    properties = r['properties'].map(parseFieldDefinition);
  }

  let items: FieldDefinition | undefined;
  if (r['items'] != null) {
    items = parseFieldDefinition(r['items']);
  }

  let transform: Transform | undefined;
  if (r['transform'] != null) {
    transform = parseTransform(r['transform']);
  }

  return {
    name,
    fieldType,
    required: r['required'] !== false,
    pattern: typeof r['pattern'] === 'string' ? r['pattern'] : undefined,
    allowedValues: Array.isArray(r['enum']) ? r['enum'] : undefined,
    min: typeof r['min'] === 'number' ? r['min'] : undefined,
    max: typeof r['max'] === 'number' ? r['max'] : undefined,
    minLength: typeof r['min_length'] === 'number' ? Math.trunc(r['min_length']) : undefined,
    maxLength: typeof r['max_length'] === 'number' ? Math.trunc(r['max_length']) : undefined,
    properties,
    items,
    transform,
  };
}

// ---------------------------------------------------------------------------
// MetricDefinition
// ---------------------------------------------------------------------------

export type MetricType = 'integer' | 'float';

export interface MetricDefinition {
  name: string;
  field?: string;
  metricType?: MetricType;
  formula?: string;
  min?: number;
  max?: number;
}

function parseMetricDefinition(raw: unknown): MetricDefinition {
  if (raw === null || typeof raw !== 'object' || Array.isArray(raw)) {
    throw new ContractCompileError('metric must be a mapping');
  }
  const r = raw as Record<string, unknown>;
  const name = r['name'];
  if (typeof name !== 'string' || !name) {
    throw new ContractCompileError('metric.name is required');
  }
  let metricType: MetricType | undefined;
  if (r['type'] != null) {
    const mt = String(r['type']).toLowerCase();
    if (mt !== 'integer' && mt !== 'float') {
      throw new ContractCompileError(`unknown metric type: ${JSON.stringify(r['type'])}`);
    }
    metricType = mt as MetricType;
  }
  return {
    name,
    field: typeof r['field'] === 'string' ? r['field'] : undefined,
    metricType,
    formula: typeof r['formula'] === 'string' ? r['formula'] : undefined,
    min: typeof r['min'] === 'number' ? r['min'] : undefined,
    max: typeof r['max'] === 'number' ? r['max'] : undefined,
  };
}

// ---------------------------------------------------------------------------
// GlossaryEntry
// ---------------------------------------------------------------------------

export interface GlossaryEntry {
  field: string;
  description: string;
  constraints?: string;
  synonyms?: string[];
}

function parseGlossaryEntry(raw: unknown): GlossaryEntry {
  if (raw === null || typeof raw !== 'object' || Array.isArray(raw)) {
    throw new ContractCompileError('glossary entry must be a mapping');
  }
  const r = raw as Record<string, unknown>;
  const fieldName = r['field'] ?? r['term'];
  const description = r['description'] ?? r['definition'];
  if (typeof fieldName !== 'string' || !fieldName) {
    throw new ContractCompileError('glossary.field (or .term) is required');
  }
  if (typeof description !== 'string' || !description) {
    throw new ContractCompileError('glossary.description (or .definition) is required');
  }
  let synonyms: string[] | undefined;
  if (r['synonyms'] != null) {
    if (!Array.isArray(r['synonyms'])) {
      throw new ContractCompileError('glossary.synonyms must be a list');
    }
    synonyms = r['synonyms'].map(String);
  }
  return {
    field: fieldName,
    description,
    constraints: typeof r['constraints'] === 'string' ? r['constraints'] : undefined,
    synonyms,
  };
}

// ---------------------------------------------------------------------------
// Contract + CompiledContract
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// ODCS v3.x input
// ---------------------------------------------------------------------------

type Json = Record<string, unknown>;

const isMap = (v: unknown): v is Json =>
  v !== null && typeof v === 'object' && !Array.isArray(v);

// ODCS logicalType -> field type. This SDK has no date or timestamp type, so
// those read as strings (the shape is not checked locally).
const ODCS_LOGICAL: Record<string, string> = {
  integer: 'integer', long: 'integer', int: 'integer',
  number: 'float', double: 'float', float: 'float', decimal: 'float',
  boolean: 'boolean', bool: 'boolean',
  object: 'object', record: 'object', struct: 'object', map: 'object',
  array: 'array', list: 'array',
  any: 'any',
};

// Used only when a property has no logicalType.
const ODCS_PHYSICAL: Record<string, string> = {
  int: 'integer', integer: 'integer', bigint: 'integer', smallint: 'integer',
  tinyint: 'integer', long: 'integer', short: 'integer',
  float: 'float', double: 'float', real: 'float', decimal: 'float',
  numeric: 'float', number: 'float',
  boolean: 'boolean', bool: 'boolean',
  struct: 'object', record: 'object', object: 'object', map: 'object',
  json: 'object', jsonb: 'object', variant: 'object',
  array: 'array', list: 'array',
};

/** A document with no `ontology` and an `apiVersion: v3*`, `kind: DataContract`
 * or a `schema` list. */
function isOdcs(r: Json): boolean {
  if ('ontology' in r) return false;
  const api = r['apiVersion'];
  return (
    (typeof api === 'string' && api.startsWith('v3')) ||
    r['kind'] === 'DataContract' ||
    Array.isArray(r['schema'])
  );
}

function odcsDescription(v: unknown): string | undefined {
  if (typeof v === 'string') return v;
  if (isMap(v)) {
    for (const k of ['purpose', 'usage', 'limitations']) {
      if (typeof v[k] === 'string') return v[k] as string;
    }
  }
  return undefined;
}

function odcsEnum(v: unknown): unknown[] | undefined {
  if (!Array.isArray(v)) return undefined;
  const out = v
    .map((i) => (isMap(i) ? i['value'] : i))
    .filter((i) => ['string', 'number', 'boolean'].includes(typeof i));
  return out.length ? out : undefined;
}

function odcsProp(p: Json): Json {
  const logical = p['logicalType'];
  let type: string;
  if (typeof logical === 'string') {
    type = ODCS_LOGICAL[logical] ?? 'string';
  } else {
    const base = String(p['physicalType'] ?? '').toLowerCase().split(/[(< ]/)[0] ?? '';
    type = ODCS_PHYSICAL[base] ?? 'string';
  }
  const opts = isMap(p['logicalTypeOptions']) ? p['logicalTypeOptions'] : {};
  const ent: Json = {
    name: String(p['name']),
    type,
    required: p['required'] === true || p['primaryKey'] === true,
  };
  const en = odcsEnum(p['enum']) ?? odcsEnum(opts['enum']);
  if (en) ent['enum'] = en;
  for (const [src, dst] of [
    ['pattern', 'pattern'],
    ['minLength', 'min_length'],
    ['maxLength', 'max_length'],
    ['minimum', 'min'],
    ['maximum', 'max'],
  ] as const) {
    if (opts[src] !== undefined && opts[src] !== null) ent[dst] = opts[src];
  }
  return ent;
}

/** Map an ODCS v3.x document (`schema[0].properties[]`) to the native contract
 * mapping. Unknown ODCS fields are ignored. */
function odcsToNative(r: Json): Json {
  const api = r['apiVersion'];
  if (typeof api === 'string' && !api.startsWith('v3')) {
    throw new ContractCompileError(`unsupported ODCS apiVersion ${JSON.stringify(api)}; supported: v3.x`);
  }
  let version = r['version'];
  if (typeof version === 'number') version = String(version);
  if (typeof version !== 'string' || !version) {
    throw new ContractCompileError("ODCS document is missing the required 'version' field");
  }
  const schema = r['schema'];
  if (!Array.isArray(schema) || schema.length === 0 || !isMap(schema[0])) {
    throw new ContractCompileError(
      "ODCS document has no 'schema' array; fields are read from schema[].properties[]",
    );
  }
  const s0 = schema[0];
  const props = s0['properties'];
  if (!Array.isArray(props) || props.length === 0) {
    throw new ContractCompileError(
      `ODCS schema[0] (${JSON.stringify(s0['name'])}) has no properties; nothing to validate`,
    );
  }
  // Same order as the Rust importer: dataProduct, schema[0].name, root name.
  const name =
    [r['dataProduct'], s0['name'], r['name']].find(
      (v): v is string => typeof v === 'string' && v.trim() !== '',
    ) ?? 'imported_contract';
  const entities = props.filter((p): p is Json => isMap(p) && !!p['name']).map(odcsProp);
  const native: Json = { version, name, ontology: { entities } };
  const desc = odcsDescription(r['description']) ?? odcsDescription(s0['description']);
  if (desc) native['description'] = desc;
  return native;
}

export interface ContractData {
  version: string;
  name: string;
  description?: string;
  complianceMode: boolean;
  entities: FieldDefinition[];
  glossary: GlossaryEntry[];
  metrics: MetricDefinition[];
}

export interface CompiledContract {
  contract: ContractData;
  patterns: Map<string, RegExp>;
  declaredTopLevelFields: Set<string>;
  validate(event: unknown): ValidationResult;
}

export class Contract {
  private constructor(private readonly data: ContractData) {}

  static fromYaml(source: string): Contract {
    let raw: unknown;
    try {
      raw = yaml.load(source);
    } catch (e) {
      throw new ContractCompileError(`invalid YAML: ${e}`);
    }
    if (raw === null || typeof raw !== 'object' || Array.isArray(raw)) {
      throw new ContractCompileError('contract YAML must be a mapping at the top level');
    }
    let r = raw as Record<string, unknown>;
    if (isOdcs(r)) r = odcsToNative(r);

    const version = r['version'];
    const name = r['name'];
    if (typeof version !== 'string' || !version) {
      throw new ContractCompileError('contract.version is required');
    }
    if (typeof name !== 'string' || !name) {
      throw new ContractCompileError('contract.name is required');
    }

    // `ontology` is optional; a contract may have none.
    const ontologyRaw = r['ontology'] ?? {};
    if (ontologyRaw === null || typeof ontologyRaw !== 'object' || Array.isArray(ontologyRaw)) {
      throw new ContractCompileError('contract.ontology must be a mapping');
    }
    const entitiesRaw = (ontologyRaw as Record<string, unknown>)['entities'] ?? [];
    if (!Array.isArray(entitiesRaw)) {
      throw new ContractCompileError('contract.ontology.entities must be a list');
    }
    const entities = entitiesRaw.map(parseFieldDefinition);

    const glossary = Array.isArray(r['glossary'])
      ? r['glossary'].map(parseGlossaryEntry)
      : [];
    const metrics = Array.isArray(r['metrics'])
      ? r['metrics'].map(parseMetricDefinition)
      : [];

    return new Contract({
      version,
      name,
      description: odcsDescription(r['description']),
      complianceMode: r['compliance_mode'] === true,
      entities,
      glossary,
      metrics,
    });
  }

  compile(): CompiledContract {
    const patterns = new Map<string, RegExp>();
    compileFieldPatterns(this.data.entities, '', patterns);
    validateTransformTypes(this.data.entities, '');

    const declaredTopLevelFields = this.data.complianceMode
      ? new Set(this.data.entities.map((e) => e.name))
      : new Set<string>();

    const compiled: CompiledContract = {
      contract: this.data,
      patterns,
      declaredTopLevelFields,
      validate(event: unknown): ValidationResult {
        return runValidate(compiled, event);
      },
    };
    return compiled;
  }
}

// ---------------------------------------------------------------------------
// Compile helpers
// ---------------------------------------------------------------------------

function compileFieldPatterns(
  fields: FieldDefinition[],
  prefix: string,
  out: Map<string, RegExp>,
): void {
  for (const f of fields) {
    const path = prefix ? `${prefix}.${f.name}` : f.name;
    if (f.pattern != null) {
      try {
        out.set(path, new RegExp(f.pattern));
      } catch (e) {
        throw new ContractCompileError(
          `Invalid regex ${JSON.stringify(f.pattern)} for field ${JSON.stringify(path)}: ${e}`,
        );
      }
    }
    if (f.fieldType === 'object' && f.properties) {
      compileFieldPatterns(f.properties, path, out);
    }
  }
}

function validateTransformTypes(fields: FieldDefinition[], prefix: string): void {
  for (const f of fields) {
    const path = prefix ? `${prefix}.${f.name}` : f.name;
    if (f.transform != null && f.fieldType !== 'string') {
      throw new ContractCompileError(
        `Field '${path}' declares a PII transform but has type '${fieldTypeDisplay(f.fieldType)}' — transforms are only supported on string fields. If this field holds PII, change its type to 'string'.`,
      );
    }
    if (f.fieldType === 'object' && f.properties) {
      validateTransformTypes(f.properties, path);
    }
  }
}
