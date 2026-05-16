# RFC-037: API Endpoint as Contract Source

**Status:** Draft  
**Date:** 2026-05-16  
**Author:** Alex Suarez  

---

## Problem

Users who want to contract-gate an existing API today must manually transcribe
the API's schema into YAML. This is error-prone and slows adoption. With CSV
inference (RFC-035) already shipping, the natural next step is live-endpoint
inference: paste a URL, let ContractGate fetch a sample, and auto-draft the
contract.

This also closes the gap in the RFC-036 wizard — the "Connect an API" path
is the fourth source option users ask for most.

---

## Goals

1. Add `POST /contracts/infer/url` — fetches the URL, detects JSON or CSV,
   runs the existing inference engine, returns a draft YAML contract.
2. Add a **"Connect an API"** tile to the `NewContractWizard` (RFC-036) that
   drives this route.
3. Support optional custom request headers (for APIs that require an auth token
   or `Accept` override).
4. Keep the validation engine budget unaffected — upstream fetch time is
   excluded from the <15 ms p99 measurement, exactly as in RFC-034.

---

## Non-Goals

- Storing the source URL on the contract row (no migration; URL goes in the
  contract `description` as a note — user can edit).
- Polling / scheduled re-inference when the upstream API changes.
- OAuth / multi-step auth flows (only static headers in v1).
- Pagination — only the first page / response body is sampled.
- Binary response formats (only JSON and CSV in v1).

---

## Decisions

| # | Question | Decision |
|---|---|---|
| D1 | Auth model | Optional `headers` map (`{key: value}`) in request body. Passed verbatim to upstream. No secrets stored server-side. |
| D2 | Max response size | 10 MB (same as CSV). Returns 413 if exceeded. |
| D3 | Timeout | 10 s (`INFER_URL_TIMEOUT_MS` env var, default 10000). |
| D4 | JSON shape handling | Array of objects → infer directly. Single object → wrap in array. Nested: probe common envelope keys (`data`, `items`, `results`, `records`, `rows`) for an inner array. Else error. |
| D5 | CSV detection | If `Content-Type` is `text/csv` or `text/plain`, or URL path ends `.csv`, treat as CSV (reuse `infer_csv` parser). Otherwise treat as JSON. |
| D6 | URL in contract | Embed as a comment in the generated YAML description field: `"Inferred from <url>"`. User can edit. |
| D7 | Auth requirement | API key required (same as other write routes). The infer route doesn't persist anything, but it makes outbound HTTP requests on behalf of the caller — auth prevents abuse. |
| D8 | Max sampled rows | 1 000 rows (same as CSV). |

---

## New Route

```
POST /contracts/infer/url
x-api-key: <key>
Content-Type: application/json

{
  "name": "my_api_contract",
  "url": "https://api.example.com/v1/events",
  "headers": {                        // optional
    "Authorization": "Bearer tok_xyz",
    "Accept": "application/json"
  }
}
```

### Response (success)

```json
{
  "yaml_content": "version: \"1.0\"\nname: my_api_contract\n...",
  "field_count": 9,
  "sample_count": 47,
  "detected_format": "json"
}
```

`detected_format` is `"json"` or `"csv"` — useful for the UI to confirm what
was sniffed.

### Errors

| Status | Condition |
|--------|-----------|
| 400 | Missing / invalid URL, empty response, unrecognised shape |
| 413 | Response body exceeds 10 MB |
| 422 | Could not infer any fields (response has no recognisable rows) |
| 504 | Upstream timeout |

---

## Implementation

New module: `src/infer_url.rs`

```rust
pub struct InferUrlRequest {
    pub name: String,
    pub url: String,
    pub headers: Option<HashMap<String, String>>,
}

pub struct InferUrlResponse {
    pub yaml_content: String,
    pub field_count: usize,
    pub sample_count: usize,
    pub detected_format: String,   // "json" | "csv"
}
```

Logic:
1. Validate URL (must be `http://` or `https://`).
2. Build `reqwest::Client` with timeout. Forward caller-supplied headers.
3. Fetch. Check response size ≤ 10 MB.
4. Detect format from `Content-Type` / URL suffix.
5. Parse: JSON path → `serde_json`, CSV path → reuse `infer_csv::parse_csv`.
6. For JSON: unwrap envelope if needed → call `infer_fields_from_objects_pub`.
7. Serialise contract to YAML with description `"Inferred from <url>"`.
8. Return `InferUrlResponse`.

Wired in `main.rs`:
```
.route("/contracts/infer/url", post(infer_url::infer_url_handler))
```

---

## Frontend Changes

`NewContractWizard.tsx` — add fourth tile and `ApiStep`:

**Tile:**
```
🔌 Connect an API
Paste an endpoint URL. ContractGate fetches a
sample and infers the contract schema.
```

**`ApiStep`:**
- URL text input
- Expandable "Add headers" section (key/value pairs, add/remove rows)
- "Fetch & Infer" button → calls `inferUrl()`
- Editable YAML textarea (same pattern as CSV step)
- "Save Contract" → `createContract(yaml)`

New API client function in `lib/api.ts`:
```ts
export const inferUrl = (params: {
  name: string;
  url: string;
  headers?: Record<string, string>;
}) => apiFetch<InferUrlResponse>("/contracts/infer/url", {
  method: "POST",
  body: JSON.stringify(params),
});
```

---

## Cargo Dependencies

`reqwest` with `json` and `stream` features is already in `Cargo.toml`
(used by `public_catalog.rs`). No new dependencies needed.

---

## Acceptance Criteria

- [ ] `POST /contracts/infer/url` returns inferred YAML for a public JSON API.
- [ ] `POST /contracts/infer/url` returns inferred YAML for a CSV endpoint.
- [ ] Custom headers are forwarded to the upstream request.
- [ ] 10 MB limit enforced — returns 413 above threshold.
- [ ] 10 s timeout — returns 504 on slow upstream.
- [ ] Envelope unwrapping works for `data`, `items`, `results`, `records`, `rows`.
- [ ] Wizard "Connect an API" tile navigates to `ApiStep`.
- [ ] Full infer → edit → save flow works end-to-end in the wizard.
- [ ] `cargo check` and `tsc --noEmit` pass clean.
- [ ] Unit tests: URL validation, format detection, envelope unwrapping.

---

## Resolved Questions

- **OQ1:** SSRF protection — shipped in the same PR. Hostname is resolved via
  DNS before the HTTP request; all resolved addresses are checked against blocked
  ranges: loopback (127/8, ::1), RFC 1918 private (10/8, 172.16/12, 192.168/16),
  link-local/APIPA (169.254/16 — covers AWS metadata endpoint), IPv6 link-local
  (fe80::/10), unique-local (fc00::/7), multicast, and IPv4-mapped equivalents.
  Bare IP literals are also rejected without a DNS round-trip. **Closed: shipped.**
