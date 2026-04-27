# Chunk 7 — Templates + Marketplace

**Theme:** Network effect. Reusable contract starters, public + private.
**Why now:** Lands after CLI + SDKs. Templates need a registry API SDKs/CLI can pull from.

## Items

- [ ] Public contract template library `[M]` — REST, event, gRPC, dbt model starters.
- [ ] Versioned template registry API `[M]` — SDKs/CLI pull templates programmatically.
- [ ] In-app template browser with search + one-click import `[M]`.
- [ ] Template submission pipeline `[M]` — lint, test, review, publish workflow.
- [ ] Organization-private template namespaces `[S]` — internal reusable patterns.
- [ ] Community template ratings & usage stats `[S]`.

## Hard dependency

- Chunk 2 (CLI/registry API surface) and Chunk 6 (SDK fetch path) must exist for programmatic pull to work.

## Surface to reuse

- Versioning store — templates are contracts under the hood.
- Existing dashboard tabs — browser is a new tab pattern.

## Open questions for the conversation

1. Hosting: separate registry domain or part of main API?
2. Submission gate: human review only, automated lint + human approve, or fully open?
3. Private namespace = scoped under org id from RFC-001, or distinct concept?
4. Ratings — anonymous thumbs, signed-in only, or weighted by usage?
5. RFC required (registry is a long-lived public contract).

## Suggested first step

Seed the public library with 5–10 starters, hosted as plain Git repo + S3. Build the registry API around that minimum. Defer browser UI until pull-via-CLI works end to end.
