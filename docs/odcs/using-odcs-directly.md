# Using an ODCS v3.x contract directly

DataContractGate reads Open Data Contract Standard (ODCS) v3.x documents as
they are. No `ontology` block is needed. These all accept an ODCS file:

- CLI: `contractgate validate`, `synth`, `test`, `enforce`, `push`, `deploy`
- API: `POST /contracts` (create), `POST /contracts/{id}/versions`, draft
  edits, `POST /contracts/deploy`, `POST /contracts/synth`,
  `POST /playground/validate`, `POST /contracts/import`
- Python: `Contract.from_yaml(...)` in `contractgate-py`

A document counts as ODCS when it has no `ontology` and carries
`apiVersion: v3.x`, `kind: DataContract`, or a `schema` list. Native
DataContractGate YAML is read exactly as before. A stored contract is always
kept as native YAML; an ODCS document is converted once, on the way in.

## How it maps

| ODCS | DataContractGate |
| --- | --- |
| `dataProduct`, then `schema[0].name`, then root `name` | contract name |
| `version` (string or number) | version |
| `description` as text or `{purpose, usage, limitations}` | description |
| `schema[0].properties[]` | fields |
| `logicalType` (or `physicalType` when absent: `varchar(20)`, `BIGINT`, `NUMERIC(10,2)`) | field type; `map` becomes an object |
| `required`, `primaryKey` | required |
| `unique`, single-column `primaryKey` | batch uniqueness rule |
| `logicalTypeOptions` `pattern`, `format`, `minLength`, `maxLength`, `minimum`, `maximum` | the matching field rules |
| property `enum` (plain values or `{value: ...}` items) | allowed values |
| property `quality` library metrics `nullValues`, `missingValues`, `duplicateValues`, `invalidValues` | quality rules |
| nested names (`a.b`) and `items` | nested objects and array items |

Unknown ODCS fields are ignored.

## What does not map

- Only `schema[0]` is read. Other schema objects are reported as a note
  (`contractgate validate` prints it) and skipped.
- `relationships` and foreign keys, composite primary key uniqueness.
- Dataset-level `quality` (for example `rowCount`), `sql` and `custom` quality
  checks from other engines.
- `servers`, `team`, `roles`, `slaProperties`, `support`, `price`,
  `customProperties` (except the `x-cg-*` ones DataContractGate writes),
  `authoritativeDefinitions`, `classification`, `tags`, `examples`.
- Map key and value types (a `map` field accepts any object).

## Errors

- A Data Contract Specification (datacontract.com, `dataContractSpecification:`)
  file is a different format: convert it first, for example with
  `datacontract export --format odcs`.
- `apiVersion` other than `v3.*` is rejected with the supported range.
- Missing `version`, a missing `schema` array, or a `schema[0]` with no
  properties each get a message naming the problem.
