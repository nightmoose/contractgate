# RFC-115 — Anonymous playground

**Status:** Accepted
**Date:** 2026-11-14

## Problem

`/playground` on the dashboard is a logged-out preview. The contract YAML and the sample event are not editable, and the "Try the live demo first" link on that preview points at `/playground`. The real check, `POST /playground/validate`, requires an API key or a session (RFC-042) because an open, unlimited compiler is an abuse path. A first visit therefore cannot run a contract without creating an account.

There is no WASM build of the engine to run the check in the browser.

## Decision

Add `POST /public/playground/validate` on the public router.

- The body is `{ "yaml_content": string, "event": any JSON }`. There is no contract id. The handler does not take application state and does not read or write Postgres.
- The response is the existing playground shape plus `disposition`: `pass` when validation succeeds, otherwise the contract's `on_fail` (`quarantine` by default, or `drop`).
- The body limit is 64 KiB. The per-IP token bucket is 1 request/second with a burst of 8. The bucket key is a different UUID from the catalog limiter, so the two ceilings do not share tokens. `Fly-Client-IP` wins over `X-Forwarded-For`, same as the catalog limiter.
- A matching `X-DCG-SSR-Token` does not skip this limiter. The dashboard calls the route from the browser.
- `POST /playground/validate` stays authenticated and keeps the 1 MB protected body limit. Both routes call the same compile-and-validate function.

The dashboard renders the real editor when there is no session, calls the public route, and shows Sign up after a result. Preview pages other than `/playground` still link to it. The link is omitted on `/playground`.

## Out of scope

Pricing, signup, and billing are unchanged. The authenticated playground is unchanged apart from the additive `disposition` field.
