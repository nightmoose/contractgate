# RFC-035: CSV Contract Inference

**Status:** Draft
**Date:** 2026-05-15
**Author:** Alex Suarez

---

## Problem

`POST /contracts/infer` accepts JSON samples only. Customers whose data lives
in CSV files must manually translate column names and types into contract YAML,
which is error-prone and slow. CSV is the dominant format for open data sources
(relevant to RFC-034) and internal bulk exports.

RFC-006 extended inference to Avro, Protobuf, and OpenAPI but did not include
CSV.

---

## Goals

1. Add `POST /contracts/infer/csv` — accepts a raw CSV body (or base64),
   auto-detects delimiter, infers a draft contract YAML using the same type
   heuristics as `infer.rs`.
2. Reuse the existing `infer_fields_from_observations` logic — CSV parsing is
   just a different front-end feeding the same engine.
3. Match the existing `InferResponse` shape so callers need no SDK changes.

---

## Non-Goals

- Multi-file / zip upload (single file only).
- Excel / ODS format (separate RFC if needed).
- Header-less CSVs (headers required; unnamed columns are rejected with a clear
  error).
- Streaming inference on very large files (> `MAX_CSV_BYTES` is rejected).

---

## Decisions

| # | Question | Decision |
|---|---|---|
| Q1 | Delimiter detection | Auto-detect: sniff first 4 KB, score comma / semicolon / tab by consistency across rows. Caller may override via `?delimiter=` query param. |
| Q2 | Crate | `csv` (already in ecosystem, no new heavy dep). Add to `Cargo.toml`. |
| Q3 | Body encoding | Raw UTF-8 bytes preferred (`Content-Type: text/csv`). Base64 fallback via JSON body `{ "base64": "..." }` for clients that can't send raw binary. |
| Q4 | Row sampling | Up to 1000 rows used for inference (same spirit as JSON sampling). |
| Q5 | Route | `/contracts/infer/csv` — consistent with `/infer/avro`, `/infer/proto`, `/infer/openapi` from RFC-006. |
| Q6 | Empty / whitespace values | Treated as missing (contributes `required: false`). |
| Q7 | Type coercion order | integer → float → boolean → string. UUID/date/datetime pattern detection runs on string columns only. |

---

## New Route

```
POST /contracts/infer/csv?name=my_contract&delimiter=,
```

**Headers:**
```
Content-Type: text/csv
x-api-key: <key>
```

**Body:** raw CSV bytes (UTF-8). First row must be the header.

**Alternatively (JSON wrapper):**
```json
{
  "name": "my_contract",
  "description": "optional",
  "base64": "<base64-encoded CSV>"
}
```

**Response:** same `InferResponse` as `POST /contracts/infer`:
```json
{
  "yaml_content": "...",
  "field_count": 7,
  "sample_count": 423
}
```

---

## Implementation

New file: `src/infer_csv.rs`

### Delimiter sniffing

```rust
fn detect_delimiter(sample: &[u8]) -> u8 {
    let candidates = [b',', b';', b'\t'];
    // For each candidate, count consistent column counts across lines.
    // Highest consistency score wins. Tie-break: comma > tab > semicolon.
}
```

### Parsing + inference

```rust
pub async fn infer_csv_handler(/* ... */) -> AppResult<Json<InferResponse>> {
    // 1. Read body (raw bytes or base64 JSON wrapper).
    // 2. Detect delimiter (or use ?delimiter= override).
    // 3. Parse with `csv::ReaderBuilder` — first row = headers.
    // 4. Convert each row to serde_json::Value (object keyed by header).
    // 5. Call existing `infer_fields_from_objects(&rows)`.
    // 6. Serialize to YAML, return InferResponse.
}
```

Steps 5–6 are identical to the JSON inference handler — no duplication of
type-heuristic logic.

### Error cases

| Condition | HTTP | Message |
|-----------|------|---------|
| No header row (empty file) | 400 | "CSV body is empty" |
| Duplicate column name | 400 | "duplicate column: {name}" |
| Body exceeds `MAX_CSV_BYTES` | 413 | "CSV too large" |
| Invalid UTF-8 | 400 | "CSV must be valid UTF-8" |
| Cannot detect delimiter | 400 | "could not detect delimiter; pass ?delimiter= explicitly" |

---

## Integration with RFC-034

When ContractGate fetches a public data source with `source_format = "csv"`,
the fork fetch handler (`POST /contracts/:id/fetch`) will use the same CSV
parser from `infer_csv.rs` to deserialize upstream rows into
`serde_json::Value` objects before applying the fork filter. No separate
implementation needed.

---

## Migration

No DB migration required. Route addition only.

---

## Cargo.toml

```toml
csv = "1.3"
```

(Check if already present before adding.)

---

## Tests

- Unit: delimiter detection on comma / semicolon / tab / mixed files.
- Unit: type inference on integer, float, boolean, UUID, ISO date, enum
  columns.
- Unit: missing values → `required: false`.
- Integration: `POST /contracts/infer/csv` with a real CSV fixture → valid
  YAML that passes `cargo test`.
- Edge: duplicate header → 400. Empty body → 400. Oversized body → 413.

---

## Acceptance Criteria

- [ ] `POST /contracts/infer/csv` returns valid contract YAML for a
  well-formed CSV.
- [ ] Auto-detects comma, semicolon, and tab delimiters correctly.
- [ ] `?delimiter=` override respected.
- [ ] All error cases above return correct HTTP status + message.
- [ ] Output YAML passes existing contract validation (no schema regression).
- [ ] All new unit + integration tests pass (`cargo test`).
- [ ] `csv` crate added to `Cargo.toml`; `cargo check` clean.
