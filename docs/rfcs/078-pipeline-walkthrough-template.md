# RFC-078 — Cross-surface pipeline walkthrough template

**Status:** Implemented (RAG instance present but not GA-promoted; see RFC-077)
**Date:** 2026-06-08
**Branch:** TBD
**Depends on:** RFC-077 (RAG-ingestion profile — first walkthrough instance)
**Severity:** P3 — docs + examples only, no code, no engine change

---

## Problem

A data engineer evaluating ContractGate for one ingestion surface (say CSV)
should not have to re-learn the mental model when they later wire up another
(say RAG or Kafka). Today every supported surface has a **reference doc**
(`v1-ingest-reference.md`, `csv-inference-reference.md`,
`kafka-ingress-reference.md`, `kinesis-ingress-reference.md`) — these document
request/response shapes, headers, env vars, and status codes. None of them is a
**walkthrough**: none answers "what do I type, start to finish, to get a record
gated before it hits my pipeline." There is also no `examples/` directory in the
repo, so there is nothing to copy-paste.

Reference and walkthrough are different jobs. Reference describes the surface;
a walkthrough takes someone from zero to a working gate in five minutes. We are
missing the second one on every surface.

Blog posts are explicitly **not** the artifact: they go stale as the engine
moves, they bury the canonical docs, and they fragment the source of truth.
Docs-first; any future blog is a funnel that links *into* these walkthroughs.

## Goal

A single **walkthrough spine** — the same five beats on every supported surface
— plus one walkthrough per surface that fills the spine. Consistency is the
feature: the second walkthrough a reader opens should take 90 seconds because
the shape is already familiar.

## Non-goals

- No per-niche-use-case content. One walkthrough per *ingestion surface we
  support*, not per scenario.
- No engine, API, or contract-format change. This is docs + runnable examples.
- No blog series in this RFC.
- Does not rewrite the existing reference docs — walkthroughs link to them for
  exhaustive detail; reference docs stay as the lookup layer.

## The spine (identical on every surface)

Each walkthrough has exactly these five beats, in this order:

1. **The contract** — copy-paste YAML for this surface.
2. **The command** — the exact `contractgate` / endpoint invocation.
3. **A passing record** — input + the clean result.
4. **A failing record** — input + the specific violation it produces (show the
   gate actually biting; e.g. PII unredacted → blocked).
5. **Wire it in** — the one-liner / minimal snippet to put the check in the
   reader's real pipeline (e.g. LangChain/LlamaIndex call site, a curl in CI,
   a Kafka producer hook).

A short markdown template lives at `docs/walkthroughs/_TEMPLATE.md`; each
surface page fills the surface-specific slots. Writing the spine once and
filling slots is the leverage — most surfaces are documenting paths that
already exist.

## Surfaces to cover (bounded set)

| Surface | Walkthrough page | Runnable example | Notes |
|---|---|---|---|
| RAG corpus ingest | `docs/walkthroughs/rag.md` | `examples/contracts/rag/` | Shipped by RFC-077; first instance, most net-new thinking |
| HTTP / API bulk ingest | `docs/walkthroughs/api.md` | `examples/contracts/api/` | Documents existing `/v1/ingest` path |
| CSV / file ingest | `docs/walkthroughs/csv.md` | `examples/contracts/csv/` | Builds on existing inference route |
| Kafka ingress | `docs/walkthroughs/kafka.md` | `examples/contracts/kafka/` | Existing path; gated behind feature flag |
| Kinesis ingress | `docs/walkthroughs/kinesis.md` | `examples/contracts/kinesis/` | Existing path |

"Done" = every supported surface has one walkthrough on the spine + one runnable
example. New surfaces get a row only when the surface itself ships.

## Approach

1. Land RFC-077 first — its RAG reference doc + example contract becomes the
   reference implementation of the spine.
2. Extract the spine into `docs/walkthroughs/_TEMPLATE.md`.
3. Fill one surface at a time, RAG → API → CSV → Kafka → Kinesis. Each is
   independently shippable; demand can reorder the queue.
4. Each runnable example is exercised by the same parse+validate test harness
   RFC-077 introduces, so examples cannot silently rot.

## Testing

- Every `examples/contracts/**/*.yaml` parses and validates against the current
  engine (reuses RFC-077's test harness — an example that needs new code is a
  finding, not a doc).
- Each walkthrough's passing/failing record is the literal input used in that
  example's test, so the doc and the test cannot drift.
- `cargo test` + `cargo check` run by maintainer (cargo unavailable here).

## Rollout

Additive docs + examples. No migration, no API change, no config change. Fully
backward compatible.

## Open questions

- Walkthrough order after RAG — API or CSV next? (Lean: API; it's the most
  common evaluation entry point.)
- Keep walkthroughs under `docs/walkthroughs/` or co-locate each with its
  reference doc? (Lean: separate dir — walkthroughs are a coherent set a reader
  browses together.)
