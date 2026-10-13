# RFC-104 — AISchemaGate copy cleanup

**Status:** shipped 2026-10-13
**Driver:** 2026-10-13 10:00 ET in-app retest (`~/Desktop/ContractGate-E2E/RETEST-inapp-2026-10-13.md`). RFC-102 passed A6/A7/A8/B1/B2/B6; this is the follow-up for the remnants the tester caught.

## Problem (from the retest)

RFC-102 made the sidebar and header AISchemaGate for AISG accounts, but five surfaces still read "DataContractGate" or Kafka-oriented copy:

1. **Dashboard hero** (`app/page.tsx:36,46`) — "DataContractGate v0.1 … Streaming-compatible" shown for AISG users.
2. **Account page API-keys subtitle** (`app/account/page.tsx:503`) — "Use these in your Kafka connector config as `contractgate.api.key`".
3. **Account page empty-state** (`app/account/page.tsx:556`) — "Create one to start using the Kafka connector."
4. **Account page Kafka-Connect quick-reference section** (`app/account/page.tsx:988-1007`) — full connector properties block, with DCG-only `contractgate.*` keys.
5. **Account page Event Payload Storage** (`app/account/page.tsx:1036`) — "When on, **DataContractGate** retains the … body of each event."
6. **Generate from Sample** (`app/contracts/page.tsx:1160`) — "Sample data is sent to **DataContractGate** to generate the contract."
7. **Quarantine tab lock icon** (`app/contracts/page.tsx:1983`) — stale lock badge from pre-RFC-099 state; the tab is open on Free now (content-gated per-event by `payload_redacted`).
8. **`/usage` 404** — the tester typed the URL expecting a usage page. The usage widget already lives on `/account`; the Rust backend's `/usage` is an API endpoint, not a frontend page.

## Design

**Rule:** anywhere a user-facing string names the product, read `useBrand()` and switch. Anywhere a UI concept doesn't apply to a brand (Kafka Connect for AISG), hide it instead of translating it — Kafka ingestion is DataContractGate's product scope; translating it to "LLM calls" would mislead AISG users.

### What changed

| Surface | Treatment |
|---|---|
| Hero brand label + headline + pills | Branch on `aisg`. AISG gets its own tagline ("Validate every LLM response against a semantic contract before it reaches your app logic") and three pills focused on LLM output. |
| Account API-keys subtitle | AISG: pass as `api_key=` to `LLMGate` SDK, or `x-api-key` on `/egress/{id}`. DCG: unchanged. |
| Account empty-state copy | AISG: "start validating LLM output with the LLMGate SDK." DCG: unchanged. |
| Account Kafka-Connect section | Hidden for AISG via `{!aisg && (…)}`. DCG unchanged. The AISG onboarding (install the SDK) lives in the Docs sidebar entry, already AISG-aware per RFC-096. |
| Event Payload Storage copy | Lifted `useBrand()` into `EventPayloadStorageSection` and substituted `brandName` into the explainer. |
| Generate from Sample copy | Lifted `useBrand()` into `GeneratorTab` and substituted `brandName`. |
| Quarantine lock icon | Removed `"quarantine"` from the `growthOnly` tab list (both AISG and DCG branches) — now matches the actual gating from RFC-099. |
| `/usage` | New `app/usage/page.tsx` that server-redirects to `/account` (where the UsageWidget renders). |

### Deliberately not in scope

- **Hero "How it works" expanded panel** — defaults closed, still has DCG copy inside. Low-priority; the tester hasn't flagged it. Translate later if a user expands and complains.
- **Docs pages** — `app/docs/kafka-connect/page.tsx` et al. still DCG-only, but those pages are already hidden from the AISG sidebar via `AISG_HIDDEN` in `components/Sidebar.tsx:15-23`.
- **Marketing/legal text** — different repo and different RFC lineage.

## Verification

- `npm run build` clean.
- Route tree now lists `/usage` as a dynamic route.
- No backend code touched.
- Manual walkthrough: an AISG user on `/` sees the AISG hero; on `/account` sees LLMGate SDK copy and no Kafka Connect section; on `/contracts` Generate tab sees "sent to AISchemaGate"; the Quarantine tab has no lock. A DCG user sees everything exactly as before.
