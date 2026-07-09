# HTTP / API Walkthrough

Validate events over the universal HTTP ingest endpoint — anything that can
POST can gate against a contract. Full detail in the
[POST /v1/ingest reference](../v1-ingest-reference.md).

## 1. The contract

The full runnable file is
[`examples/contracts/api/user_events.yaml`](../../examples/contracts/api/user_events.yaml).

```yaml
version: "1.0"
name: "user_events"
ontology:
  entities:
    - name: user_id
      type: string
      required: true
      pattern: "^[a-zA-Z0-9_-]+$"
    - name: event_type
      type: string
      required: true
      enum: ["click", "view", "purchase", "login"]
    - name: timestamp
      type: integer
      required: true
    - name: amount
      type: number
      required: false
      min: 0
```

## 2. The command

Validate locally first with [`cg test`](../cg-test-reference.md), then send to
the live endpoint:

```
cg test --contract examples/contracts/api/user_events.yaml --data events.ndjson
```

```bash
curl -X POST https://app.datacontractgate.com/v1/ingest/{contract_id} \
  -H "X-Api-Key: cg_live_<your_key>" \
  -H "Content-Type: application/x-ndjson" \
  --data-binary @events.ndjson
```

## 3. A passing record

```json
{ "user_id": "u_123", "event_type": "purchase", "timestamp": 1714000000, "amount": 49.99 }
```

```
contract: user_events (v1.0)
  PASS  1
1/1 records passed   validated in 0.1ms
```

Over HTTP the response is `200` with a per-event `results` array; each entry has
`"violations": []` on pass.

## 4. A failing record

`event_type` is not in the allowlist, and `amount` is negative:

```json
{ "user_id": "u_123", "event_type": "delete", "timestamp": 1714000000, "amount": -5 }
```

```
  FAIL  1
record   0  event_type   enum_violation    Field 'event_type' value "delete" not in allowed set: [click, view, purchase, login]
record   0  amount       range_violation   Field 'amount' value -5 is below minimum 0
```

Over HTTP this event lands in the response `results` with its `violations`
populated and is routed to quarantine (unless `dry_run=true`).

## 5. Wire it in

Gate a batch in CI before promotion — `cg test` exits non-zero on any failure:

```
cg test --contract user_events.yaml --data batch.ndjson --quiet || exit 1
```

In an application, POST batches to `/v1/ingest/{contract_id}` and branch on each
result's `violations`. Use `Idempotency-Key` for at-most-once processing and
`dry_run=true` to validate without persisting. See the
[endpoint reference](../v1-ingest-reference.md) for limits and headers.
