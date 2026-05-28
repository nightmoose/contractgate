# RFC-064 — Kafka Connect SMT: Dynamic Contract Reload + DLQ Routing

**Status:** Draft
**Date:** 2026-05-27 (rewritten 2026-05-27 from earlier enterprise-gated draft)
**Branch:** `nightly-maintenance-<date>-rfc064-smt-reload-dlq`
**Addresses:** Frequent feedback that the SMT requires a task restart on contract change and only supports a single DLQ.
**Depends on:** none (ships in the existing single-module `confluent-connector/`)

> Two community-tier improvements to the Kafka Connect SMT: dynamic
> contract reload without task restart, and per-violation DLQ routing
> rules. Both ship free, both opt-in via config, both land in the
> existing `confluent-connector/` module — no Maven restructure
> required.

---

## Why community-tier (not enterprise)

The earlier draft of this RFC gated both features behind an enterprise
license. That was the wrong line to draw:

- **Dynamic reload is table stakes** for anyone running the SMT in
  production. Gating it pushes serious users to alternatives or to
  forking the community SMT.
- **DLQ routing is "anyone running Kafka properly needs this,"** not
  "big-company enterprise needs this." Routing failures to different
  topics by severity or type is a common Kafka pattern; the SMT
  shouldn't be the friction point.

A real enterprise tier should differentiate on multi-tenant fleet
management, RBAC across SMT instances, compliance report generation —
not on per-SMT operational quality of life. Ship these as community,
keep enterprise gating for genuinely "big-company" features once
inbound demand validates which ones.

---

## Feature 1: Dynamic Contract Reload

### Today

`ContractGateValidator` reads its config (including the contract
reference) once at task start via `configure(Map<String,?> props)`.
Updating a contract requires bouncing the Connect task — disruptive on
busy clusters.

### Design

Add an optional background poller per SMT instance. When enabled, the
poller:

1. Every `contractgate.reload.poll.ms` (default 30000, min 5000), GETs
   the current version of the configured contract from the Rust
   gateway.
2. Compares against the cached version hash.
3. On change: fetches the new contract YAML, parses + validates it
   locally, swaps the in-task `Contract` reference via `AtomicReference`.
4. Logs INFO with old → new version. Emits Connect metrics
   (`contractgate.reload.success` counter,
   `contractgate.reload.failure` counter,
   `contractgate.reload.current_version` gauge).
5. On parse/validation failure: keeps the old contract, logs WARN with
   the reason, increments the failure counter. **Never** swaps to a
   broken contract.

### Server-side requirement

The SMT needs a way to fetch the current contract version + body. The
existing contract endpoints from [RFC-028](028-contract-queryability.md)
already cover the body. Verify during implementation whether there's a
cheap "what's the current version hash" check; if not, add one
(`HEAD /v1/contracts/:name` returning ETag, or
`GET /v1/contracts/:name/version`). That server-side endpoint is a
small change and ships in the same nightly.

### Config (new keys)

| Key | Default | Description |
|---|---|---|
| `contractgate.reload.enabled` | `false` | Enable hot reload. Opt-in to preserve current behavior. |
| `contractgate.reload.poll.ms` | `30000` | Polling interval. Minimum 5000 (enforced at config-parse time). |
| `contractgate.reload.failure.action` | `warn` | One of `warn` (keep old contract, log) or `fail-task` (mark the Connect task failed; useful when contract integrity is a hard requirement). |

When `enabled=false` (default), behavior is byte-identical to today.

### Implementation

Lives in `confluent-connector/src/main/java/io/datacontractgate/connect/smt/reload/`:

```
reload/
├── DynamicContractReloader.java       background thread + polling loop
└── ContractVersionCheck.java          HTTP version probe + body fetch
```

`ContractGateValidator` swaps its `Contract` field to an `AtomicReference`
(was a final field). The reloader, if enabled, calls `contract.set(...)`
on successful reload. `apply(SourceRecord)` reads via `contract.get()`
on every record — single volatile read, no contention with the reloader.

Reloader lifecycle:

- Started from `ContractGateValidator.configure(...)` when
  `reload.enabled=true`.
- Stopped from `ContractGateValidator.close()` (Connect calls this on
  task shutdown / config change).
- One reloader instance per SMT instance per task. Doesn't share across
  tasks — Connect tasks are already cheap and isolated.

---

## Feature 2: Per-Violation DLQ Routing

### Today

`ContractGateValidator` has a single `errors.deadletterqueue.topic.name`
(standard Connect DLQ). All violations go there.

### Design

Add a small rules engine that picks a DLQ topic per record based on
violation metadata:

```
contractgate.dlq.routing.rules=[
  { "match": { "severity": "error", "type": "pii_leak" }, "topic": "audit.pii_failures" },
  { "match": { "severity": "error"                     }, "topic": "dlq.errors" },
  { "match": { "severity": "warn"                      }, "topic": "dlq.warnings" }
]
contractgate.dlq.routing.default=dlq.fallback
```

Rules evaluated top-to-bottom; first match wins. Default applies if
nothing matches. JSON value parsed via Jackson (already a dep).

Match fields available initially:
- `severity` — `error` | `warn`
- `type` — violation type string from the gateway response
- `field` — the field name that violated (for `type=schema` etc.)
- `contract` — contract name (for multi-contract task setups)

Future fields layer on without a protocol change as the gateway
violation response evolves.

### Config (new keys)

| Key | Default | Description |
|---|---|---|
| `contractgate.dlq.routing.enabled` | `false` | Enable per-violation routing. Opt-in. |
| `contractgate.dlq.routing.rules` | `[]` | JSON array of `{match: {...}, topic: "..."}` rules. |
| `contractgate.dlq.routing.default` | (none) | Fallback topic. Required when `enabled=true`; config validation rejects missing default. |

When `enabled=false` (default), behavior is identical to today — single
DLQ via standard Connect `errors.deadletterqueue.topic.name`.

### Implementation

`confluent-connector/src/main/java/io/datacontractgate/connect/smt/dlq/`:

```
dlq/
├── DlqRouter.java                      rule evaluator
├── DlqRule.java                        rule data class
└── DlqRoutingConfig.java               config parser + validator
```

`ContractGateValidator` calls `dlqRouter.route(violation, ctx)` when
the validation fails and routing is enabled. Returns the topic name
to use; null means "fall through to standard Connect DLQ" (unreachable
when `enabled=true` because `default` is required, but defensive).

### Producing to multiple topics from an SMT

SMTs don't normally have a producer handle. Two options to evaluate
at implementation time:

1. **Kafka Connect 3.6+ `errantRecordReporter` with per-record topic
   override** (if available — verify against the pinned 3.6.0 version
   in `pom.xml`). Cleanest.
2. **Open a dedicated producer in the SMT** using the worker's
   bootstrap config. Adds a separate producer config surface but is
   well-trodden in third-party SMTs.

Prefer (1). If unavailable, fall back to (2) and document the additional
config keys (`contractgate.dlq.routing.producer.*`) needed for the
internal producer.

---

## Backwards compatibility

Both features are opt-in via `*.enabled=false` defaults. The existing
SMT behavior is unchanged for everyone who doesn't set the new keys.

Existing tests (`ContractGateValidatorTest`) must still pass without
modification.

---

## Tests

- Unit: rule matching (each match field, top-to-bottom evaluation,
  default fallback, malformed JSON rejection at config-parse time).
- Unit: contract version hash compare (no-op on unchanged, swap on
  change, no-swap on parse failure).
- Integration (embedded Kafka): SMT with 3 rules routes 3 test
  violations to 3 different topics.
- Integration (embedded Kafka + mock HTTP gateway): contract bumped from
  v1 to v2; SMT picks up the change within `poll.ms`.
- Integration: gateway returns an unparseable contract; SMT keeps old
  contract, failure metric increments, `apply()` continues working.
- Regression: existing `ContractGateValidatorTest` passes unchanged.

---

## Docs (per project rules — user-facing)

Two new sections in `docs/connect-reference.md` (the existing SMT doc;
if it doesn't exist as a single consolidated doc yet, the equivalent
content lives in `confluent-connector/README.md` and should be added
there for now):

- "Dynamic contract reload" — config keys, polling semantics, failure
  behavior, troubleshooting, metrics names.
- "Per-violation DLQ routing" — config schema, rule evaluation order,
  available match fields, examples.

Per CLAUDE.md: any new user-facing config key needs a doc entry.

---

## Out of scope

- Push-based reload (gateway → SMT push). Polling is fine for v1.
- Conditional routing on record payload values (only on violation
  metadata). Customers who need payload-level routing can chain a
  standard Connect Filter SMT downstream.
- Rule hot-reload (rules require task restart for v1). Same trigger as
  contract reload could power this in a follow-up.
- Per-rule rate limiting or sampling.
- Multi-contract simultaneous validation in one SMT instance (one SMT,
  one contract — unchanged from today).

---

## Acceptance Criteria

1. `mvn -f confluent-connector/pom.xml verify` succeeds.
2. With `contractgate.reload.enabled=true`, an embedded-Kafka
   integration test demonstrates the SMT picking up a contract version
   bump within `poll.ms`.
3. With `contractgate.dlq.routing.enabled=true` and 3 rules configured,
   an embedded-Kafka integration test routes 3 test violations to 3
   different topics.
4. Without either flag, behavior is byte-identical to today — existing
   `ContractGateValidatorTest` passes unchanged.
5. Server-side: a cheap "current version" probe (`HEAD` or
   `GET /v1/contracts/:name/version`) exists and is documented; if not
   already present, it's added in the same PR.
6. `docs/connect-reference.md` (or `confluent-connector/README.md`)
   updated with both feature sections.

**Cannot test locally:** Alex (or CI) runs the Maven build and the
embedded-Kafka integration tests; Alex runs cargo for any server-side
version-probe endpoint addition.
