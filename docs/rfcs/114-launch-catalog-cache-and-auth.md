# RFC-114 — Public catalog cache, cheap bad keys, dashboard SSR bypass

**Status:** Accepted
**Date:** 2026-11-10

## Problem

Anonymous catalog reads (`/catalog`, `/public-contracts`, `/public-contract-families`, and `/public-contracts/{slug}` including `/yaml` and `/odcs`) query Postgres on every request and send no `Cache-Control`. A load against the hosted API showed those routes as the slow public surface.

Each invalid API key also does a database lookup and writes a warning. A client that replays one bad key, or rotates many, spends a pool connection per request.

The dashboard renders the catalog on Vercel. Those server-side calls share one egress address and the API's 30 requests/second per-IP limit, so a cold minute returns 429 to catalog pages. Vercel egress addresses are not fixed, so they cannot be allowlisted.

The Python local validator ignores `format: email`. The Rust engine rejects values that fail a small structural check (one `@`, no whitespace, a dotted domain). A Kinesis sample let 967 bad emails through the Python path.

## Decision

1. **Public read cache.** An in-process map, 60-second TTL, at most 512 entries, keyed by path and query string, stores anonymous `200` responses for those catalog routes (and the `/family` and family-detail GETs under the same prefixes). Successful anonymous responses send `Cache-Control: public, max-age=60, s-maxage=300, stale-while-revalidate=600`. A request with `Authorization` or `x-api-key` is not stored and is marked `private, no-store`. `/published/{ref}` stays uncached: that handler is link-token aware. The per-IP limiter still runs outside the cache, so a cache hit spends a token.

2. **Negative API-key cache.** The key cache already records a definitive rejection for 60 seconds under SHA-256 of the raw key. The auth middleware was deleting that entry on every 401, so the next request queried again. It no longer deletes it. A key created after it was rejected is invisible until that entry expires: 60 seconds at worst, then the next request reads the database. Transient database errors are still not cached. Valid keys are unchanged. The per-request rejection log is `debug`. One `warn` is emitted when an address crosses the failure limit.

3. **Failed-auth limit.** 30 rejected credentials per client IP per 60 seconds, then HTTP 429 with `Retry-After`. The counter runs only after the credential has been rejected. A key or session that validates is not counted and is not blocked.

4. **SSR bypass.** If `CONTRACTGATE_TRUSTED_SSR_TOKEN` is set on the API and the request header `X-DCG-SSR-Token` matches (constant-time), the public per-IP limiter is skipped. No other limiter sits on those routes. The dashboard sends the header from `dashboard/lib/catalog.ts` only when the same server env var is set. The name is not `NEXT_PUBLIC_`. Unset on either side: previous behavior. Catalog fetches use `next: { revalidate: 60 }`. The detail page stays `dynamic = "force-dynamic"`.

5. **Python `format`.** The local validator parses `format` and applies the Rust rules for `email`, `uuid`, and `uri`/`url` (case-insensitive name, original spelling in the message). Any other format name passes.

## Remaining Python gaps

These are field types, not `format` values. The Rust engine enforces them; the Python parser still rejects the type name:

- `type: date` — real `YYYY-MM-DD` calendar date
- `type: timestamp` — RFC 3339 date-time

## Non-goals

- Caching `/published/{ref}` or any authenticated route.
- Changing the 60-second positive cache (revocation is still visible within 60 seconds).
- Publishing a new Python package. Release tags require `pyproject.toml` to match `Cargo.toml`, so the SDK version stays 0.4.1.
