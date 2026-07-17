# Worklist 2026-07-16 — for Sonnet — RFC-079 unify inference on Rust engine

Full rationale + design: [`docs/rfcs/079-unify-inference-on-rust-engine.md`](../rfcs/079-unify-inference-on-rust-engine.md).
Frontend-only. One branch/PR. **Do WS1 only** (Workstream 2 = RFC-080, separate worklist).

## Ground rules (CLAUDE.md + session-learned)

- **NO git operations of any kind.** Do not `git add/commit/branch/checkout/push`. Alex handles all git. Just edit files in the working tree and report.
- Be ultra-concise in any doc/comment you write. Comment only where behavior isn't obvious.
- **Test before declaring done:** `cd dashboard && npm run build` (tsc + build) must pass — CI gates on this. No cargo impact (no Rust change).
- Preserve existing behavior: flat-sample inference must stay identical; only nested-sample inference changes (broken → correct).
- Update `MAINTENANCE_LOG.md` (append a dated entry) when done.

## Context

Two inference engines exist. The Rust engine (`POST /contracts/infer`, already live, 10 MB cap, returns `{ yaml_content, field_count, sample_count }`) handles nested objects correctly. The client-side JS inferrer (`dashboard/app/contracts/_lib.tsx`) has no object/array branch, so nested payloads stringify to `"[object Object]"` and produce a contract that rejects every real record. Goal: route Generate-from-Sample through the Rust engine and delete the JS inferrer.

## Steps

1. **`dashboard/lib/api.ts`** — add `inferSamples()` mirroring the existing `inferCsv`:
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
   Goes through `apiFetch` so it carries the Bearer JWT like every other authed call. Match `inferCsv`'s exact style.

2. **`dashboard/app/contracts/page.tsx` `GeneratorTab`** (~line 1104) — replace the client-side `buildYaml(name, inferFields(records))` with an async `inferSamples({ name, samples: records })` call; set `generatedYaml` from `response.yaml_content`. **Parse the pasted JSON locally first** (so JSON syntax errors stay instant/local), then send the parsed `samples` array. Handle the async loading + error states consistent with how CSV inference already does it in this file. Add a one-line notice near the paste box:
   > *Sample data is sent to ContractGate to generate the contract. To keep data fully local, use Start Blank (YAML editor) or `cg test` locally.*

3. **`dashboard/app/contracts/_lib.tsx`** — delete `inferFields`, `buildYaml`, and `sniffPattern`. **Verify first** that only `GeneratorTab` imported them (VisualBuilder.tsx and WorkbenchClient.tsx have their own local copies per the RFC — confirm with grep before deleting). Update the `page.tsx` import list and the `_lib.tsx` header comment. If `_lib.tsx` becomes empty/near-empty, leave a short header note rather than deleting the file unless nothing else imports it.

## Acceptance

- `cd dashboard && npm run build` passes (catches removed-import / dead-code).
- Flat sample → same contract as before (unchanged).
- The RAG-style nested sample `{"text":"...","_cg":{"source":"...","doc_id":"...","ingested_at":123,"pii_redacted":true}}` → contract with `_cg` as `type: object` + nested `properties` (previously `enum: ["[object Object]"]`).
- Privacy notice renders on the Generate-from-Sample panel.
- No behavior change to CSV/URL inference (already on Rust).

## Do NOT

- Touch `src/infer.rs` or any Rust — the engine is already correct.
- Add a new backend route — `POST /contracts/infer` already exists.
- Start RFC-080 (Visual Builder) — that's a separate worklist.
- Run any git command.
