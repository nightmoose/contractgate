# URL Contract Inference — API Reference

**RFC:** 037 (SSRF hardening: RFC-049)  
**Status:** Accepted  
**Added:** 2026-05-24  
**Plan:** Growth+ (see [plan-gating-reference.md](plan-gating-reference.md))

---

## Overview

`POST /contracts/infer/url` fetches a caller-supplied URL, auto-detects whether
the response is JSON or CSV, and runs the same inference engine as the JSON and
CSV inference endpoints. The result is a draft YAML contract.

**Security:** the endpoint implements SSRF protection (RFC-049) and does **not**
follow HTTP redirects. See the SSRF and redirect sections below.

---

## Endpoint

```
POST /contracts/infer/url
```

**Auth:** not required.

### Request body (JSON)

| Field | Type | Required | Description |
|---|---|---|---|
| `name` | string | Yes | Name embedded in the generated contract. |
| `description` | string | No | Optional description. Defaults to `"Inferred from {url}"` if omitted. |
| `url` | string | Yes | HTTP or HTTPS URL to fetch. Must start with `http://` or `https://`. Max 2 048 characters. |
| `headers` | object | No | Optional map of headers forwarded to the upstream request (e.g. `Authorization`, `X-Api-Key`). |

### Response `200 OK`

```json
{
  "yaml_content": "version: \"1.0\"\nname: \"census_data\"\n...",
  "field_count": 12,
  "sample_count": 500,
  "detected_format": "json"
}
```

| Field | Type | Description |
|---|---|---|
| `yaml_content` | string | Complete draft YAML contract. |
| `field_count` | integer | Number of fields inferred. |
| `sample_count` | integer | Number of rows sampled (≤ 1 000). |
| `detected_format` | string | `"json"` or `"csv"`. |

### HTTP status codes

| Status | Meaning |
|---|---|
| `200 OK` | Inference succeeded. |
| `400 Bad Request` | Invalid URL, SSRF-blocked address, non-2xx upstream response (including 3xx redirects), empty upstream body, response > 10 MB, or bad JSON/CSV. |
| `422 Unprocessable Entity` | Upstream responded successfully but no fields could be inferred (empty array, scalar body, etc.). |
| `504 Gateway Timeout` | Upstream did not respond within the configured timeout. |

---

## Format detection

| Signal | Detected format |
|---|---|
| `Content-Type: text/csv` | CSV |
| `Content-Type: text/plain` | CSV |
| URL path ends with `.csv` | CSV |
| Everything else | JSON |

When format is detected as CSV, the body is processed by the same engine as
`POST /contracts/infer/csv` (including delimiter auto-detection).

---

## JSON shape handling

The endpoint handles several common upstream JSON shapes automatically:

| Shape | Handling |
|---|---|
| `[{…}, {…}, …]` — array of objects | Inferred directly |
| `{…}` — single object | Wrapped in `[obj]`, inferred as a 1-row sample |
| `{"data": […]}`, `"items"`, `"results"`, `"records"`, `"rows"` | Inner array unwrapped |
| `[["col1","col2"],[val,val],…]` — json_rows (Census API) | Header row detected; data rows converted to objects |

---

## SSRF protection

Before making any HTTP request the hostname is resolved via DNS. All resolved
IP addresses are checked against blocked ranges. If **any** address falls in a
blocked range the request is rejected with `400 Bad Request`.

Blocked ranges:

| Range | Description |
|---|---|
| `0.0.0.0/8` | Unspecified |
| `127.0.0.0/8` | IPv4 loopback |
| `::1` | IPv6 loopback |
| `10.0.0.0/8` | RFC 1918 private |
| `172.16.0.0/12` | RFC 1918 private (172.16–172.31) |
| `192.168.0.0/16` | RFC 1918 private |
| `169.254.0.0/16` | Link-local / APIPA — **includes AWS metadata endpoint 169.254.169.254** |
| `fe80::/10` | IPv6 link-local |
| `fc00::/7` | IPv6 unique local (fc00:: and fd00::) |
| `ff00::/8` | IPv6 multicast |
| `224.0.0.0/4` | IPv4 multicast |
| `240.0.0.0/4` | IPv4 reserved/broadcast |
| `::ffff:0:0/96` | IPv4-mapped — inherits IPv4 block rules |

To close the DNS rebinding window, the first allowed `SocketAddr` returned by
the resolver is **pinned** into the HTTP client via `ClientBuilder::resolve()`.
The actual TCP connection uses this pre-checked IP directly and cannot be
redirected by a subsequent DNS lookup.

---

## Redirects are not followed

As of RFC-049, the HTTP client is built with `Policy::none()` — **redirects are
not followed**. A `301`, `302`, or any other 3xx response from the upstream is
treated as a non-2xx status and the endpoint returns `400 Bad Request`.

**Rationale:** the SSRF pre-flight check covers only the initial host. A
redirect to a new hostname (e.g. `302 → http://169.254.169.254/`) would result
in a fresh, unchecked DNS resolution and TCP connection, bypassing the IP block.
Inference sources are stable data endpoints and do not need redirects; supply
the canonical final URL directly.

---

## Configuration

| Variable | Default | Description |
|---|---|---|
| `INFER_URL_TIMEOUT_MS` | `10000` | Upstream request timeout in milliseconds. |

---

## Limits

| Limit | Value |
|---|---|
| Max upstream response body | 10 MB |
| Max sampled rows (JSON or CSV) | 1 000 |
| URL max length | 2 048 characters |
| Allowed schemes | `http://` and `https://` only |

---

## Examples

### Infer from a JSON API

```bash
curl -X POST https://your-instance/contracts/infer/url \
  -H "Content-Type: application/json" \
  -d '{
    "name": "github_repos",
    "url": "https://api.github.com/users/octocat/repos",
    "headers": {"Accept": "application/vnd.github.v3+json"}
  }'
```

### Infer from a CSV download

```bash
curl -X POST https://your-instance/contracts/infer/url \
  -H "Content-Type: application/json" \
  -d '{
    "name": "census_population",
    "url": "https://example.com/data/population.csv"
  }'
```

### Infer from a Census API (json_rows format)

```bash
curl -X POST https://your-instance/contracts/infer/url \
  -H "Content-Type: application/json" \
  -d '{
    "name": "acs_population",
    "url": "https://api.census.gov/data/2021/acs/acs5?get=NAME,B01001_001E&for=state:*"
  }'
```

The Census API returns a json_rows array (`[["NAME","B01001_001E","state"],[val,val,val],…]`).
The header row is automatically detected and each data row becomes a JSON object.

---

## Edge cases

- **Private URL supplied.** Any URL that resolves to a blocked range returns
  `400 Bad Request` with a message identifying the blocked IP. The URL is not
  fetched.
- **Redirect to private range.** With redirects disabled, a 302 response
  (whether pointing to a private address or not) returns `400 Bad Request`
  immediately.
- **Upstream returns non-JSON, non-CSV.** If the Content-Type is neither
  `text/csv` nor `text/plain` and the URL does not end in `.csv`, the body is
  parsed as JSON. A parse failure returns `400 Bad Request`.
- **Empty upstream body.** Returns `400 Bad Request`.
- **Timeout.** Returns `504 Gateway Timeout`. Increase `INFER_URL_TIMEOUT_MS`
  for slow upstream sources.

---

## Related

- [csv-inference-reference.md](csv-inference-reference.md) — CSV inference from a local document.
- [RFC-037](rfcs/037-api-source-contract-creation.md) — original design.
- [RFC-049](rfcs/049-ssrf-redirect-hardening.md) — SSRF redirect hardening.
