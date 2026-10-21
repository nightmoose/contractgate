# RFC-095 — AISchemaGate Documentation Portal (`aischemagate.dev`)

**Status:** Draft
**Date:** 2026-10-11
**Depends on:** RFC-089 (LLM-pasteable onboarding — `llm-integration.md` + `llms.txt`), RFC-090 (MCP server), RFC-093 (egress as first-class door), RFC-094 (LLMGate SDK + aischemagate alias package)
**Companion RFCs:** RFC-094 (SDK), RFC-096 (frontend + DNS + CORS)

---

## Problem

The AISchemaGate dev brief (2026-10-11) requires a developer documentation
portal at `aischemagate.dev` that teaches:

- OpenAI structured outputs + AISchemaGate
- Anthropic tool-calling + AISchemaGate
- LangChain / CrewAI / MCP integrations

The portal must present the AISchemaGate brand (new frontend, RFC-096) while
sourcing from the same canonical documentation this repo already maintains.
RFC-089 established the pattern: markdown files in `docs/` are the canonical
source; a prebuild step copies them to the serving surface; a drift test
(`tests/llm_docs_test.rs`) asserts every endpoint the docs cite still exists
in the router.

What this RFC must add:

1. A new canonical doc, `docs/llm-output-validation.md`, written to a
   developer (not an agent) who arrives from the AISchemaGate angle. The
   RFC-089 playbook stays as-is for the data-ingest audience.
2. Quickstarts for each provider lane the brief lists (OpenAI structured
   outputs, Anthropic tool calling, LangChain, CrewAI, MCP). Each is a
   10–20-line snippet in Python and TypeScript against the RFC-094
   `LLMGate` class imported from the `aischemagate` alias package.
3. A docs-portal routing plan so `aischemagate.dev` serves the content.
   Options and recommendation in §Design 2.
4. An expansion of `tests/llm_docs_test.rs` to cover the new doc and the
   `LLMGate` SDK snippets (compile-check contract YAML, assert cited
   endpoints exist, assert `LLMGate` has every method the snippets call).
5. A locked **authorized marketing-copy** table for aischemagate.com with
   authorized / modified / forbidden verdicts for every hero and
   feature-grid line in the brief.

## Goal

A developer who lands on `aischemagate.dev` has one page per LLM lane,
each with a runnable Python and TypeScript quickstart that pulls the
`aischemagate` alias package shipped in RFC-094 and routes all enforcement
through the shared backend. Marketing copy on `aischemagate.com` is locked
to a truth table in Appendix A so claims cannot drift from shipped
behavior.

**Done means:**

1. `docs/llm-output-validation.md` exists in this repo as the canonical
   source. It covers §0 preconditions, §1 output-contract authoring,
   §2 one-call validate, §3 auto-repair, §4 prompt-side masking,
   §5 provider-specific quickstarts (OpenAI / Anthropic / LangChain /
   CrewAI / MCP), §6 verification, §7 troubleshooting.
2. The RFC-089 playbook (`docs/llm-integration.md`) gets a one-line
   pointer at the end of §0: "If your job is to validate an LLM's JSON
   output rather than wire up data ingestion, switch to
   `llm-output-validation.md` now."
3. `docs/llms.txt` has a new "Start here" entry linking to the output-
   validation playbook, hosted under `aischemagate.dev/llm-output-validation`
   (or wherever RFC-096's build settles).
4. `tests/llm_docs_test.rs` compiles any example YAML in the new doc and
   asserts every endpoint, MCP tool, and `LLMGate` method the doc cites
   exists on the shipped surface.
5. `README.md` gets one short paragraph after the RFC-093 egress paragraph
   that points at AISchemaGate and the new doc.
6. Appendix A of this RFC locks every hero / feature-grid line in the dev
   brief as authorized / modified / forbidden, with the RFC cite that
   justifies each verdict.

## Non-goals

- **A rewrite of the RFC-089 agent playbook.** `llm-integration.md` is the
  canonical ingest playbook and stays as-is. This RFC adds a sibling
  playbook; it does not fork or replace the existing one.
- **A rebrand of `contractgate` → `aischemagate`.** The core package stays
  `contractgate`. `aischemagate` is a sister alias (RFC-094 §Non-goals).
- **A new tenant service, auth model, or API-key issuance path.** Shared
  with DataContractGate (RFC-056, RFC-039). RFC-096 covers the CORS
  allowlist change.
- **A new "AI Guardrails" SKU or plan tier.** RFC-045 (plan gating) is
  the surface for pricing changes. Not in scope.
- **A standalone quickstart repo.** The quickstarts live inside the
  playbook. If they outgrow it, that is a follow-up.
- **A LangChain / CrewAI adapter package.** v1 ships cookbook snippets in
  the playbook (RFC-094 §Non-goals). Dedicated adapters would be their own
  RFC.
- **New engine code, new endpoints, new SDK code.** All code work is in
  RFC-094. This RFC is docs + marketing-copy spec + drift gate extension.
- **Owning the aischemagate.com marketing content in this repo.** The
  marketing surface lives in the new frontend repo under RFC-096. This
  RFC owns the *authorized copy table*; whoever ships the site applies it.

## Design

### 1. New doc: `docs/llm-output-validation.md`

Structure, written to a developer (not an agent — RFC-089's playbook keeps
the agent-imperative voice):

```
§0  Install                        pip install aischemagate / npm i aischemagate
§1  Author an output contract      3 sample JSON → cg contracts infer → deploy
§2  Validate one output             LLMGate.validate_only in 10 lines (Py + TS)
§3  Validate with auto-repair      LLMGate.completion(auto_repair=True)
§4  Mask the prompt                LLMGate.mask_prompt + RFC-004 transforms
§5  Provider quickstarts           OpenAI, Anthropic, LangChain, CrewAI, MCP
§6  Verify                         One known-bad, one known-good; expect 1/0
§7  Troubleshooting                Shape errors, streaming, forbidden footguns
```

Each §5 lane is a tabbed Python/TS block with a header note stating:

- **OpenAI structured outputs** — `response_format={"type":"json_object"}`;
  `LLMGate` picks the content string and `json.loads` it.
- **Anthropic tool calling** — `tools=[{...}]`, `tool_choice={"type":"tool",
  "name": "..."}`; `LLMGate` picks the first `tool_use` block and reads
  `.input`.
- **LangChain** — use `LLMGate.validate_only(resp)` after your chain
  returns. Snippet imports `langchain_openai.ChatOpenAI` and calls
  `.invoke`, then passes the AIMessage to `validate_only`.
- **CrewAI** — same pattern. The crew returns a task output; pass it
  through `validate_only`.
- **MCP** — tells the reader to use the shipped `egress_validate` MCP tool
  (RFC-093); `LLMGate` is unnecessary because the agent already has the
  JSON object. Links to `docs/mcp-reference.md`.

Rule for each snippet: **under 20 lines**, no filler, uses only `LLMGate`
+ one provider SDK + one `import os` for the API key. No `print` boilerplate.

### 2. Serving routes — where `aischemagate.dev` reads from

Three options. Draft recommendation: **Option B.**

**Option A — Same prebuild as RFC-089.** The frontend's `prebuild` step
copies `docs/llm-output-validation.md` into `public/` of the new
AISchemaGate frontend (RFC-096). The frontend serves it as a raw markdown
file at `aischemagate.dev/llm-output-validation`. Pros: zero new tooling,
mirrors RFC-089. Cons: raw markdown is readable but not pretty; no
tabbed Python/TS views without extra glue.

**Option B — Rendered portal under `aischemagate.dev/docs`.** The new
frontend (RFC-096) imports `docs/llm-output-validation.md` as content and
renders it with Next.js MDX + a shared `<CodeTabs>` component. The raw
markdown is still served at `aischemagate.dev/llm-output-validation.md`
for the agent playbook pattern (so `/llms.txt` can link to a raw file).
Pros: tabbed snippets render as tabs, syntax highlighting, cross-links.
Cons: the frontend owns the renderer; this RFC specifies the render
contract.

**Option C — Standalone docs host (Mintlify / Docusaurus).** New tool,
new repo integration. Rejected unless the brief later requires it; too
much net-new tooling for one playbook.

Design for Option B:

- Canonical source: `docs/llm-output-validation.md` in this repo.
- The AISchemaGate frontend repo (RFC-096) adds a build-time step that
  pulls this file (either via git-submodule-free `curl` from a tag, or a
  committed snapshot updated by a small script). The pull mechanism is
  RFC-096's problem; this RFC only specifies the file name and the
  rendered path.
- Rendered path: `aischemagate.dev/docs/llm-output-validation`.
- Raw path: `aischemagate.dev/llm-output-validation.md` (served from
  the same source, no MDX).
- `aischemagate.com/docs` 301s to `aischemagate.dev/docs` (RFC-096).

### 3. Drift gate extension (`tests/llm_docs_test.rs`)

The RFC-089 test walks `docs/llm-integration.md` for endpoint drift.
Extend it to:

- Open `docs/llm-output-validation.md`.
- Grep for `POST /egress/{contract_id}`, `POST /contracts/infer`,
  `POST /contracts/deploy` and assert each exists in the router.
- Grep for MCP tool names (`egress_validate`) and assert the MCP server's
  tool list contains each.
- Grep for `LLMGate.` method calls (`validate_only`, `completion`,
  `mask_prompt`) and assert each is a public method on `LLMGate` in
  `contractgate.llm`. This requires parsing the Python module statically
  or shelling to `python -c "import contractgate.llm; …"` in the test. The
  simpler path: a sidecar `sdks/python/tests/test_llm_gate.py` test
  asserts the method names, and this Rust test only checks that the
  Python file exists and contains the strings — docs assert code, SDK
  tests assert surface.
- Compile every example YAML in the doc with `cg_core::Contract::compile`.

### 4. README paragraph

Add after the RFC-093 "Validate what leaves" paragraph:

> **AISchemaGate.** The same engine powers AISchemaGate
> (`aischemagate.com`), the LLM-and-agent surface of the engine DataContractGate and AISchemaGate share.
> `pip install aischemagate` or `npm i aischemagate` to get the `LLMGate`
> wrapper over OpenAI / Anthropic / Ollama / vLLM. Validates structured
> output against your contract via the shipped `/egress/{contract_id}`
> endpoint. Non-streaming only in v1.

### 5. `docs/llms.txt`

Insert after the existing `llm-integration.md` "Start here" entry:

```
- [LLM output validation playbook](https://aischemagate.dev/llm-output-validation.md): Paste this when the job is to validate an LLM's JSON output (OpenAI / Anthropic / MCP). Picks up where the integration playbook leaves off.
```

### 6. Markdown render contract (Option B specifics)

RFC-096 renders the markdown. The contract between RFCs:

- Fenced code blocks tagged `python` and `typescript` adjacent to each
  other render as a two-tab `<CodeTabs>` block. Default tab is Python.
- All relative links in the markdown resolve against
  `https://github.com/nightmoose/contractgate/blob/main/`.
- All API URLs in snippets point at `https://app.datacontractgate.com`
  (the shared backend; AISchemaGate's domain is marketing-only).
- Front-matter in the markdown (if added later) is reserved for the
  frontend's use; this RFC does not require it.

### 7. Appendix A — Authorized marketing copy for `aischemagate.com`

The dev brief proposes specific hero / sub-headline / feature-grid
wording. This appendix locks each line as **authorized** (ships as
written), **modified** (ships with the change here), or **forbidden**
(must not ship, with the RFC cite that forbids it). Whoever renders
`aischemagate.com` (RFC-096) applies the authorized/modified set.

#### Hero

| Brief copy | Verdict | Authorized text |
|---|---|---|
| "Stop LLMs & AI Agents from Outputting Bad JSON." | authorized | same |
| "Patent-Pending, Sub-15ms Semantic Enforcement Engine for AI Outputs. Enforce strict JSON schemas, eliminate hallucinated values, and sanitize PII before bad data hits your application logic or database." | modified | "Patent-Pending Sub-15ms Semantic Enforcement Engine for AI Outputs. Enforce strict JSON schemas, catch out-of-range and off-enum values the contract declares, and sanitize PII before bad data hits your application logic or database." (Replaces "eliminate hallucinated values" with the narrower, truthful claim; see RFC-093 §Non-goals and RFC-094 §Non-goals.) |

#### Feature grid

| Brief copy | Verdict | Authorized text |
|---|---|---|
| "Sub-15ms Output Gate: Real-time semantic validation of LLM outputs." | authorized | same (figure matches `/llms.txt`: ~31 µs per-event p99; ~86k events/sec/core). |
| "Auto-Infer Contracts: Generate schema contracts automatically from 3 sample JSON outputs." | modified | "Auto-Infer Contracts: Generate schema contracts automatically from 5–20 known-good JSON outputs." (Three under-samples and misses patterns; RFC-093 recommends 5–20.) |
| "Auto-Repair & Replay: Intercept malformed payloads and run automated micro-fixes or re-prompts." | modified | "Auto-Repair: Re-prompt automatically up to a bounded retry budget when validation fails. Replay quarantined events against a corrected contract version." (Split so each clause is accurate — repair = RFC-094 re-prompt loop, replay = pre-existing RFC-003.) |
| "Boundary PII Masking: Strip sensitive data before sending prompts out or storing model responses." | authorized | same (RFC-004 covers the storage side; RFC-094 `mask_prompt` covers the prompt side). |

#### Forbidden claims (do not ship on aischemagate.com)

- "Eliminate hallucinations." — The contract catches only what it can
  express; semantic truth outside the schema is a boundary problem
  (RFC-093 §Non-goals, RFC-094 §Non-goals).
- "Streaming LLM response validation." — v1 rejects streaming
  (RFC-094 §Non-goals).
- "Zero-latency." — Low-latency, not zero. Use the figures from `/llms.txt`.
- "Format-preserving encryption / tokenization." — RFC-004 §Non-goals.
- "Calls OpenAI / Anthropic for you" (unless Q1 of RFC-094 lands on
  Option A). — The caller owns the provider call under Option B.
- "AISchemaGate replaces DataContractGate." — Shared backend, two front
  doors. Marketing must not imply replacement; RFC-096 is explicit.

### 8. Open-question dependencies on RFC-094

Several copy decisions here depend on how RFC-094 Q1 resolves (does
`LLMGate.completion` call providers?). If Q1 lands on **A** (LLMGate calls
providers), the "Calls OpenAI / Anthropic for you" claim moves from
forbidden to authorized, and the §5 provider quickstarts drop the client-
construction lines. If Q1 lands on **B** (caller binds client), the claim
stays forbidden and the snippets include the one `openai_client = OpenAI()`
line. Appendix A will be regenerated after Q1 is signed off.

## Implementation checklist

1. Write `docs/llm-output-validation.md` to the structure in §Design 1.
2. Add the one-line pointer at the end of `docs/llm-integration.md` §0.
3. Update `docs/llms.txt` with the new "Start here" entry.
4. Extend `tests/llm_docs_test.rs` per §Design 3.
5. Add the README paragraph per §Design 4.
6. Write `docs/llm-sdk-reference.md` (if not already shipped under
   RFC-094 — the two RFCs can share the file).
7. Appendix A stays in this RFC. The AISchemaGate frontend repo (RFC-096)
   mirrors it as `MARKETING_COPY.md` and reads it as the source of truth.
8. STATUS.md and MAINTENANCE_LOG updates after merge.
9. No Rust / dashboard / migration changes.

## Test plan

1. `cargo test tests::llm_docs_test` — passes against main after the
   assertion additions.
2. `cargo test` — no regressions.
3. `pytest sdks/python/tests/test_llm_gate.py` — covers the SDK surface
   the docs assert exists (owned by RFC-094).
4. `npm test` in `sdks/typescript/` — mirror of (3).
5. `curl https://aischemagate.dev/llm-output-validation.md` returns the
   raw doc after RFC-096 deploys.
6. Visual smoke: the rendered portal at
   `aischemagate.dev/docs/llm-output-validation` has working Python/TS
   tabs on each snippet.
7. Marketing-copy diff: every line of hero / feature-grid copy on
   `aischemagate.com` is diffed against Appendix A. Any claim not in
   Appendix A is removed before publish.

## Rollout

1. Accept RFC-094, RFC-095, RFC-096 as a set (product launch, not
   isolated features).
2. RFC-094 lands first (SDK code). The doc snippets must type-check
   against the shipped `LLMGate` before this RFC merges.
3. RFC-095 docs land next on this branch.
4. RFC-096 ships the frontend and wires the renderer; the doc is live on
   `aischemagate.dev` after its deploy.
5. STATUS.md gains three new rows (094 / 095 / 096). MAINTENANCE_LOG
   captures the launch.

## Decisions (signed off 2026-10-11)

- **Appendix A copy table: all narrowed claims and edits approved.**
  "Eliminate hallucinated values" → "catch out-of-range and off-enum
  values the contract declares"; "3 sample outputs" → "5–20 known-good
  outputs"; "Auto-Repair & Replay" split into two accurate clauses.
  Forbidden-claims list stands.
- **Q1 → B (rendered portal under `aischemagate.dev/docs`).** RFC-096
  frontend owns the renderer; this repo owns the canonical markdown.
- **Q2 → `aischemagate.dev` served by the new frontend** (same Vercel
  project as `aischemagate.com`, routed per-domain). No `.dev` → `.com`
  301.
- **Q3 → Doc pull via tagged release + sync script** in the frontend repo.
  Weekly cron plus manual trigger. Human reviews each sync PR.
- **Q4 → Marketing-copy ownership: Appendix A here is authoritative,**
  mirrored in `aischemagate-www` as `MARKETING_COPY.md`. Any addition
  requires a PR against this RFC first.
- **Q5 → LangChain / CrewAI are cookbook snippets only** in v1. Dedicated
  adapter packages are a follow-up RFC if adoption warrants it.

Note: `LLMGate` method naming (completion / message) depends on RFC-094
Q1 → B. Appendix A copy stays as locked above because B keeps the
"Calls OpenAI / Anthropic for you" claim on the forbidden list.

Original open-question text retained below as a record.

## Open questions (resolved — see Decisions above)

- **Q1 — Portal rendering** (Options A / B / C in §Design 2): draft
  recommends B. Confirm.
- **Q2 — `aischemagate.dev` DNS target**: is this a Vercel / Netlify
  deploy of the new frontend (RFC-096), or does `.dev` 301 to
  `aischemagate.com/docs`? Draft assumes the frontend serves both
  domains. Confirm.
- **Q3 — Doc-pull mechanism** between this repo and the AISchemaGate
  frontend: tagged release + script, submodule, or periodic commit? Draft
  recommends tagged release + a small script in the frontend repo.
- **Q4 — Marketing-copy ownership**: Appendix A here is authoritative,
  mirrored in RFC-096's frontend repo as `MARKETING_COPY.md`. Any
  marketing addition to `aischemagate.com` requires a PR to this RFC
  first. Confirm.
- **Q5 — LangChain / CrewAI snippets**: cookbook-only in v1 (not adapter
  packages). Confirm.

Close Q1–Q5 before docs merge, per RFC-004 / RFC-093 convention.
