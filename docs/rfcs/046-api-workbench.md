# RFC-046 — API Workbench

**Status:** Draft  
**Date:** 2026-05-18  
**Branch:** `nightly-maintenance-2026-05-18-rfc046-api-workbench`  
**Plan tier:** Growth+

---

## Problem

Authoring contracts for non-trivial APIs is high-friction:

1. Engineers must read docs, reverse-engineer request/response shapes, decide which fields matter, write YAML by hand, then wire up enforcement.
2. For large or evolving APIs (internal services, partner integrations, third-party data sources) this becomes a bottleneck that kills adoption.
3. Teams routinely skip contract coverage entirely because the authoring cost is too high.

The result: the gateway is underutilized at exactly the boundaries where it provides the most value.

---

## Goals

1. Let users seed a session from a URL, OpenAPI/Swagger spec, or a curl/Postman/Bruno collection.
2. Browse and test discovered endpoints interactively — entirely in the browser.
3. Infer a starting contract schema from real observed responses (client-side only).
4. Score inferred fields by confidence; surface low-confidence fields for human review.
5. Refine the contract live: required flags, patterns, enums, temporal types, PII tags, business rules.
6. Build multi-endpoint "contract suites" in one session.
7. Export as ContractGate YAML and/or ODCS-compatible YAML.
8. One-click deploy via the existing `POST /contracts/deploy` endpoint.
9. Detect contract drift: re-probe an endpoint and diff against the saved contract.
10. Provide a Postman/Bruno collection export as an enterprise escape hatch for teams that cannot use browser fetch (VPN-gated internal APIs).

---

## Non-Goals

- Server-side API proxying. ContractGate servers will never touch user credentials or response payloads. All API calls originate from the user's browser.
- Auth credential storage in ContractGate's database. Credentials live in `sessionStorage` only and are cleared on tab close.
- Automatic enforcement wiring. Users initiate deployment explicitly.
- Mocking or traffic replay beyond what the browser fetches live.

---

## Security Model

**Browser-local execution is the only execution model.**

| Concern | Mitigation |
|---|---|
| Credential theft | API keys/tokens stored in `sessionStorage` only; never sent to ContractGate servers; cleared on tab close |
| PII in response bodies | Responses inspected client-side; only the derived contract schema (no raw payload) is uploaded |
| SSRF | Impossible — requests originate from the user's browser, not ContractGate infrastructure |
| CORS-blocked endpoints | Documented limitation; Postman/Bruno export provided as the escape hatch |

For teams whose APIs are behind a VPN or corporate proxy (where browser fetch is blocked by CORS or network policy), the Postman/Bruno export path is the recommended alternative. ContractGate remains out of the data path in both cases.

---

## User Experience

### 1. Seed Input

The Workbench accepts any of:

- **Base URL** — e.g. `https://api.example.com/v2`. ContractGate attempts to discover `openapi.json` / `swagger.json` at common paths.
- **OpenAPI/Swagger spec URL** — fetched directly from the browser; parsed client-side.
- **Uploaded spec file** — `.json` or `.yaml` dropped onto the page.
- **curl command** — pasted; parsed into method + URL + headers.
- **Postman Collection JSON** — imported; endpoints listed for exploration.
- **Bruno Collection** — same.

### 2. Endpoint Browser

Discovered endpoints are listed with method badges (GET, POST, etc.). Users click an endpoint to open the request panel:

- Path parameter fields auto-populated from the spec or editable.
- Query params and request body editable as JSON.
- Auth panel: Bearer token, API key (header or query), or Basic — stored in `sessionStorage`.

**Send** fires a `fetch()` from the browser. Response is displayed with syntax highlighting.

### 3. Schema Inference

After a response is received, ContractGate infers a schema from the JSON body:

- Each field gets a **confidence score** (0–100) based on:
  - Present in spec (`+40`)
  - Non-null in observed response (`+30`)
  - Consistent type across multiple samples (`+20`)
  - Matches a known semantic pattern — UUID, ISO 8601, email, currency (`+10`)
- Fields with confidence < 60 are flagged amber; < 30 flagged red.
- Users promote, demote, or drop fields before saving.

### 4. Live Contract Refinement

For each field users can set:

- `required: true/false`
- `type` override (string / integer / number / boolean / array / object)
- `pattern` (regex)
- `enum` values (populated from observed samples, editable)
- `min` / `max` for numeric fields
- Temporal type: `date` / `datetime` / `timestamp` (leverages RFC-044 date type)
- PII tag: `pii: true` + masking strategy (links to PII masking config)
- Business rule annotation (free text, stored in `glossary`)

### 5. Multi-Endpoint Suite

Users build a **suite** — a named collection of endpoint contracts that logically belong together (e.g. "Stripe Webhooks", "Partner Feed v3"). The suite is saved as a group of ContractGate contracts sharing a common name prefix.

### 6. Export & Deploy

Two export targets:

| Target | Format | Use |
|---|---|---|
| ContractGate YAML | `docs/contracts/<name>.yaml` | Direct enforcement via ingest pipeline |
| ODCS-compatible YAML | `docs/contracts/<name>.odcs.yaml` | Interop with external contract registries |

**ODCS Export:** targets the latest published ODCS schema version by default. A version selector dropdown is shown in the export dialog; if the target ODCS version is known at build time it is pre-selected, otherwise "Latest" is the default. The `contractgate infer` CLI exposes this as `--odcs-version <version>` (default: `latest`).

**Deploy** calls `POST /contracts/deploy` for each contract in the suite. Requires Growth plan (enforced via `<PlanGate minTier="growth">`).

### 7. Drift Detection

Users can re-run a saved Workbench session against a live endpoint. ContractGate diffs the newly observed schema against the deployed contract and shows:

- **Fields added** (green) — candidates to add to the contract
- **Fields removed** (red) — may indicate breaking change
- **Type changes** (amber) — always a breaking change candidate
- **Enum value changes** (amber) — new values observed outside the contract's enum list

Drift report links to the existing contract version history.

### 8. Postman / Bruno Export (Enterprise Escape Hatch)

For teams whose endpoints are not reachable from a browser (VPN, mTLS, corporate proxy), the Workbench exports:

- **Postman Collection v2.1** JSON — pre-configured requests matching the explored session.
- **Bruno Collection** — same, in Bruno's native format.

Both include a post-response test script that formats the response JSON for piping into `contractgate infer` (see Newman CLI Pipe below). ContractGate never enters the data path.

### 9. Newman CLI Pipe

For teams running Postman collections in CI or locally via Newman, ContractGate supports a pipe-friendly output mode:

```bash
newman run collection.json --reporters cli,json --reporter-json-export response.json \
  | contractgate infer --from-newman response.json --out contracts/my-api.yaml
```

The `contractgate infer` subcommand (added to the existing CLI):

- Reads Newman's JSON reporter output.
- Applies the same client-side inference logic (field types, confidence scoring, semantic patterns).
- Writes a ContractGate YAML contract to `--out`.
- Optionally writes an ODCS-compatible YAML with `--odcs`.
- Accepts `--odcs-version <version>` to pin the ODCS schema version (defaults to latest; see ODCS Export below).

This path keeps ContractGate entirely out of the network path while supporting VPN-gated or mTLS-protected internal APIs.

---

## Plan Gating

| Feature | Tier |
|---|---|
| API Workbench (full) | Growth+ |
| Workbench read-only preview (no deploy) | Free (teaser) |

Free users see the Workbench tab with a `<PlanGate minTier="growth">` upsell card, plus a **Try It** teaser mode:

- Single endpoint only (no suite builder).
- Full inference and refinement UI enabled — users can see the value.
- **Save**, **Deploy**, and **Export** buttons are disabled and show an upsell tooltip: "Save and deploy contracts — Growth plan required."
- Teaser session state is not persisted across page loads.

This surfaces the core value proposition to Free users without giving away the full feature.

---

## Dashboard Changes

- New **Workbench** tab in the main nav (between "Scaffold" and "Collaborate").
- Session state persisted in `localStorage` (endpoint list, auth config without credentials, inferred schemas).
- Credentials stored in `sessionStorage` only.

---

## Pricing Page Update

Add row to the feature table (RFC-045 introduced the table):

| Feature | Free | Growth | Enterprise |
|---|---|---|---|
| API Workbench (Try It — 1 endpoint) | ✓ | ✓ | ✓ |
| API Workbench (full — save/deploy/suite) | ✗ | ✓ | ✓ |

---

## Rollout

1. `WorkbenchPage` component — seed input + endpoint browser (no inference yet)
2. Client-side schema inference + confidence scoring
3. Live contract refinement UI
4. Free-tier Try It teaser mode (single endpoint, Save/Deploy disabled)
5. Multi-endpoint suite builder (Growth+)
6. ContractGate YAML export + `POST /contracts/deploy` integration
7. ODCS export with version selector dropdown
8. Drift detection
9. Postman / Bruno collection export
10. `contractgate infer --from-newman` CLI subcommand
11. `<PlanGate>` wiring + pricing page update
12. Docs: `docs/api-workbench-reference.md`

---

## Resolved Questions

1. **Free-tier teaser** — included in this RFC. Single-endpoint Try It mode, Save/Deploy disabled.
2. **Newman CLI pipe** — included in this RFC as `contractgate infer --from-newman`.
3. **ODCS version** — latest by default; version selector in export dialog and `--odcs-version` CLI flag for pinning.
