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
| property `name` | the field name, kept literally (`Rate.x` and `Amount [A]` are one field each, not a path) |
| nested `properties` on an object, and `items` | nested objects and array items |

Unknown ODCS fields are ignored.

A property name is the column name. Dots and brackets stay in it. DataContractGate
does not split `Rate.x` into an object `Rate` with a field `x`. A real nested
object is a property with `logicalType: object` and its own `properties` list.
Native contracts are unchanged: a field `customer` of type object with a
property `address` is still addressed as `customer.address`. When a dotted
ODCS name is missing as a literal key, validation also accepts the same path
as nested JSON, so an older nested payload still matches.

Properties are optional unless `required: true` (or `primaryKey: true`).
Synthetic rows leave those fields out on some rows. A number field is a
number whenever it is present.

To shape synthetic values on a number or integer property, add a custom property named `x-cg-distribution`:

```yaml
- name: amount
  logicalType: number
  customProperties:
    - property: x-cg-distribution
      value: { type: normal, mean: 120, sd: 25 }
```

`type` is `uniform`, `normal`, `lognormal` (`mean` and `sd` of the underlying normal) or `exponential` (`rate`). Details: [synthetic data reference](../synthetic-data-reference.md#value-distributions). A bad value is an import error that names the property.

## Choosing a schema object

An ODCS file can describe several tables or topics in `schema[]`. A
DataContractGate contract covers one of them. The first is used unless you pick
one by `name` (`physicalName` and `id` also match):

```sh
contractgate validate --schema-object customers contracts/shop.odcs.yaml
contractgate synth --schema-object customers contracts/shop.odcs.yaml
```

`--schema-object` is on `validate`, `synth`, `test` and `enforce`. The API takes
`schema_object` on `POST /contracts/synth` (with `yaml_content`) and on
`POST /contracts/import`. An unknown name is an error that lists the names in
the file. A native contract ignores it.

The note that lists the ignored objects (and any composite key or relationships
note) is printed by the CLI and returned as `notes` in the JSON response of
`POST /contracts/synth` and `POST /contracts/import`. `notes` is left out when
there is nothing to say. Passing `--schema-object` (or `schema_object` in the
API) means you already chose the object, so that ignored-objects note is left
out. A composite key or relationships note is still printed when it applies.
To cover several objects, import each one as its own contract.

## What does not map

- One schema object is read per contract (see below). The others are skipped
  and listed in a note.
- `relationships` and foreign keys. A contract validates one object's events
  and has no lookup into another, so these are not enforced. A note says so.
- Uniqueness of a composite primary key. Each key column is made required, but
  uniqueness rules cover one field, so the combination is not checked. A note
  says so.
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
