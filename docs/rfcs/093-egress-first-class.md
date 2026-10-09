# RFC-093 — Egress as a first-class door

**Status:** Shipped
**Branch:** nightly-maintenance-2026-09-29-rfc093-egress-first-class
**Date:** 2026-09-30
**Depends on:** RFC-029 (egress validation), RFC-030 (egress PII and leakage), RFC-065 (contract scope on egress), RFC-006/035/079 (inference), RFC-089 (agent playbook), RFC-090 (MCP server)

---

## Problem

The outbound path already works. `POST /egress/{contract_id}` runs the same
`validate()` engine as ingest, applies the same PII transforms, honors
`egress_leakage_mode`, and writes audit and quarantine rows with
`direction = 'egress'` (RFC-029, RFC-030, RFC-065). Disposition (`block`,
`fail`, `tag`) and `dry_run` are call-time parameters. None of that is what a
new user or an agent can find.

| Surface | What it says about egress |
|---|---|
| `README.md` | Ingest only. "Stop bad data before it reaches your warehouse." |
| `docs/llm-integration.md` | No mention. The paste flow wires `POST /v1/ingest` and stops. |
| MCP server (RFC-090) | `infer_contract`, `validate_events`, `deploy_contract`, `get_quarantine`, `list_contracts`. `validate_events` hits ingest or the playground. No egress tool. |
| Python SDK (RFC-005) | `ingest()`. No egress method. |
| CLI | No egress subcommand. RFC-029 listed `validate-egress` as optional; it was never built. |
| Dashboard | The only egress screen is a tab on the Contract Catalog page, next to "Browse & Import" (`dashboard/app/catalog/page.tsx`). Leakage mode is a control on the contract YAML tab. |

The case this misses: the warehouse is already clean, and a later step fouls
the payload. A notebook, a reverse-ETL job, or an agent assembling state
renames a field, casts an amount negative, or pastes a raw email back in.
Whatever is about to be sent to a downstream system, including a hosted
decision model, is an outbound payload. Ingress cannot see it. Egress can,
and nothing in the adoption path says so.

"Contracts are a bitch to design and maintain" is the objection to contracts
as a category, not an argument against this door. The builder is the answer,
and it already infers a contract from samples. It is not pointed at outbound
samples anywhere a user looks.

## Goal

One contract language, two doors. A person or an agent can infer a contract
from known-good outbound payloads, deploy it, and call egress so that only
the returned `payload` is forwarded.

**Done means:**

1. The playbook has an outbound section an agent will actually follow, with
   the known-good-sample rule written as an instruction, not a footnote.
2. The MCP server can dry-run and, when asked, live-call
   `POST /egress/{contract_id}`, and the Python SDK can do the same.
3. The dashboard egress check lives on the contract, next to ingest, not
   only as a catalog tab.
4. The README states the two doors in one short paragraph.
5. No change to `validate()`, no new rule type, no model client.

## Non-goals

- **A client for any decision model, including Jev.** Jev (TypeSafe AI,
  September 2026) is the motivating example: a hosted model that judges a
  posted state and returns a typed answer with a confidence. ContractGate
  does not call it. The caller posts the egress `payload` themselves.
  Putting a remote model call on the ingest or egress path would throw away
  the latency budget and send customer payloads to a third party. Do not
  build it under this RFC.
- **Checking truth the contract cannot express.** A value inside the
  declared range, enum, and pattern still passes when it is factually
  wrong ("three late payments" when the truth is zero). That is the
  boundary of every contract, inbound or outbound. This RFC does not add a
  model, a heuristic, or a second engine to close it.
- **Detecting fouled inference samples in the server.** If the caller
  infers from the broken stream, the builder writes the break into the
  contract. The server cannot know the samples were bad. The playbook
  forbids it. No new infer parameter, no `direction` field on infer.
- **Forcing one `contract_id` for both doors.** RFC-029's symmetric case
  still holds: when the outbound shape matches the inbound shape, reuse the
  contract. When the outbound payload is a different object (a condensed
  state, not the warehouse row), it gets its own contract in the same
  language. Do not add a direction flag that makes one document serve both
  shapes.
- **Streaming egress.** HTTP stays the door, as RFC-029 scoped it. Kafka
  and Kinesis consumers can call the same endpoint. No outbound connector.
- **A new quarantine product.** Rows are already tagged `direction`.
  Filtering the quarantine UI is follow-up work, not a gate on this RFC.
- **Reopening disposition defaults.** `block` / `fail` / `tag` stay
  call-time. No `egress:` block in the locked YAML.

## Design

### 1. The loop

```
known-good outbound samples
        │
        ▼
POST /contracts/infer          ← existing builder, unchanged
        │
        ▼
review YAML, POST /contracts/deploy
        │
        ▼
trash process assembles a payload
        │
        ▼
POST /egress/{contract_id}?dry_run=true     then, live, dry_run=false
        │
        ▼
forward response.payload only          ← caller sends this onward
```

Samples are the last payloads the caller is willing to stand behind, not
whatever the trash process emitted this morning. The known-bad check is the
same shape as the ingest playbook: a schema-valid wrong event must fail.
Use an event like `{"event_type":"purchsae","amount":-49.99}` only when
those constraints are in the contract. Do not hard-code that payload into
the product.

The contract describes the object egress will see. It does not describe the
warehouse table unless that table is the object being sent.

### 2. Playbook

Add a section to `docs/llm-integration.md` after the ingest wiring, in the
imperative, to the agent. Order:

1. Decide whether the outbound object is the same shape as ingest. Same
   shape: reuse the `contract_id`. Different shape: collect 5–20
   known-good outbound samples and infer a second contract. State the
   sample rule in one sentence: do not infer from the fouled output.
2. Deploy it (existing deploy step).
3. Call `POST /egress/{contract_id}` with `dry_run=true` first. Document
   `disposition` (`block` default, `fail`, `tag`) and the response fields
   the agent must read: `payload`, `outcomes`, `passed`, `failed`.
4. Wire the producer of the outbound payload to forward `payload` and to
   drop or quarantine what egress excluded. Never forward the original
   body after a `block` or `fail`.
5. Verify with one known-bad object and one known-good object.

`tests/llm_docs_test.rs` already fails when a playbook path is not a real
route. The new section must cite `POST /egress/{contract_id}` so that gate
covers it. Also add the endpoint to `docs/llms.txt` in the same index style
as ingest.

Update `docs/egress-validation-reference.md` with the sample rule and the
"forward `payload` only" rule. Do not fork the reference into a second doc.

### 3. MCP

Add `egress_validate` to `mcp/`. Same client and auth as the other tools.

| | |
|---|---|
| Gateway | `POST /egress/{contract_id}` |
| Args | `contract_id`, `events`, optional `version`, `disposition`, `dry_run` |
| Default | `dry_run: true`, `disposition: block` |
| Returns | The response body, including non-2xx outcome bodies, so the agent can repair the payload from `violations` |
| Hint | `destructiveHint` only when `dry_run` is false (live calls write audit and quarantine) |

`validate_events` stays the ingest tool. Do not overload it with a
direction flag. Agents confuse flags; a separate tool name is the door.

Document the tool in `docs/mcp-reference.md`. Bump the package version in
the same change as `mcp/server.json` and `package.json` `mcpName`, per the
existing publish rule.

### 4. Python SDK

Add `egress()` on the sync client and `egress()` on the async client, next
to `ingest()`. Arguments mirror the query string: `contract_id`, `events`,
`disposition`, `dry_run`, `version`. Return the decoded response. No new
dependency. Unit-test with a mocked transport the way `ingest()` is tested.

The local validator is unchanged. It has no direction. Callers who are
offline keep using `Contract.compile().validate()`.

### 5. Dashboard

Move the egress check onto the contract the user already has open.

- On the contract page, an "Outbound" check: paste or drop a JSON payload,
  choose disposition, dry-run by default, show `passed` / `failed`, the
  per-record violations, and the cleaned `payload` the caller would
  forward. This is the demo of the door.
- Leave the catalog tab in place so existing links keep working. Stop
  treating it as the home. The catalog subtitle and help copy
  (`dashboard/lib/help/catalog.ts`, ids `page.catalog` and `catalog.egress`)
  should describe the catalog as the import surface, and point the egress
  explanation at the contract page.
- Do not add a top-level nav item. Do not touch the leakage-mode control
  on the YAML tab (RFC-030).

Empty state: if the contract has no stable version, say so and link to
deploy. Do not invent a second builder UI. "Generate from sample" already
exists; the outbound check's empty state tells the user to infer from
known-good outbound samples, then come back.

### 6. README

One short paragraph after the ingest pitch. Not a new headline. State:
the same contract can gate what leaves, the endpoint is
`POST /egress/{contract_id}`, failing rows are blocked and declared PII
is masked, and the caller forwards the returned payload. Link
`docs/egress-validation-reference.md`. Keep `make demo` and the agent
paste block where they are.

## Implementation checklist

1. Playbook section, `llms.txt` line, egress reference additions, drift test.
2. `egress_validate` MCP tool, reference doc, package version bump, `npm test`.
3. Python SDK `egress()` on both clients, with tests.
4. Contract-page outbound check; catalog tab retained; help copy updated.
5. README paragraph.
6. `cargo test` for the doc drift gate. No engine change is in scope, so
   no new validation tests unless a response field the playbook names is
   not actually returned.

## Success

An agent that has only the playbook and the MCP server can infer a contract
from known-good outbound samples, dry-run a bad payload, and wire the
caller to forward `payload`. A person can do the same check from the
contract page without opening the catalog. Ingest behavior and the
validation hot path are unchanged.

## Out of scope follow-ups

- CLI `cg egress` against a deployed contract. Local `cg test` already
  validates a file with no direction.
- Quarantine list filtered by `direction`.
- A worked outbound example in the stream demo. Worth doing after the
  contract-page check exists, not before.
