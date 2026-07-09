# RFC-076 — `cg test`: local contract dry-run against sample data

**Status:** Draft
**Date:** 2026-06-01
**Branch:** `nightly-maintenance-2026-06-01-rfc076-contract-test-dry-run`
**Follows:** RFC-024 (Brownfield Scaffolder, `cg enforce`), RFC-006 (Multi-Format Inference)
**Severity:** P1 — competitive evaluation blocker (see §Context)

---

## Context

Findigs is evaluating ContractGate against the incumbent schema-registry tools.
Their primary decision criterion is the **rule-authoring iteration loop**: how
fast can an engineer write or infer a contract, throw representative JSON at it,
read the violations, fix the rule, and repeat — entirely locally, with no
running server, no Kafka, no network.

This is the one place where the incumbents are weak (their loop is
edit → deploy to registry → produce to a topic → consume failures), and it is
the one place ContractGate currently has **no CLI surface**, despite the engine
being able to do it in microseconds.

## Problem

The validation engine (`src/validation.rs`) already does everything needed:
`validate(&CompiledContract, &Value)` returns a `ValidationResult` with
per-field, per-kind `Violation` detail, in ~31µs p99. But nothing in the CLI
exposes "run this data through that engine":

- **`cg validate`** (`cli/commands/validate.rs`) only parses + compiles contract
  YAML. It answers "is this contract well-formed?" — *not* "does my data pass?".
  It never calls `validate()`.
- **`cg enforce`** (`cli/commands/enforce.rs`) *does* call `validate()` over a
  set of events and build a `ViolationReport`, but `collect_events()` hard-fails
  without `--topic` (`anyhow::bail!("specify --topic to select events to
  validate")`) and live consume requires `--features scaffold` + a Kafka broker.
  There is no local-file path.

So today, to test a contract against sample data an engineer must stand up
Kafka, produce to a topic, and run a feature-gated shadow enforce. That is the
exact slow loop we beat incumbents on in theory — and don't, in the demo.

## Goal

A single, dependency-free command:

```
cg test --contract user_events.yaml --data samples.ndjson
```

that loads the contract, runs every record through `validate()`, and prints a
clear pass/quarantine summary with per-record violation detail — exit 0 if all
pass, 1 if any fail. No server, no Kafka, no `scaffold` feature, no network.

Tight enough to sit inside an inner edit loop and in CI.

## Non-goals

- No new engine logic. `cg test` is a thin CLI shell over the existing
  `validate()` / `validate_envelope_batch()` and the `ViolationReport`
  formatters in `scaffold::report`. If a check is wrong, that's a separate RFC
  against `validation.rs`.
- No contract-ID/remote lookup (`id:uuid`). Local YAML path only, matching the
  current `enforce` MVP boundary. Remote fetch is a follow-up.
- No metric-formula evaluation. `validate_metric` still only checks the min/max
  envelope; formula metrics remain skipped (tracked separately). `cg test`
  reports exactly what the engine checks today — no more, no less, so the dev
  loop never lies about what production will enforce.

## Proposed design

### New subcommand

Add `pub mod test;` to `cli/commands/mod.rs` and a `Test(TestArgs)` variant to
the CLI enum, wired the same way `Enforce`/`Validate` are.

```
cg test \
  --contract <FILE>          # path to contract YAML (required)
  --data <FILE>              # NDJSON, JSON array, or single JSON object (required)
  [--format human|json]      # default: human
  [--fail-fast]              # stop at first failing record (default: off)
  [--envelope]               # treat --data as one envelope payload, use
                             #   validate_envelope_batch (auto-on if the contract
                             #   declares an `envelope:` stanza)
  [--quiet]                  # summary line only, suppress per-violation detail
```

### Input handling

`--data` accepts three shapes, auto-detected, so engineers can paste whatever
they have:

1. **NDJSON** — one JSON object per line (the common case; streams large files
   without loading all into memory).
2. **JSON array** — `[ {...}, {...} ]`.
3. **Single JSON object** — `{...}` (one record; the fastest possible loop).

Detection: trim leading whitespace; `[` → array, `{` on a single logical line vs
multiple lines disambiguates NDJSON-of-one vs array. When ambiguous, try
`serde_json::from_str::<Value>` over the whole buffer first (array/object), fall
back to line-by-line NDJSON. A parse failure on a record is itself reported as a
record-level error (index + the serde message), not a hard abort — a malformed
line shouldn't hide the validation result of the other 999.

`--data -` reads stdin, so `cg infer ... | cg test --contract c.yaml --data -`
chains the infer and test loops.

### Execution

Reuse the existing `validate()` call directly. For the array/NDJSON record set,
this is the same loop `enforce::run` already runs:

```rust
let compiled = load_contract_from_file(&args.contract)?;   // reuse enforce's loader, minus id: branch
let records  = read_records(&args.data)?;                  // new: file/stdin → Vec<Value> | streaming iter
for (idx, rec) in records.enumerate() {
    let result = validate(&compiled, &rec);
    // accumulate into the same ViolationReport shape enforce uses
}
```

When `--envelope` (or the contract has an `envelope:` stanza),
call `validate_envelope_batch(&compiled, cfg, &payload)` instead and render its
`BatchRecordViolation` list — the per-record-index detail is already there.

Uniqueness rules are batch-level (`check_uniqueness_batch`), so when records are
loaded as a full batch we also run that pass and fold its `(idx, Violation)`
results in. (Skipped in `--fail-fast` streaming mode; documented.)

### Output

Reuse `scaffold::report::ViolationReport` and its `format_markdown`/`format_json`
helpers — `enforce` already produces exactly this report from a
`Vec<Vec<Violation>>`, so `cg test` gets consistent output for free.

**Human (default):**
```
contract: user_events (v1.0)
data:     samples.ndjson  (1,000 records)

  PASS  994
  FAIL    6

  record 12  event_type   enum_violation   value "delete" not in allowed set: [click, view, purchase]
  record 12  timestamp    range_violation  value -1 is below minimum 0
  record 88  user_id      pattern_mismatch value "Bad ID!" does not match required pattern
  ... (3 more)

6/1000 records failed (0.6%)   validated in 1.2ms
```

**JSON (`--format json`):** the `format_json(&report)` output, suitable for CI
assertions and the dashboard.

### Exit codes

- `0` — all records pass.
- `1` — one or more records fail validation.
- `2` — usage / load error (contract won't compile, data file unreadable,
  unparseable top-level JSON). Distinct from `1` so CI can tell "my data is bad"
  from "I broke the harness".

## Why this beats the incumbents (the Findigs pitch)

| Step | Incumbent (registry + topic) | ContractGate `cg test` |
|---|---|---|
| Author rule | edit schema | edit YAML (or `cg infer`) |
| Get sample data in | produce to a topic | point at a local file / stdin |
| See failures | consume a DLQ / dead-letter topic | printed inline, per record |
| Iterate | redeploy schema, re-produce | re-run one command |
| Infra required | broker + registry running | none |
| Loop latency | seconds–minutes | low-ms, dominated by file read |

The engine speed (31µs/record) was never the bottleneck for them — the *loop*
was. This command turns our existing speed into a felt advantage in their POC.

## Testing

- Unit: `read_records` shape detection (NDJSON / array / single / stdin /
  malformed-line-is-a-record-error).
- Integration (`tests/cli_test_dryrun.rs`, mirrors `tests/cli_validate.rs`):
  - all-pass file → exit 0, "FAIL 0".
  - mixed file → exit 1, correct record indices and `ViolationKind`s.
  - `--envelope` payload → routes through `validate_envelope_batch`, per-record
    indices correct.
  - bad contract YAML → exit 2, not 1.
  - `--fail-fast` stops at first failing record.
- No new property tests needed — engine behavior is already covered by
  `validation.rs` tests; this RFC only adds I/O + wiring.

## Docs

Per CLAUDE.md (user-facing CLI flag → doc): add
`docs/cg-test-reference.md` (flags, input formats, exit codes, the chained
`infer | test` recipe) and add a row to the CLI section of the docs index.
Update `docs/STATUS.md` with the RFC-076 row.

## Rollout

Single nightly-maintenance branch. Additive only — no change to `validate`,
`enforce`, the engine, schema, or config. Zero risk to the hot path or existing
behavior.

## Open questions

1. **Name collision risk:** `cg test` vs the Rust convention of `cargo test`.
   Alternative: `cg check --data`. Recommendation: keep `test` — it's the verb
   users reach for ("let me test my contract"), and there's no `cg test` today.
2. Should `--data` directory globbing be supported (run a folder of fixtures)?
   Defer to v2; single file/stdin covers the eval loop.
3. Should we emit a JUnit-XML report variant for CI dashboards? Defer; `--format
   json` is enough for the Findigs eval.
