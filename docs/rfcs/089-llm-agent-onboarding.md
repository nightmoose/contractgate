# RFC-089 — LLM-pasteable onboarding (`/llms.txt` + agent integration playbook)

**Status:** Draft
**Date:** 2026-08-06
**Branch:** nightly-maintenance-2026-08-06-rfc089
**Depends on:** RFC-005 (Python SDK), RFC-006/035/037 (inference), RFC-028 (deploy-contract), RFC-039 (auth), RFC-078 (walkthrough spine)

---

## Problem

User feedback, verbatim:

> "the easiest way to reduce friction for me is if i can just paste it into claude
> where it has the details and it can implement it for me. so like an article with
> the github repo."
>
> Example of what they actually paste: two URLs + `and implement ty`.

ContractGate has no artifact that survives that workflow. Every current entry
point is human-shaped:

| Surface | What an agent gets when it fetches this |
|---|---|
| `README.md` (GitHub raw) | Docker-demo-first. `make demo` stands up a *local* stack — it never produces a cloud account, a key, or wiring in the user's own repo. |
| `docs/quickstart-5min.md` | Correct and cloud-shaped, but not linked from any stable public URL, and step 1 hand-rolls escaped YAML inside a `curl` body — an agent will copy that escaping instead of writing a real contract file. |
| `app.datacontractgate.com/docs/*` | React pages (`app/docs/page.tsx`, `python-sdk`, `kafka-connect`). A plain fetch returns a JS shell, not the instructions. |
| `/openapi.json` | Machine-readable, but it's a route inventory, not a plan. It does not tell an agent *what order* to do things in, or that a contract must exist before ingest. |

Net effect: the human is the compiler. They read, translate, and then instruct
the agent. That is exactly the friction the feedback names.

## Goal

**One URL.** Pasted into Claude / Cursor / Codex with nothing but "implement
this", the agent wires ContractGate cloud validation into the user's repo
without further human input beyond supplying an API key.

**Done means:** starting from a repo the agent has never seen, with
`CONTRACTGATE_API_KEY` in the environment, the agent produces —

1. `contracts/<name>.yaml` committed, in the locked semantic-contract format;
2. that contract deployed as a `stable` version (real `contract_id` in hand);
3. the user's producer calling `POST /v1/ingest/{contract_id}`;
4. a dry-run verification that a known-bad event is rejected.

## Non-goals

- **MCP server / Claude skill / plugin.** Follow-up (RFC-090). This doc is the
  prerequisite content for all three — write it once, wrap it later.
- **Self-hosted path.** Deliberately excluded: the paste flow's job is to end
  with events flowing into an account. `make demo` stays the README's answer for
  "no signup". `/llms.txt` gets exactly one line pointing at it so agents don't
  wander.
- **Rewriting `quickstart-5min.md` or the `/docs` React pages.** Both stay.
- **Any change to the validation engine.** Zero Rust changes on the hot path.

## Design

### 1. Two files, one source of truth

| File (canonical, in repo) | Served at | Purpose |
|---|---|---|
| `docs/llms.txt` | `https://app.datacontractgate.com/llms.txt` | ~30-line index: what ContractGate is, the repo URL, and links to the playbook + reference docs. Follows the llmstxt.org convention. |
| `docs/llm-integration.md` | `https://app.datacontractgate.com/llm-integration.md` | The self-contained playbook. This is the URL that gets pasted. |

Serving mechanism: a prebuild step copies both into `dashboard/public/`, so
Vercel serves them as **static files — no auth, no JS, no redirect**.

```
dashboard/package.json:  "prebuild": "node scripts/sync-llm-docs.mjs"
dashboard/.gitignore:    public/llms.txt
                         public/llm-integration.md
```

Rationale for copy-at-build rather than a Next route handler: `docs/` sits
outside `dashboard/`, so a runtime `fs.readFile` is not guaranteed to be in the
Vercel bundle. Copies are gitignored, so the repo file is unambiguously the
source of truth and cannot drift.

`Content-Type: text/plain; charset=utf-8` via `dashboard/vercel.json` headers,
so browsers show it raw and agents don't get HTML-escaped markdown.

### 2. Playbook content contract

`docs/llm-integration.md` is written **to the agent, in the imperative**, and
must contain these sections in this order. Ordering is load-bearing: it encodes
the dependency chain (key → sample → contract → deploy → wire → verify).

**§0 Preconditions.** Repo URL (`github.com/nightmoose/contractgate`), base URL,
and: get a key at `app.datacontractgate.com/account`, export it as
`CONTRACTGATE_API_KEY`. Explicit instruction: *never inline the key in code, a
config file, or a commit — read it from the environment.*

**§1 Find the event shape.** What to grep for in the user's repo: producer
calls, `json.dumps` / `JSON.stringify` payloads, Kafka `produce`, warehouse
`INSERT`, webhook handlers. Collect 5–20 **real** sample events.

**§2 Infer the contract.** `POST /contracts/infer` with the samples, header
`X-Api-Key`. Body cap 10 MB (RFC-043). Format-specific variants listed
(`/infer/csv`, `/infer/url`, `/infer/avro`, `/infer/proto`, `/infer/openapi`).
Returns contract YAML.

**§3 Write the contract file.** The locked YAML format inlined **verbatim** from
`CLAUDE.md` — this is the single most important part of the doc, because it is
what stops an agent inventing fields. Plus a field table: `type`, `required`,
`pattern`, `enum`, `min`/`max`, `glossary`, `metrics`. Instruction to review the
inferred YAML against the real domain (tighten enums, add patterns) rather than
committing the inference blindly.

**§4 Deploy it.** `POST /contracts/deploy` (or `cg deploy-contract <FILE>
--source <feed> --deployed-by <ci>`). States the semantics that matter: promotes
straight to `stable`, deprecates prior stable, `409` on duplicate
`(name, version)`, refuses while pending quarantine events exist. Capture the
returned `contract_id`.

**§5 Wire the producer.** `POST /v1/ingest/{contract_id}`, `X-Api-Key`,
`application/json` (array or single object) or `application/x-ndjson`; optional
`Idempotency-Key` (24 h window); query params `version`, `dry_run`, `atomic`.
Copy-paste snippets for TypeScript (`fetch`), Python (first-party SDK, RFC-005),
and `curl`.

**§6 Verify.** Send one known-good and one known-bad event with
`?dry_run=true`; assert the bad one comes back with violations and the good one
clean. Then drop `dry_run`. Dry-run-first ordering means a wrong guess by the
agent cannot pollute the audit log, quarantine, or metered usage.

**§7 Confirm in the dashboard.** Where to look: contract Audit tab, Quarantine
tab, Usage.

**§8 Response + error table.** `401` (missing/invalid key), `403` (key not
authorized for that contract — per-key `allowed_contract_ids`, RFC-065), `409`
(version exists), `413` (over body cap), validation-violation response shape.

**§9 Do not.** Don't invent contract fields not present in the samples. Don't
"fix" a failing validation by disabling the gate. Don't commit keys. Don't use
`/playground/validate` for production wiring — it's the UI's scratchpad.

### 3. Discoverability

- **README:** one line at the top of "Try it in 10 minutes" — *"Using Claude,
  Cursor, or Codex? Paste this: `https://app.datacontractgate.com/llm-integration.md`
  + `github.com/nightmoose/contractgate` and say 'implement this'."*
- **`/docs` page:** third card, linking to the raw `.md` (not a React wrapper).
- **Dashboard, immediately after key creation:** a **Copy paste-prompt** button
  emitting the exact two-line prompt. It references `$CONTRACTGATE_API_KEY` —
  it must **never** interpolate the raw key into text destined for a chat window.

### 4. Drift gate

A doc that agents execute is an API surface, and it decays like one. Two cheap
CI gates:

1. **Endpoint existence.** `tests/llm_docs_test.rs` — extract every
   `` `METHOD /path` `` from `docs/llm-integration.md`, normalize path params,
   assert each exists in the router (build the app and read `/openapi.json`).
   Fails the build the moment the doc cites a route that no longer exists.
2. **Example validity.** The `§3` contract YAML must parse and validate through
   the real engine (`cg test`, RFC-076), reusing the RFC-078 example harness.

Both run in the existing test lane; neither touches the hot path.

### 5. Security

- Public, unauthenticated, cacheable, contains **no tenant data**.
- No secrets anywhere in the doc; key only via env var; explicit don't-commit rule.
- The doc is a set of instructions a coding agent will execute with tool access,
  so it stays scoped to ContractGate endpoints and the user's own repo: no
  third-party fetches, no `curl | sh`, no credential-reading steps beyond one
  named env var.

## Implementation checklist

1. `docs/llm-integration.md` — the playbook, §0–§9 above.
2. `docs/llms.txt` — index (+ single self-host pointer line).
3. `dashboard/scripts/sync-llm-docs.mjs` — copy both into `public/`; fail loudly
   if a source file is missing.
4. `dashboard/package.json` — add `prebuild`.
5. `dashboard/.gitignore` — ignore the two copies.
6. `dashboard/vercel.json` — `text/plain; charset=utf-8` headers for both paths.
7. `README.md` — paste-prompt line.
8. `dashboard/app/docs/page.tsx` — third card.
9. Dashboard account/key screen — **Copy paste-prompt** button (env-var reference only).
10. `tests/llm_docs_test.rs` — endpoint drift gate.
11. `cargo check && cargo test`; `cd dashboard && npm run build`.

Files 1–2 are the deliverable; 3–6 make the paste flow physically work; 7–9 are
distribution; 10 keeps it true.

## Success metrics

- **Primary:** share of new orgs whose first successful `/v1/ingest` lands
  within 24 h of key creation (measurable today from existing usage rows).
- Fetch count on `/llms.txt` and `/llm-integration.md` (Vercel analytics; agent
  user-agents are distinguishable).
- Drop in "how do I integrate" support threads.

## Open questions

1. **Host on `app.*` or a new `docs.*` subdomain?** Recommend `app.*`: fewer DNS
   and edge moving parts, and it's already where the key lives. Revisit only if
   marketing wants docs off the app domain.
2. **Should `/llms.txt` enumerate every reference doc in `docs/`?** Recommend
   no — index the playbook plus the four highest-value references
   (`v1-ingest`, `deploy-contract`, `csv-inference`, `quarantine-replay`).
   A 40-link index buries the one link that matters.
