# Joint example: one ODCS v3.1 `orders` contract, files via CsvPath, events via DCG

**Purpose.** One standard ODCS v3.1.0 data contract (`orders.odcs.yaml`) that a CsvPath user can run on
**files** (CSV batches) and a DataContractGate (DCG) user can run on **events** (streaming/API ingest),
with no translation layer. It uses **only standard ODCS fields**: no `x-` extensions and no `customProperties`.

Files:
- `orders.odcs.yaml`: the contract (ODCS `apiVersion: v3.1.0`)
- `orders_sample.csv`: 20 good rows plus 5 bad rows (header + 25 data rows)
- `validate.py` / `enforce.py`: helper scripts for schema validation and CsvPath enforcement
- `official-odcs-json-schema-v3.1.0.json`: from bitol-io/open-data-contract-standard (byte-identical to the copy bundled with CsvPath)
- `validate.out`, `csvpath-enforce.out`, `dcg-import-*.out`: captured run output

## Fields

| Field | logicalType | Required | Constraint (standard ODCS location) |
|---|---|---|---|
| order_id | string | yes, `unique`, `primaryKey` | `logicalTypeOptions.pattern: ^ORD-[0-9]{8}$`; property quality `library/duplicateValues mustBe 0` |
| customer_id | string | yes | pattern `^CUST-[0-9]{6}$`; quality `library/nullValues mustBe 0` |
| order_ts | timestamp | yes | `logicalTypeOptions.format: yyyy-MM-dd'T'HH:mm:ss'Z'` |
| status | string | yes | quality `library/invalidValues`, `arguments.validValues: [pending, paid, shipped, delivered, cancelled]` |
| currency | string | yes | `invalidValues` validValues `[USD, EUR, GBP]` |
| amount | number | yes | `minimum: 0` |
| quantity | integer | yes | `minimum: 1`, `maximum: 100` |
| email | string | yes | `format: email` + simple pattern; `classification: restricted` |
| ship_country | string | yes | pattern `^[A-Z]{2}$`, `minLength/maxLength: 2` |
| discount_pct | number | no | `minimum: 0`, `maximum: 100` |
| sales_channel | string | no | `invalidValues` validValues `[web, mobile, store]` |

Schema-level quality: `library/rowCount mustBeGreaterThan 0`.

**Why enums are `invalidValues`/`validValues` and not `enum:`.** A property-level `enum: [{value: …}]`
(as in CsvPath's how-to, which targets v3.2.0) is **rejected by the v3.1.0 JSON Schema** ("Unevaluated
properties are not allowed ('enum' was unexpected)"). In v3.1 the standard way to say "allowed values"
is a library quality rule, `metric: invalidValues` with `arguments.validValues`. CsvPath converts that
to `in(...)`. Also, v3.1 `team.members[]` has no `email` field, so the owner address goes in `username`
(it's an example.com address).

## Bad rows (file line numbers, header = line 1)

| Line | order_id | Rule broken | Bad value |
|---|---|---|---|
| 22 | ORD-123 | order_id `pattern` | `ORD-123` (needs 8 digits) |
| 23 | ORD-00000022 | status `invalidValues` (enum) | `refunded` |
| 24 | ORD-00000023 | amount `minimum: 0` | `-5.00` |
| 25 | ORD-00000024 | quantity `maximum: 100` | `500` |
| 26 | ORD-00000025 | email `format: email` / pattern | `not-an-email` |

Each bad row breaks exactly one rule. Lines 2–21 are valid.

## Validate with CsvPath (files)

Environment: `csvpath==0.0.630` in the existing Python 3.12 venv `/tmp/csvp` (from earlier work in
`/workspace/cg-csvpath/CSVPATH-ODCS.md`; `pip install csvpath` needs Python ≤ 3.12).

```bash
cd /workspace/odcs-joint
/tmp/csvp/bin/python validate.py   # CsvPath OdcsContractLoader.validate + from_path, and the official schema
/tmp/csvp/bin/python enforce.py    # OdcsConverter -> csvpath statement -> CsvPath().collect() on the CSV
```

Results:
- CsvPath `OdcsContractLoader.validate` (bundled v3.1.0 schema): **PASS**, no shim
- CsvPath `OdcsContractLoader.from_path`: **PASS**
- Official ODCS v3.1.0 JSON Schema (jsonschema): **PASS**
- Conversion skipped-features report: `[]`, so every rule converted
- Enforcement: **20/25 rows match. All 5 bad rows rejected** (lines 22–26), and each was caught by its own rule
  (regex on order_id, `in()` on status, decimal bound on amount, integer bound on quantity, email check).

Note: the standalone converter runs in `print, no-raise, no-fail` mode, so count matched lines to get pass/fail
(`enforce.py` lists the rows that were rejected). CsvPath writes `config/ logs/ archive/ …` folders into the working directory when it runs.

## Validate / import with DCG (events)

Local gateway `http://localhost:8080` (dev no-auth, demo org header `x-org-id: cccccccc-cccc-cccc-cccc-cccccccccccc`).

```bash
# New contract
python3 -c "import json;print(json.dumps({'odcs_yaml':open('orders.odcs.yaml').read()}))" > /tmp/imp_new.json
curl -s -H 'x-org-id: cccccccc-cccc-cccc-cccc-cccccccccccc' -H 'content-type: application/json' \
  -X POST http://localhost:8080/contracts/import --data @/tmp/imp_new.json
# -> HTTP 500 {"error":"Database error: a required value is missing (not-null violation)"}

# As a new version of an existing contract
#   payload: {"odcs_yaml": <yaml, version 3.0.0-joint>, "target_contract_id": "<existing id>"}
# -> HTTP 201, draft, import_source=odcs_stripped, requires_review=true
```

### DCG caveats (current state, before the in-flight `feat/odcs-strict` fix)
- **New-contract import returns 500** (not-null violation). Importing onto an existing `target_contract_id` works.
- **Kept:** all 11 field names, `required` flags, types (`number` → float, `integer` → integer).
- **Dropped:** every constraint. `pattern`, `minimum`/`maximum`, `minLength`/`maxLength`, and `format` come back `null`,
  because Mode B import reads constraints only from `customProperties` `x-cg-*`, not from `logicalTypeOptions`.
- **Dropped:** every quality rule (`quality: []`). DCG maps only its own `type: completeness|validity|freshness|uniqueness`,
  not standard `type: library` + `metric`, so the enums written as `invalidValues/validValues` are lost too.
- `timestamp` becomes `string` (no timestamp type or format check). `unique`/`primaryKey` are not kept.
- Object `description` is ignored. DCG used the schema-level string description instead, and the contract name stayed as the target contract's.

What the DCG fix needs for this contract to be enforced on events the same way: read `logicalTypeOptions`
(pattern, minimum, maximum, minLength, maxLength, format), map `library` quality (`invalidValues.validValues` → enum,
`duplicateValues`/`unique` → uniqueness, `nullValues` → completeness, `rowCount` → ignore or batch-only), accept
object `description` and top-level `name`, fix the new-contract 500, and (ideally) export the same standard fields.
