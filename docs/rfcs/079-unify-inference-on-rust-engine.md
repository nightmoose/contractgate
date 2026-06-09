# RFC-079 — Unify contract inference on the Rust engine

**Status:** Draft
**Date:** 2026-06-08
**Branch:** TBD
**Addresses:** Nested-object inference bug found during 2026-06-08 hosted-product
walkthrough (Generate-from-Sample emits `enum: ["[object Object]"]` for nested
payloads — breaks the RFC-077 RAG envelope).
**Severity:** P1 — headline onboarding feature produces a broken contract for
the RAG record shape we are about to promote.

---

## Problem

The dashboard has **two** contract-inference implementations:

1. **Rust** (`src/infer.rs`, `infer_fields_from_objects`) — behind
   `POST /contracts/infer`. Handles nested objects correctly (recurses into
   `FieldType::Object` → `properties`), arrays, pattern/enum/date detection.
   This is the tested, canonical engine. CSV (`/contracts/infer/csv`) and URL
   (`/contracts/infer/url`) inference already route through it.

2. **Client-side JavaScript** (`dashboard/app/contracts/_lib.tsx`,
   `inferFields` + `buildYaml`) — used *only* by the **Generate from Sample**
   tab (`page.tsx` `GeneratorTab`, line ~1104). It has three type branches:
   boolean, number/integer, else **string**. There is no object/array case.

### The bug

A nested value (e.g. the RFC-077 RAG envelope `{ text, _cg: {...} }`) falls
through the JS inferrer to `type: "string"`. `String(value)` on an object
yields the literal `"[object Object]"`; since both sample records stringify
identically, the `enum` heuristic fires and emits:

```yaml
- name: _cg
  type: string
  enum:
    - "[object Object]"
```

`buildYaml` has no `properties` support, so the nested envelope is lost
entirely. The panel marks this `✔ ready to edit & save` — a user can save and
deploy a contract that **rejects every real record** (no value equals the
literal string `"[object Object]"`).

This collides directly with the RAG wedge (RFC-077): the canonical RAG record
is nested, and the headline "paste your data → get a contract" feature cannot
infer it.

### Why two engines exist

Best understanding: the JS path was added to keep pasted sample data on the
client (privacy). But that guarantee is **already not held** — the CSV and URL
inference paths both POST sample data to the backend over the authenticated
API. So the JS inferrer is not enforcing a consistent privacy policy; it is an
undocumented inconsistency that also happens to be the buggy one.

## Goal

One inference engine everywhere (Rust), an explicit data-handling notice on the
panels that send samples, and a fully-local path preserved for users who cannot
send data at all.

## Non-goals

- No change to the Rust inference engine — it is already correct.
- No new backend route — `POST /contracts/infer` already exists, is routed with
  the 10 MB cap (`src/main.rs:1445`), and returns
  `{ yaml_content, field_count, sample_count }`.
- Not changing CSV/URL inference — already on the Rust engine.

## Workstream 1 — Consolidate Generate-from-Sample on the Rust engine (this RFC, build now)

1. **`dashboard/lib/api.ts`** — add `inferSamples()` mirroring `inferCsv`:

   ```ts
   export interface InferSamplesResponse {
     yaml_content: string;
     field_count: number;
     sample_count: number;
   }
   export const inferSamples = (params: {
     name: string;
     description?: string;
     samples: unknown[];
   }) => apiFetch<InferSamplesResponse>("/contracts/infer", {
     method: "POST",
     body: JSON.stringify(params),
   });
   ```

   Goes through `apiFetch`, so it carries the Bearer JWT like every other
   authenticated call.

2. **`dashboard/app/contracts/page.tsx` `GeneratorTab`** — replace the
   client-side `buildYaml(name, inferFields(records))` with an async
   `inferSamples({ name, samples: records })` call; set `generatedYaml` from the
   response. Parse the pasted JSON locally first (so JSON errors stay
   instant/local), then send the parsed `samples` array. Add a one-line notice:
   *"Sample data is sent to ContractGate to generate the contract. To keep data
   fully local, use Start Blank (YAML editor) or `cg test` locally."*

3. **`dashboard/app/contracts/_lib.tsx`** — delete `inferFields`, `buildYaml`,
   and `sniffPattern` (only `GeneratorTab` imported them; `VisualBuilder.tsx`
   and `WorkbenchClient.tsx` have their own local copies — verified). Update the
   `page.tsx` import list and the file-header comment.

4. **Result:** nested objects (the RAG envelope) infer correctly for free,
   because the Rust engine already recurses into `properties`. The
   `"[object Object]"` class of bug becomes structurally impossible — there is
   only one engine.

### Privacy posture after this change

- **Sends data (now consistent + disclosed):** Generate-from-Sample, CSV, URL —
  all POST samples to the backend over TLS, authenticated, with a visible
  notice.
- **Fully local (preserved):** **Start Blank** YAML editor (nothing leaves the
  browser), and `cg test` / `make demo` for local validation.

## Workstream 2 — Visual Builder nested-object support (Accepted, DEFERRED)

The Visual Builder (`dashboard/app/contracts/VisualBuilder.tsx`) cannot express
`type: object` at all: its `FieldType` union is
`"string" | "integer" | "number" | "boolean" | "date"`, its `FieldState` is a
flat model, and its `buildYaml` walks a flat field list. So the RAG envelope
cannot be built manually through the UI either — the only nesting-capable path
today is hand-written YAML in Start Blank.

Fixing this is a real builder rework: add `"object"` to the type union, give
`FieldState` recursive child `properties`, render nested field editors, and
make `buildYaml` recurse. That is a substantial, UI-heavy change — exactly the
"frontend overhaul" CLAUDE.md cautions against doing casually, and it needs
visual + `npm run build` verification that can't be done blind.

**Decision:** deferred to its own focused follow-up (proposed RFC-080) so
Workstream 1 — which fixes the actual reported bug and the RAG-onboarding
blocker — can ship cleanly and independently. Until then, the RAG walkthroughs
(RFC-077/078) hand users correct nested YAML, and Generate-from-Sample (post-WS1)
infers it correctly.

## Testing

- **Backend:** already covered — `src/infer.rs` nested-object tests exist.
  No backend change to test.
- **Frontend (maintainer-run):** `cd dashboard && npm run build` must pass after
  the api.ts + page.tsx + _lib.tsx changes (catches the removed-import / dead-code
  cases). Manual: paste a flat sample → contract (unchanged behavior); paste the
  RFC-077 RAG sample → contract with `_cg` as `type: object` + nested
  `properties` (the bug case, now correct). Confirm the privacy notice renders.
- No `cargo` impact (no Rust change).

## Rollout

Frontend-only (Vercel). No migration, no backend deploy, no API change. The
`POST /contracts/infer` route is already live. Backward compatible: flat-sample
inference is unchanged; nested-sample inference goes from broken to correct.
