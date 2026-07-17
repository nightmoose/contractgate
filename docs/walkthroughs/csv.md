# CSV / File Walkthrough

CSV is a two-step surface: **infer** a draft contract from a sample file, then
**validate** rows against it. ContractGate generates the contract for you — you
review it and tighten it. Full detail in the
[CSV inference reference](../csv-inference-reference.md).

## 1. The contract

Start from a sample CSV
([`examples/contracts/csv/signups.csv`](../../examples/contracts/csv/signups.csv)):

```csv
user_id,plan,signup_ts,mrr
u_1001,pro,1714000000,49.99
u_1002,free,1714000100,0
u_1003,enterprise,1714000200,499.00
```

Inference produces a draft contract — it detects types, marks columns
`required: false` if any sampled row is empty, and collapses low-cardinality
string columns (≤10 distinct) into an `enum`. For the file above `plan`
collapses to `enum: [pro, free, enterprise]`:

```yaml
version: "1.0"
name: "signups"
ontology:
  entities:
    - name: user_id
      type: string
      required: true
    - name: plan
      type: string
      required: true
      enum: ["pro", "free", "enterprise"]
    - name: signup_ts
      type: integer
      required: true
    - name: mrr
      type: float
      required: true
      min: 0
```

> The draft is a starting point. Review the inferred `enum`/`required`/`min`
> and tighten before promoting to stable. The reviewed contract used for the
> rest of this walkthrough is saved at
> [`examples/contracts/csv/signups.yaml`](../../examples/contracts/csv/signups.yaml).

## 2. The command

Infer the contract from the CSV, then validate rows against it:

```bash
curl -X POST https://your-instance/contracts/infer/csv \
  -H "Content-Type: application/json" \
  -d '{ "name": "signups", "csv_content": "<your-csv>" }' \
  | jq -r .yaml_content > signups.yaml
```

```
cg test --contract signups.yaml --data signups.ndjson
```

> `cg test` takes JSON/NDJSON, not raw CSV — convert rows to JSON objects
> (one per line) before testing, or POST them to `/v1/ingest`.

## 3. A passing record

[`examples/contracts/csv/pass.json`](../../examples/contracts/csv/pass.json):

```json
{ "user_id": "u_1001", "plan": "pro", "signup_ts": 1714000000, "mrr": 49.99 }
```

```
contract: signups (v1.0)
  PASS  1
1/1 records passed   validated in 0.1ms
```

## 4. A failing record

`plan` is outside the inferred allowlist and `mrr` is negative
([`examples/contracts/csv/fail.json`](../../examples/contracts/csv/fail.json)):

```json
{ "user_id": "u_9999", "plan": "trial", "signup_ts": 1714000500, "mrr": -10 }
```

```
  FAIL  1
record   0  plan   enum_violation    Field 'plan' value "trial" not in allowed set: ["pro", "free", "enterprise"]
record   0  mrr    range_violation   Field 'mrr' value -10 is below minimum 0
```

This is the moment to decide: is `trial` a real new plan (widen the contract) or
bad data (keep the gate)? That decision is exactly what the contract captures.

## 5. Wire it in

Re-infer when the source schema legitimately changes; otherwise gate new files
in CI:

```
cg test --contract signups.yaml --data new_signups.ndjson --quiet || exit 1
```

For continuous ingest, POST rows (as JSON) to
[`/v1/ingest/{contract_id}`](../v1-ingest-reference.md).
