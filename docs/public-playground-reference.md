# Anonymous playground

A visitor can validate one event against a contract YAML without an account. The check compiles the YAML, runs the same engine as ingest, and stores nothing: no contract row, no audit row, no quarantine row.

The dashboard page is `/playground`. It works signed out. After a result, Sign up is the next step. A signed-in session uses `POST /playground/validate` instead, which can also load contracts already saved for that account.

## Request

`POST /public/playground/validate`

```json
{
  "yaml_content": "version: \"1.0\"\nname: user_events\nontology:\n  entities:\n    - name: user_id\n      type: string\n      required: true\n",
  "event": { "user_id": "alice" }
}
```

No `Authorization` header and no `x-api-key`. A `contract_id` in the body is ignored. The route cannot read another tenant's contracts.

## Response

`200` with `passed`, `violations`, `validation_us`, `transformed_event`, and `disposition`.

| `disposition` | Meaning |
|---|---|
| `pass` | The event would be accepted. |
| `quarantine` | The event failed. Ingest would quarantine it. This is the default `on_fail`. |
| `drop` | The event failed and the contract says `on_fail: drop`. Nothing would be stored. |

`422` is invalid contract YAML. `413` is a body over 64 KiB. `429` is the per-IP limit, with `Retry-After` and `X-RateLimit-Limit`, `X-RateLimit-Remaining`, and `X-RateLimit-Reset`.

## Limits

One request per second per client IP, with a burst of 8. On Fly the address is `Fly-Client-IP`. The catalog's 30 requests/second limiter is a different bucket. The trusted SSR token does not skip this one.

The Rust `regex` crate matches in linear time. The body cap bounds how much YAML one request can ask the parser to read.
