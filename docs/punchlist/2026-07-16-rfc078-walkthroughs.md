# Worklist 2026-07-16 — for Sonnet — RFC-078 cross-surface walkthroughs

Full rationale + design: [`docs/rfcs/078-pipeline-walkthrough-template.md`](../rfcs/078-pipeline-walkthrough-template.md).
Docs + runnable examples only. No engine/API/contract-format change.

## SCOPE FOR THIS PASS

RFC-078's first instance (RAG) and its test harness depend on **RFC-077, which is deferred**. So this pass does **everything except the RAG row**:

- The walkthrough **spine template**.
- Walkthroughs for the **four existing surfaces**: API (`/v1/ingest`), CSV, Kafka, Kinesis.
- One runnable example contract per surface.
- Validation that each example parses/validates against the current engine.

Leave a stub row / note that RAG is pending RFC-077. Do NOT invent a RAG profile or a new engine feature — "an example that needs new code is a finding, not a doc."

## Ground rules (CLAUDE.md + session-learned)

- **NO git operations of any kind.** Alex handles all git. Edit files, report.
- Be ultra-concise. Docs-first; **no blog-style content** (RFC-078 explicitly excludes blogs).
- Do NOT rewrite existing reference docs — walkthroughs **link to** them for exhaustive detail (`v1-ingest-reference.md`, `csv-inference-reference.md`, `kafka-ingress-reference.md`, `kinesis-ingress-reference.md`).
- Contract YAML must follow the locked format in CLAUDE.md.
- **Validate examples:** cargo now runs in-session — build the `cg` CLI and run `cg test --contract <file> --data <file>` (RFC-076) on each example's passing + failing record. Use `CARGO_TARGET_DIR=/tmp/cgtarget`. If cargo is unavailable, hand the validation step to the maintainer and say so explicitly.
- Update `MAINTENANCE_LOG.md` when done.

## The spine (identical on every surface) — put in `docs/walkthroughs/_TEMPLATE.md`

Exactly these five beats, in order:

1. **The contract** — copy-paste YAML for this surface.
2. **The command** — the exact `contractgate` / endpoint invocation.
3. **A passing record** — input + the clean result.
4. **A failing record** — input + the specific violation (show the gate biting; e.g. PII unredacted → blocked).
5. **Wire it in** — the minimal snippet to put the check in the reader's real pipeline (curl in CI, Kafka producer hook, etc.).

## Deliverables

| Surface | Walkthrough page | Runnable example | Source of truth to link |
|---|---|---|---|
| HTTP / API bulk ingest | `docs/walkthroughs/api.md` | `examples/contracts/api/` | `docs/v1-ingest-reference.md`, existing `/v1/ingest` path |
| CSV / file ingest | `docs/walkthroughs/csv.md` | `examples/contracts/csv/` | `docs/csv-inference-reference.md` |
| Kafka ingress | `docs/walkthroughs/kafka.md` | `examples/contracts/kafka/` | `docs/kafka-ingress-reference.md` (feature-flagged path) |
| Kinesis ingress | `docs/walkthroughs/kinesis.md` | `examples/contracts/kinesis/` | `docs/kinesis-ingress-reference.md` |
| RAG corpus ingest | — (stub, pending RFC-077) | — | note only |

Each `examples/contracts/<surface>/` holds: the contract YAML, a `pass` record, and a `fail` record — the literal inputs used in that surface's walkthrough beats 3 and 4, so doc and example can't drift.

## Order

Do API first (most common evaluation entry point), then CSV, Kafka, Kinesis. Each is independently shippable. Verify each surface's request/response shape against its reference doc before writing the beats — don't guess headers/status codes.

## Acceptance

- `docs/walkthroughs/_TEMPLATE.md` + four surface pages exist, all on the same five-beat spine.
- `examples/contracts/{api,csv,kafka,kinesis}/` each have contract + pass + fail.
- Every example contract parses and validates against the engine (or the validation step is explicitly handed to the maintainer with the exact commands).
- Passing/failing records in the docs are the literal example inputs.

## Do NOT

- Build the RAG walkthrough or any RFC-077 dependency.
- Change the engine, API, or contract format.
- Run any git command.
