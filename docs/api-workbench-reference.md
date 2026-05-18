# API Workbench Reference

**RFC:** 046  
**Plan tier:** Growth+ (Try It mode available on Free)

The API Workbench turns API exploration into contract creation in one workflow. Paste a base URL, OpenAPI spec, curl command, or Postman/Bruno collection, explore endpoints live in your browser, infer a contract schema from real responses, refine field rules, and deploy enforcement — all without ContractGate ever touching your credentials or response payloads.

---

## Security model

All API calls originate from your browser. ContractGate servers never see:

- API keys, Bearer tokens, or Basic credentials
- Response payloads (only the derived contract schema is uploaded on deploy)
- Internal service traffic

Credentials are stored in `sessionStorage` only and are cleared when the tab closes. Non-sensitive session state (endpoint list, inferred schemas, suite) is persisted in `localStorage`.

For endpoints that cannot be reached from a browser (VPN, mTLS, corporate proxy), use the [Postman / Bruno export](#postman--bruno-export) or the [Newman CLI pipe](#newman-cli-pipe-contractgate-infer).

---

## Plan gating

| Feature | Free | Growth | Enterprise |
|---|---|---|---|
| Try It (1 endpoint, inference visible) | ✓ | ✓ | ✓ |
| Save to suite | ✗ | ✓ | ✓ |
| Deploy to ContractGate | ✗ | ✓ | ✓ |
| Export (YAML, ODCS, Postman, Bruno) | ✗ | ✓ | ✓ |
| Drift detection | ✗ | ✓ | ✓ |

---

## Seed input

The Workbench accepts any of the following seed formats:

| Mode | What to provide |
|---|---|
| **Base URL** | `https://api.example.com/v2` — ContractGate attempts to auto-discover `openapi.json` or `swagger.json` at common paths |
| **OpenAPI URL** | Direct URL to an OpenAPI 3.x or Swagger 2.x spec (JSON or YAML) |
| **Upload** | Drop a `.json` or `.yaml` spec file onto the page |
| **curl** | Paste a curl command: `curl -X GET "https://..." -H "Authorization: Bearer TOKEN"` |
| **Postman** | Paste a Postman Collection v2.1 JSON |

CORS note: the spec fetch and all endpoint requests run from your browser. If the target server does not send `Access-Control-Allow-Origin` headers, the browser will block the request. Use the [Postman / Bruno export](#postman--bruno-export) path instead.

---

## Endpoint browser

After seeding, discovered endpoints appear in the left sidebar with method badges (GET, POST, PUT, PATCH, DELETE). Click an endpoint to open the request panel.

**Request panel:**

- **Path parameters** — auto-populated from the spec; editable inline.
- **Query parameters** — toggle enabled/disabled per param.
- **Auth** — select None, Bearer, API Key, or Basic. Credentials stored in `sessionStorage` only.
- **Request body** — editable JSON textarea for POST/PUT/PATCH methods.

Click **Send** to fire the request from your browser.

---

## Schema inference

After a successful response, ContractGate infers a field schema from the JSON body:

| Observation | Output |
|---|---|
| UUID-shaped string | `pattern: ^[0-9a-f]{8}-...` |
| ISO 8601 date string | `temporal_type: date` |
| ISO 8601 datetime string | `temporal_type: datetime` |
| Numeric Unix timestamp | `temporal_type: timestamp` |
| Email-shaped string | `pattern: ^[^\s@]+@[^\s@]+\.[^\s@]+$` |
| ≤8 distinct string values | `enum: [...]` |
| Field null in any sample | `required: false` |
| Integer values | `type: integer` |
| Decimal values | `type: number` |

Each field receives a **confidence score** (0–100):

- 🟢 ≥70 — high confidence
- 🟡 40–69 — review recommended
- 🔴 <40 — low confidence, likely needs manual override

Send the same endpoint multiple times (with different params) to improve confidence — the Workbench merges samples on each send.

---

## Field refinement

Click any field row to expand the refinement panel:

| Setting | Effect |
|---|---|
| Type | Override inferred type (string / integer / number / boolean / array / object / any) |
| Temporal type | Mark as `date`, `datetime`, or `timestamp` (RFC-044) |
| Pattern | Regex the value must match |
| Enum values | Comma-separated allowed values |
| Min / Max | Numeric bounds (inclusive) |
| Required | Toggle required/optional |
| PII | Flag as PII — added to `glossary` with `pii: true` constraint |
| Annotation | Free-text business rule stored in the `glossary` |

---

## Multi-endpoint suite (Growth+)

Click **+ Add to suite** after refining an endpoint's schema to include it in the current suite. A suite is a named collection of endpoint contracts that are exported and deployed together.

Set the suite name in the suite panel at the bottom of the page.

**Deploy suite** calls `POST /contracts/deploy` for each contract in the suite.

---

## Export & deploy (Growth+)

### ContractGate YAML

The generated YAML follows the [locked contract format](../CLAUDE.md):

```yaml
version: "1.0"
name: "user_events"
description: "..."

ontology:
  entities:
    - name: user_id
      type: string
      required: true
      pattern: "^[0-9a-f]{8}-..."
    - name: amount
      type: number
      required: false
      min: 0

glossary:
  - field: email
    description: "PII field — handle with care"
    constraints: "pii: true"
```

Download as `.yaml` or click **Deploy to ContractGate** to call `POST /contracts/deploy` directly.

### ODCS-compatible YAML

The ODCS export targets the [Open Data Contract Standard](https://github.com/bitol-io/open-data-contract-standard). Select the target version from the dropdown:

- **Latest (2.2.2)** — default
- 2.1.0
- 2.0.0

Download as `.odcs.yaml`.

---

## Postman / Bruno export (offline / VPN)

For endpoints behind a VPN or corporate proxy where browser `fetch()` is blocked by CORS or network policy:

1. Click **↓ Postman Collection** to download a pre-configured `.postman_collection.json`.
2. Import into Postman, add your credentials to the collection variables (`BEARER_TOKEN`), and run.
3. Export responses using the Newman CLI and pipe to `contractgate infer` (see below).

A **Bruno Collection** (`.bru`) is also available for teams using Bruno as their API client.

---

## Newman CLI pipe (`contractgate infer`)

For CI or local workflows using Newman:

```bash
# 1. Run your collection and export results
newman run collection.json \
  --reporters json \
  --reporter-json-export newman-output.json

# 2. Infer a ContractGate contract from the results
contractgate infer \
  --from-newman newman-output.json \
  --name user_events \
  --out contracts/user_events.yaml

# 3. Optionally include an ODCS export
contractgate infer \
  --from-newman newman-output.json \
  --out contracts/user_events.yaml \
  --odcs \
  --odcs-version 2.2.2
```

### Flags

| Flag | Description |
|---|---|
| `--from-newman <FILE>` | Path to Newman JSON reporter export (required) |
| `--name <NAME>` | Contract name (defaults to collection name) |
| `--description <TEXT>` | Contract description |
| `--out <FILE>` | Output path for ContractGate YAML (defaults to stdout) |
| `--odcs` | Also write an ODCS-compatible YAML |
| `--odcs-version <VERSION>` | ODCS schema version: `2.2.2` (default), `2.1.0`, `2.0.0` |
| `--json` | Emit machine-readable JSON summary to stderr |

No API key is required. No network call is made. All processing is local.

---

## Drift detection (Growth+)

After an endpoint is saved to the suite, click **Check drift** to re-probe it against its saved contract. The drift report shows:

- 🟢 **Fields added** — new fields in the response not covered by the contract
- 🔴 **Fields removed** — fields the contract expects that are no longer in the response
- 🟡 **Type changes** — a field's observed type no longer matches the contract
- 🟡 **New enum values** — values observed outside the contract's allowed set

Drift results link to the contract's version history in the Contracts page.

---

## Session persistence

| Data | Storage | Cleared |
|---|---|---|
| Credentials (Bearer token, API key) | `sessionStorage` | On tab close |
| Endpoint list, inferred schemas, suite | `localStorage` | On "New session" or manual clear |

Click **↩ New session** in the header to reset the Workbench and clear `localStorage`.
