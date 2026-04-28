# ContractGate Starter Templates

Three copy-and-modify contract templates covering the most common pilot shapes.
No registry, no UI — just copy a file, rename it, and edit the fields.

## Quick start

1. Copy a starter into your repo's `contracts/` directory:
   ```
   cp contracts/starters/rest_event.yaml contracts/my_api_events.yaml
   ```
2. Rename the `name` field and edit `ontology.entities` to match your domain.
3. Validate locally (requires RFC-014 CLI):
   ```
   contractgate validate contracts/my_api_events.yaml
   ```
4. Push to the gateway:
   ```
   contractgate push contracts/my_api_events.yaml
   ```

## Starters

| File | Shape | Use when |
|------|-------|----------|
| `rest_event.yaml` | HTTP request log | Instrumenting a REST API or API gateway |
| `kafka_event.yaml` | Kafka message metadata | Validating producer output on a Kafka topic |
| `dbt_model.yaml` | dbt model row | Enforcing row-level contracts on a dbt model |

## Field reference

Each starter uses the [locked semantic contract format](../../CLAUDE.md):

- `version` — always `"1.0"` for v1 contracts.
- `ontology.entities` — the field list. Each entry has `name`, `type` (`string` / `integer` / `number`), `required`, and optional constraints (`pattern`, `enum`, `min`, `max`).
- `glossary` — human-readable descriptions and constraint prose for each field.
- `metrics` — optional formula-based metrics computed over the event stream.

## Tips

- Keep field names snake_case.
- Use `required: false` for optional fields; omit them from passing events entirely — the validator accepts missing optional fields.
- `pattern` values are Rust regex strings (RE2-compatible subset).
- `enum` values are case-sensitive string literals.
- The demo seeder (`make stack-up-demo`) publishes all three starters and posts synthetic events through each — useful for verifying your gateway + dashboard are wired up before adding your own contracts.
