# RFC-049 — Close the SSRF redirect bypass in URL contract inference

**Status:** Draft  
**Date:** 2026-05-22  
**Branch:** nightly-maintenance-2026-05-22-rfc049  
**Addresses:** REVIEW-2026-05-22-launch-readiness B3  
**Severity:** P0 — launch blocker

---

## Problem

`POST /contracts/infer/url` (`src/infer_url.rs`) fetches a caller-supplied URL.
`check_ssrf` resolves the hostname, rejects private/reserved IP ranges, and
pins the first allowed address into the reqwest client via
`ClientBuilder::resolve()` — closing the DNS-rebinding window for the **initial
host only**.

The client is built with **no redirect policy** (`src/infer_url.rs:115-119`),
so reqwest follows up to 10 redirects by default. Each redirect to a *new*
host gets a fresh connection that is **not** pinned and **not** re-checked.
A public, attacker-controlled URL can return `302 Location:
http://169.254.169.254/latest/meta-data/iam/security-credentials/` and the
gateway will fetch the cloud metadata endpoint, returning instance IAM
credentials in the inference response or error.

The doc comment in `infer_url.rs` claims the rebinding window is closed; it is
only closed for hop 0.

---

## Fix

Pick **A** (simplest, sufficient):

- **A — disable redirects.** Build the client with
  `.redirect(reqwest::redirect::Policy::none())`. A `3xx` from the upstream
  becomes a clean `400` ("source URL returned a redirect; provide the final
  URL"). Inference sources are stable data endpoints — they should not need
  redirects.
- **B — custom redirect policy.** Implement `redirect::Policy::custom` that
  re-runs `check_ssrf` against every `Location` host before allowing the hop.
  More code; only needed if real sources turn out to require redirects.

Ship A. Revisit B only if a pilot reports a legitimate redirecting source.

Additionally:
- Apply the same `Policy::none()` to any other outbound `reqwest::Client` that
  fetches a user-influenced URL. Audit: `infer_openapi`, `infer_avro`,
  `infer_proto` if any accept a URL; the Kafka/Kinesis connectors use fixed
  broker endpoints and are out of scope.

---

## Testing

- Unit/integration: a stub upstream returning `302 -> http://169.254.169.254/`
  yields `400`, not a fetch of the metadata IP.
- Existing `is_blocked_v4/v6` tests stay green.
- A normal `200 application/json` source still infers correctly.

## What does NOT change

- The pre-flight DNS resolution + IP-range block stays — it is still the
  first line of defense for hop 0.

## Rollout

Application-only, no migration. Independent of other RFCs — ship standalone.
