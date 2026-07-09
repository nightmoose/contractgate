# ContractGate Walkthroughs

Start-to-finish guides for gating data on each supported ingestion surface.
Every walkthrough follows the same five beats — **the contract**, **the
command**, **a passing record**, **a failing record**, **wire it in** — so once
you've read one, the rest take 90 seconds.

For exhaustive parameter detail, each walkthrough links to its reference doc.

| Surface | Walkthrough | When to use it |
|---|---|---|
| HTTP / API | [api.md](api.md) | Anything that can make an HTTP POST. The universal connector. |
| RAG corpus | [rag.md](rag.md) | Gate documents before chunking/embedding (provenance + PII attestation). |
| CSV / file | [csv.md](csv.md) | Infer a contract from a sample file, then validate rows. |
| Kafka | [kafka.md](kafka.md) | Validate a Kafka stream in place (clean / quarantine topics). |
| Kinesis | [kinesis.md](kinesis.md) | Validate an AWS Kinesis stream in place (clean / quarantine streams). |

All five run the **same validation engine** — only the transport differs. The
contracts in [`examples/contracts/`](../../examples/contracts/) are the runnable
sources used in each walkthrough.

New to ContractGate? Start with [api.md](api.md) — it's the most common
evaluation entry point — then [`cg test`](../cg-test-reference.md) for local
validation with no server.

Authoring a new walkthrough: copy [`_TEMPLATE.md`](_TEMPLATE.md) and fill the
five beats. See [RFC-078](../rfcs/078-pipeline-walkthrough-template.md).
