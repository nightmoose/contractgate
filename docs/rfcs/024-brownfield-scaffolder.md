# RFC-024: Brownfield Contract Scaffolder

| Field         | Value                                              |
|---------------|----------------------------------------------------|
| Status        | **Draft — 2026-05-06**                             |
| Author        | ContractGate team                                  |
| Created       | 2026-05-06                                         |
| Target branch | `nightly-maintenance-2026-05-06`                   |
| Tracking      | Post-demo roadmap (project_post_demo_roadmap.md)   |

---

## Summary

Add a `cg scaffold` command that introspects a live Kafka topic (Avro, Protobuf,
or JSON), samples events, profiles field statistics, detects PII candidates, and
emits a draft ContractGate YAML contract. A companion `cg enforce --mode shadow`
sub-command replays the scaffold output through the existing validation engine
without touching the hot ingest path.

The scaffolder is read-only by design. It never commits offsets, never writes to
the topic, and never auto-applies PII transforms.

---

## A. Contract Format

### Decision

**Emit native CG YAML. ODCS export is on-demand via the existing export endpoint.**

Rationale:

- CLAUDE.md locks the canonical format: `version`, `ontology.entities`, `glossary`,
  `metrics`. This is what the validation engine in `src/validation.rs` compiles
  against (`pub fn validate` at line 247). Emitting anything else as the primary
  output would require the user to round-trip through import before the contract
  is usable.
- The ODCS extension layer (`docs/odcs/extension-strategy.md`,
  `docs/odcs/field-mapping.md`) is already designed and partially implemented in
  `src/odcs.rs`. The scaffolder can reuse it via `POST /contracts/:id/versions/:version/export`
  after the contract is created — no new serialisation code needed.
- Dual-emit at scaffold time would produce two files that immediately diverge
  on the first human edit. The ODCS `x-contractgate-*` round-trip guarantee
  requires the native block to be authoritative; emitting ODCS first inverts
  that invariant.
- ODCS-style fields from the spec sample (`id`, `schema`, `quality`, `pii`,
  `source`) that have no native CG equivalent are captured in the profiler
  output as structured comments (YAML comments prefixed `# scaffold:`) and
  lifted into `glossary[].description` and `metrics[]` where semantics match.

**DECISION: Native CG YAML output. ODCS export deferred to the existing export
endpoint. No dual-emit at scaffold time.**

---

## B. Kafka + Schema Registry Introspection

### Consumer Configuration

```toml
# Conceptual rdkafka ClientConfig — not final code
group.id            = "contractgate-scaffold-{uuid4}"   # unique per run; never reused
auto.offset.reset   = "earliest"                        # see prod-offset safety below
enable.auto.commit  = false                             # NEVER commit; read-only
fetch.max.bytes     = 10_485_760                        # 10 MB/fetch cap
session.timeout.ms  = 10_000
max.poll.interval.ms= 30_000
```

`group.id` is a UUID4 appended to a static prefix so consumer groups never
accumulate in the broker. The CLI prints the group ID so operators can
verify it was cleaned up.

`auto.offset.reset = earliest` is safe here because `enable.auto.commit =
false` means the broker never records a committed offset for this group.
If the broker has `group.min.session.timeout.ms` that would auto-expire the
group, that's fine — we are ephemeral by design.

`max.poll.interval.ms` is set low (30 s) to ensure the ephemeral group
expires quickly after sampling completes.

### Auth Matrix

| Broker security   | SR security | rdkafka config keys                                                       |
|-------------------|-------------|---------------------------------------------------------------------------|
| PLAINTEXT         | none        | _(default)_                                                               |
| SASL/PLAIN        | basic auth  | `security.protocol=SASL_PLAINTEXT`, `sasl.mechanism=PLAIN`, `sasl.username`, `sasl.password`; SR: `schema.registry.basic.auth.user.info` |
| SASL/SCRAM-256    | basic auth  | `security.protocol=SASL_SSL`, `sasl.mechanism=SCRAM-SHA-256`, `ssl.ca.location`; SR: same |
| SASL/SCRAM-512    | basic auth  | `security.protocol=SASL_SSL`, `sasl.mechanism=SCRAM-SHA-512`             |
| mTLS (SSL)        | mTLS        | `security.protocol=SSL`, `ssl.ca.location`, `ssl.certificate.location`, `ssl.key.location`; SR: `schema.registry.ssl.*` |

All credentials are read from environment variables or a `~/.contractgate/credentials.toml`
file with `0600` permissions. Credentials are never written into the emitted
contract YAML (see §L — PII leakage risk).

### Decoder Paths

| Format    | SR required | Decoder                                     | Existing code                                  |
|-----------|-------------|---------------------------------------------|------------------------------------------------|
| Avro      | Recommended | Parse magic-byte framing → fetch schema from SR → reuse `infer_avro.rs` `avro_schema_to_fields` | `src/infer_avro.rs` — `POST /contracts/infer/avro` handler |
| Protobuf  | Recommended | Strip SR framing → fetch `.proto` from SR → reuse `infer_proto.rs` proto parser | `src/infer_proto.rs` — `POST /contracts/infer/proto` handler |
| JSON      | No          | Raw JSON deserialise → reuse `infer.rs` `infer_fields_from_objects_pub` | `src/infer.rs` line ~100 |

SR-less fallback: if SR is unreachable, Avro and Protobuf fall back to
raw-byte JSON inference (parse the payload after stripping the 5-byte magic
header). Quality drops significantly — output is annotated with
`# scaffold: sr-unavailable; schema-driven inference skipped`. See §L.

### Sample Strategy

```
samples = min(--records N [default 1000], --wall-clock T [default 30s])
```

Sampling stops at whichever limit is reached first. Partitions are assigned
round-robin; if the topic has > P partitions and we collect N records total,
each partition contributes approximately N/P records.

`--fast` skips per-record parsing and runs profiler only on the raw byte
payloads (field names from schema only, no value distribution). Useful for
very wide schemas.

**DECISION: ephemeral UUID consumer group, `enable.auto.commit=false`,
sample bounds = `min(N records, T wall-clock)`. SR-less fallback to
raw-byte JSON with quality warning.**

---

## C. Profiler Module

### Stats per Field

| Statistic          | Algorithm                                     | Notes                              |
|--------------------|-----------------------------------------------|------------------------------------|
| `null_rate`        | `null_count / total_count`                    | Drives `required` inference        |
| `distinct_count`   | HyperLogLog++ (error ≈ 0.8%)                  | 12-bit register, ~6 KB/field       |
| `min` / `max`      | Exact streaming min/max                       | Numeric fields only                |
| `p5` / `p50` / `p95` length | Reservoir sample → sort (k=1000)   | String/bytes fields                |
| `top_k` values     | Count-min sketch + exact heap (k=20)          | String fields; elided if distinct > 200 |
| `type_consensus`   | Majority vote across samples                  | Feeds `FieldType` decision         |

Memory bound: configurable via `--profiler-memory-mb` (default 64). Each
HyperLogLog register set is ~6 KB. With 64 MB budget: ≈ 10 000 fields before
spilling to disk (uses temp file + merge pass). In practice ≤200 fields is the
common case.

All profiler state is streaming — one pass over the sample set, no
materialisation of all records in RAM simultaneously. The sampled records
themselves are discarded after per-record contribution to profiler state.

**DECISION: HyperLogLog++ for distinct count, count-min sketch for top-k,
streaming fold, 64 MB default budget, single pass.**

---

## D. PII Detection

### Detection Strategy (v1)

Two orthogonal signals, combined into a `confidence` score (`0.0`–`1.0`):

**Signal 1 — field-name list** (`weight 0.6`):
Exact and fuzzy match against a curated name list. Examples:
`email`, `ssn`, `social_security`, `phone`, `credit_card`, `cc_number`,
`dob`, `date_of_birth`, `ip_address`, `password`, `passwd`, `api_key`,
`secret`, `first_name`, `last_name`, `full_name`, `address`, `zip`,
`postal_code`, `passport_number`, `driver_license`.

Fuzzy match: Levenshtein distance ≤ 1 on the base name after splitting
`snake_case` / `camelCase` tokens. False-positive rate is acceptable at v1;
human review is mandatory before promotion.

**Signal 2 — value regex** (`weight 0.4`):
Applied to sampled string values. Pattern library:

| PII type          | Pattern                                              |
|-------------------|------------------------------------------------------|
| Email             | `(?i)[a-z0-9._%+\-]+@[a-z0-9.\-]+\.[a-z]{2,}`      |
| US SSN            | `\b\d{3}-\d{2}-\d{4}\b`                              |
| US phone          | `\b(\+1\s?)?\(?\d{3}\)?[\s\-]\d{3}[\s\-]\d{4}\b`    |
| Credit card       | `\b(?:4\d{12}(?:\d{3})?|5[1-5]\d{14}|3[47]\d{13})\b` |
| IP address        | `\b(?:\d{1,3}\.){3}\d{1,3}\b`                        |
| UUID (might be PII)| `\b[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\b` (low confidence 0.1) |

Signal 2 fires if ≥ 10% of sampled non-null values match. Confidence =
`field_name_score * 0.6 + value_match_score * 0.4`. Threshold for emitting
TODO: confidence ≥ 0.4.

### Output Convention

PII candidates are **never** auto-remediated. The scaffolder emits:

```yaml
# scaffold: pii-candidate confidence=0.85 reason="field_name:email + value_match:email"
# TODO: review PII — consider transform.kind: hash or mask
- name: user_email
  type: string
  required: false
```

The `transform` block is written as a YAML comment, not live YAML. The human
must uncomment and confirm it before promotion. This is a hard invariant — no
mode flag overrides it.

**DECISION: regex + name-list v1, confidence score, emit TODO comment only.
Auto-apply is permanently blocked regardless of CLI flags. Threshold 0.4.**

---

## E. Incremental Re-scaffold and Merge Semantics (OQ1)

### Problem

Re-running `cg scaffold` on an evolving topic must not clobber human edits
to the existing contract (glossary entries, transform configurations, metric
formulas).

### Algorithm: Field-Level Three-Way Merge

Three inputs:

- **Base**: the contract YAML emitted by the previous scaffold run (stored as
  `scaffold_base` metadata on the contract version row, or reconstructed from
  git).
- **Ours**: the current live contract version (human-edited).
- **Theirs**: the new scaffold output from the current run.

Merge is performed at field granularity, not YAML text level:

```
for each field F in union(ours.fields, theirs.fields):
  base_f   = base.fields[F]    # may be None if field is new
  ours_f   = ours.fields[F]    # may be None if field was deleted
  theirs_f = theirs.fields[F]  # may be None if field disappeared from topic

  if ours_f == base_f:
    # human made no edit → accept theirs_f (schema drift update)
    result[F] = theirs_f
  elif theirs_f == base_f:
    # schema unchanged → preserve human edit
    result[F] = ours_f
  elif ours_f != base_f and theirs_f != base_f:
    # both changed → CONFLICT
    emit warning, preserve ours_f, annotate with # scaffold: conflict
  elif theirs_f is None and ours_f is not None:
    # field disappeared from topic → flag drift, preserve ours_f
    emit warning "field F not seen in sample; may have been removed"
    result[F] = ours_f  # preserved; human decides
```

`scaffold_base` is stored as a JSON blob in the `contract_versions.scaffold_metadata`
column (new, nullable). If absent (contract was hand-written), the merge
degrades to `theirs` with all human-edited fields flagged as potential
conflicts.

Schema drift summary is printed to stderr and also written to the report
sink (see §G).

**DECISION: field-level three-way merge, base stored in nullable
`scaffold_metadata` column, conflicts preserved as `ours` with annotation,
drift flagged but not auto-deleted.**

---

## F. CLI Surface

### Commands

```
cg scaffold <topic>
    --broker <host:port>            (default: $CG_KAFKA_BROKER)
    --schema-registry <url>         (default: $CG_SR_URL; omit for SR-less)
    --auth <plaintext|sasl-plain|sasl-scram-256|sasl-scram-512|mtls>
    --records <N>                   (default 1000)
    --wall-clock <seconds>          (default 30)
    --fast                          (skip value profiling; schema-driven only)
    --output <file.yaml>            (default: stdout)
    --contract-id <uuid>            (enables merge mode; requires existing contract)
    --dry-run                       (print merge diff; no write)

cg scaffold --from-file <path>      (path = .avsc / .proto / .json / ndjson)
    [--output, --dry-run same as above]

cg enforce --mode shadow
    --contract <file.yaml | id:uuid>
    --broker <host:port>
    --topic <topic>
    --records <N>   --wall-clock <T>
    --report <markdown|json|prometheus>   (default: markdown)
    --output <file | stdout>
    --dry-run                       (no Prometheus push; print report only)
```

### Exit Codes

| Code | Meaning                                                         |
|------|-----------------------------------------------------------------|
| 0    | Success; no violations (shadow) or clean scaffold               |
| 1    | Violations found in shadow mode (report written)                |
| 2    | Fatal error (connection failure, parse error, auth failure)     |
| 3    | Schema drift detected during merge (re-scaffold with `--dry-run`)|
| 4    | PII candidates detected (non-fatal; exit 0 if `--no-pii-exit`)  |

### Help Example

```
$ cg scaffold orders --broker kafka:9092 --schema-registry http://sr:8081 \
    --records 5000 --output contracts/orders.yaml
Sampling topic 'orders' … 5000 records in 12.3s
Detected format: Avro (SR schema id=42)
Fields: 9   PII candidates: 2 (confidence ≥ 0.4)
Wrote contracts/orders.yaml
Exit: 0 (review PII annotations before promoting)
```

**DECISION: two top-level subcommands (scaffold, enforce), exit codes 0–4,
credentials from env or `~/.contractgate/credentials.toml` only.**

---

## G. Shadow Enforcement Reuse

### Hot-Path Isolation

The validation engine (`src/validation.rs`, `pub fn validate` at line 247)
accepts a `CompiledContract` + `Value` and returns a `ValidationResult`. It is
synchronous, allocation-light, and carries the sub-15ms p99 guarantee.

Shadow enforcement **must not** be wired into the live ingest path. It runs as
a separate CLI process that:

1. Consumes from the topic using the same ephemeral-group pattern as the scaffolder.
2. Calls `validate(&compiled, &event)` from `src/lib.rs` directly — same binary
   code, zero diff to the ingest path.
3. Collects violations into an in-process sink.
4. Writes a report on exit.

No changes to `src/main.rs`, `src/ingest.rs`, or `src/v1_ingest.rs`. The
ingest hot path is untouched.

### Violation Sinks

| `--report` value | Output                                                                       |
|------------------|------------------------------------------------------------------------------|
| `markdown`       | Fenced table: field, violation type, sample value (redacted), count, rate   |
| `json`           | `{"violations": [...], "total": N, "violation_rate": 0.03, "contract": "..."}` |
| `prometheus`     | Push to Pushgateway: `contractgate_shadow_violations_total{field, rule}` gauge |

Prometheus push requires `$CG_PUSHGATEWAY_URL`. If unset, exits with code 2
after printing to stderr.

**DECISION: shadow runs as separate CLI process, calls `validate()` from
lib.rs directly, zero changes to ingest hot path, three report sinks.**

---

## H. Module Layout

### New Code Location

```
src/scaffold/
  mod.rs          — public API: ScaffoldConfig, run_scaffold(), run_enforce()
  kafka.rs        — rdkafka consumer setup, partition assignment, sample loop
  profiler.rs     — streaming field profiler (HyperLogLog, count-min)
  pii.rs          — PII regex patterns + name-list matching + confidence score
  merge.rs        — three-way merge algorithm
  report.rs       — markdown / JSON / prometheus violation report formatters
```

No new Cargo workspace member — scaffold code lives in the existing `contractgate`
crate under `src/scaffold/`. This avoids a second crate that would double the
compile surface for CI.

Feature-flagged behind `features = ["scaffold"]` in `Cargo.toml` so the
production gateway binary does not pull in rdkafka unless explicitly built
with the feature. The `cg` CLI binary (new `src/bin/scaffold.rs`, or an
additional subcommand in the existing `src/bin/` structure) opts in.

### Existing Code Reused

| Existing module                 | Reuse                                               |
|---------------------------------|-----------------------------------------------------|
| `src/infer.rs`                  | `infer_fields_from_objects_pub()` for JSON payloads |
| `src/infer_avro.rs`             | `avro_schema_to_fields()` for Avro SR path          |
| `src/infer_proto.rs`            | Proto3 parser for Protobuf SR path                  |
| `src/validation.rs`             | `validate()` for shadow enforcement                 |
| `src/odcs.rs`                   | ODCS export on-demand after contract creation       |
| `src/contract.rs`               | `Contract`, `FieldDefinition`, `Ontology` structs   |

**DECISION: `src/scaffold/` module inside existing crate, Cargo feature flag,
no new workspace member, reuse all four existing infer modules.**

---

## I. Test Plan

### docker-compose Fixtures

New compose profile `kafka-test`:

```yaml
zookeeper:
  image: confluentinc/cp-zookeeper:7.6.0
  profiles: [kafka-test]

kafka:
  image: confluentinc/cp-kafka:7.6.0
  profiles: [kafka-test]

schema-registry:
  image: confluentinc/cp-schema-registry:7.6.0
  profiles: [kafka-test]
```

Three fixture topics pre-seeded by a `kafka-fixture-seeder` container:
- `test.json_topic` — 500 raw JSON records (user_events shape)
- `test.avro_topic` — 500 Avro records with SR schema
- `test.proto_topic` — 500 Protobuf records with SR schema

### Test Categories

**Integration tests (`tests/scaffold_integration.rs`):**
- `test_json_scaffold_roundtrip` — scaffold JSON topic → validate output contract
  compiles and validates a known-good event
- `test_avro_scaffold_sr` — scaffold Avro topic with SR → field types match schema
- `test_sr_unavailable_fallback` — SR down → JSON fallback annotation present
- `test_merge_preserves_human_edits` — re-scaffold with existing contract →
  human-edited `glossary[0].description` is unchanged
- `test_merge_flags_schema_drift` — remove a field from fixture → exit code 3

**Golden contracts (`tests/fixtures/golden/`):**
One expected YAML output per fixture topic. CI diffs scaffold output against golden.
Golden update: `UPDATE_GOLDEN=1 cargo test scaffold`.

**Property tests (`tests/scaffold_property.rs` via `proptest`):**
- Profiler: `null_rate ∈ [0,1]` for any sample set
- Profiler: `distinct_count ≤ sample_count` always
- Profiler: HyperLogLog error ≤ 2% on uniform distributions ≥ 1000 items
- Merge: `merge(base, ours=base, theirs)` == `theirs` (no human edit → accept scaffold)
- Merge: `merge(base, ours, theirs=base)` == `ours` (no schema change → preserve edit)

**PII unit tests (`tests/pii_tests.rs`):**
- Each regex pattern: 5 true-positive and 5 true-negative examples
- Name-list: known PII field names match at confidence ≥ 0.6
- Non-PII field names (`amount`, `timestamp`, `event_type`) score ≤ 0.15

**Shadow enforcement tests:**
- Inject 10% invalid events into fixture → shadow report shows violation_rate ≈ 0.10
- `--dry-run` produces no Prometheus push, exits 1

**DECISION: docker-compose kafka-test profile, golden files, proptest for
profiler + merge, per-regex unit tests, shadow integration test.**

---

## J. Phased Delivery

### MVP — Target mid-June 2026

In scope:
- `cg scaffold --from-file` (JSON + Avro + Proto file paths; no live Kafka)
- `cg scaffold <topic>` with PLAINTEXT and SASL/PLAIN auth (Kafka + SR)
- Profiler: null_rate, distinct_count (HyperLogLog), min/max, top_k
- PII detection v1 (name-list + regex, no fuzzy matching)
- Native CG YAML output
- `--output`, `--dry-run`, `--records`, `--wall-clock`
- Exit codes 0–4
- JSON golden tests + basic integration test against kafka-test compose profile

Out of scope for MVP:
- SASL/SCRAM-256/512 and mTLS auth
- Protobuf SR path (file-based `.proto` works; live SR Protobuf deferred)
- Incremental re-scaffold / merge semantics (plain re-scaffold, no merge)
- `cg enforce --mode shadow`
- Prometheus sink
- `--fast` flag
- Fuzzy field-name PII matching

### Phase 2 — end of July 2026

- All auth methods
- Merge semantics + `scaffold_metadata` column migration
- Shadow enforcement with markdown + JSON report
- `--fast` flag + Prometheus sink

### Phase 3 — Q3 2026

- Fuzzy PII name matching
- ODCS export integration (auto-offer after scaffold)
- Scheduled re-scaffold via existing `schedule` skill / cron hook

**DECISION: MVP = file-based + JSON/Avro Kafka + basic profiler + PII v1 + exit
codes. Merge, shadow, mTLS, Prometheus deferred to Phase 2.**

---

## K. Open Questions — Resolved

### OQ1: Incremental re-scaffold merge semantics

**Question:** When re-running scaffold on an existing contract, how do we
preserve human edits while incorporating schema drift?

**Recommendation (adopted above in §E):** Field-level three-way merge with
`scaffold_metadata` stored on the version row. Conflicts are preserved as
`ours` with a YAML annotation. Drift is flagged via exit code 3 but not
auto-deleted. This is the conservative choice — data loss from accidental
overwrite of a human-tuned `transform` config would be a compliance incident.

### OQ2: Schema Registry unavailability fallback

**Question:** When SR is unreachable at scaffold time, should we abort or
degrade gracefully?

**Recommendation:** Degrade to raw-byte JSON inference with a quality warning
annotation. Abort only if `--require-sr` flag is passed (default: off).
Rationale: in many brownfield environments SR may be behind a firewall or
temporarily down. A partial scaffold with quality warnings is more useful than
a hard failure. The warning annotation ensures the contract is not promoted
to stable without review.

### OQ3: PII transform auto-apply policy

**Question:** Should scaffold ever automatically apply `transform` blocks
(e.g. `kind: hash`) when PII confidence is high?

**Recommendation:** Never. See §D. Even at confidence 1.0, auto-applying a
`transform.kind: drop` would silently discard production data the first time
the contract is enforced in non-shadow mode. The correct flow is:
scaffold → human review → manual uncomment of transform → PR review →
promote. The cost of a false-positive auto-apply (data loss) exceeds the cost
of a missed PII annotation (manual review catches it).

---

## L. Risks Not in the Spec

### 1. Credentials in Committed YAML

Kafka broker addresses and auth credentials must never appear in the emitted
contract YAML. The scaffold output contains only field schema — no connection
metadata. The CLI reads credentials from env vars or `~/.contractgate/credentials.toml`
(`0600`) and never echoes them to stdout. CI must assert the golden contract files
contain no strings matching known credential patterns.

### 2. Large Topics / Partition Lag

On a topic with millions of unread messages, `auto.offset.reset = earliest` would
sample from a very old window. Scaffolder should default to `latest - N_records`
offset seek rather than true `earliest`. Implemented as: seek each partition to
`(high_watermark - records_per_partition)` before starting the sample loop.
Add `--from-latest` / `--from-earliest` flags to let the operator choose.

### 3. Offset Poisoning

If a team re-runs scaffold with the same group ID (e.g., from a script that
hardcodes the group name), they would commit offsets and could affect a monitoring
consumer that shares the name. Mitigation: the UUID4 suffix makes collision
effectively impossible. Document `enable.auto.commit=false` as a hard invariant
in the CLI help text.

### 4. PII Field Names in Committed YAML

Even if values are never sampled into the contract, a field named `ssn` in the
emitted YAML is itself a signal that SSNs flow through the pipeline. The YAML
file will likely be committed to a git repo. This is expected and acceptable
(the contract describes the schema), but teams should be aware. Document in the
`cg scaffold` help: "emitted field names reflect the topic schema; treat the
output file with the same access controls as schema documentation."

### 5. SR-less Avro / Protobuf Quality

Without SR, Avro wire format (5-byte magic + schema ID + binary payload) is
unreadable as JSON. The raw-byte JSON fallback (`src/infer.rs`) will infer a
single `bytes` field named `_raw`. This is nearly useless. The CLI should emit
a hard warning and recommend obtaining SR access before scaffolding binary topics,
not silently produce a single-field contract. Quality gate: if decoded fields
= 1 and format is Avro/Protobuf, exit code 2 with explanation unless
`--accept-sr-fallback` is passed.

### 6. Patent Boundary

The scaffolder generates contract drafts from live data. Ensure the scaffolder
is clearly scoped as a **developer tooling** feature in patent claims, not as
part of the core validation engine (the patent-core). The `src/scaffold/`
module should have a file-level comment: `// Developer tooling — not part of
the patent-core validation engine`. No scaffold code should be imported by
`src/validation.rs` or `src/ingest.rs`.

---

## Acceptance Criteria (Phase 1 MVP)

- [ ] `cg scaffold --from-file samples.json` emits valid CG YAML that passes
      `cargo test` without any Kafka broker present.
- [ ] `cg scaffold orders --broker kafka:9092` samples ≤ 1000 records and exits
      without committing offsets (verified by broker consumer-group describe).
- [ ] PII candidates emit `# TODO` annotations, not live YAML `transform` blocks.
- [ ] `--dry-run` prints diff, writes no files, exits 0 (no violations) or 1.
- [ ] All exit codes 0–4 exercised in CI.
- [ ] Golden contract tests pass.
- [ ] `cargo check --features scaffold` passes.
- [ ] No credentials appear in any scaffold output or test fixture.
- [ ] `src/validation.rs` diff is empty (hot path untouched).
- [ ] Patent-boundary comment present in `src/scaffold/mod.rs`.

---

## Open Questions (Deferred)

1. Should `scaffold_metadata` be a separate table or a `jsonb` column on
   `contract_versions`? Lean: `jsonb` column — avoids a join on the hot
   read path, and scaffold metadata is only accessed during re-scaffold, not
   ingest.
2. `--fast` flag: skip value profiling entirely, or skip only top-k and length
   percentiles but keep null_rate? Lean: keep null_rate (one counter per field,
   near-zero cost).
3. For the Prometheus sink, push to Pushgateway or expose a `/metrics` scrape
   endpoint on a short-lived local server? Lean: Pushgateway — CLI processes
   are not long-lived.

---

## Sign-off

- [ ] Alex — approve scope, format decision, OQ resolutions
- [ ] Alex — confirm MVP boundary (mid-June target realistic?)
