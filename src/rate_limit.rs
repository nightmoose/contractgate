//! Per-API-key token-bucket rate limiter (RFC-021).
//!
//! State lives in a `DashMap<Uuid, Mutex<Bucket>>` stored on `AppState`.
//! Each bucket is keyed by `api_key_id` so limits are per-issued-key, not
//! per-IP.  Lock contention is negligible: the critical section is a handful
//! of float operations and an `Instant::elapsed()` call.
//!
//! ### Defaults (applied when `api_keys.rate_limit_*` columns are NULL)
//! - Sustained rate: **100 req/sec**
//! - Burst capacity: **200 tokens** (RFC-098: 2× rps, was 10×)
//!
//! ### Algorithm
//! Classic "leaky bucket as meter" (token bucket):
//!   tokens += elapsed_secs * rate   (capped at burst)
//!   if tokens >= 1.0 { tokens -= 1.0; allow } else { deny }
//!
//! The first request for a new key initialises the bucket at full capacity
//! so there is no cold-start penalty.

use dashmap::DashMap;
use std::{sync::Mutex, time::Instant};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Defaults
// ---------------------------------------------------------------------------

pub const DEFAULT_RATE_LIMIT_RPS: u32 = 100;
pub const DEFAULT_RATE_LIMIT_BURST: u32 = 200;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Mutable per-key state.  Wrapped in `Mutex` (not `RwLock`) because every
/// access is a write (refill + consume).
struct Bucket {
    tokens: f64,
    last_refill: Instant,
    /// Effective rate (req/sec) — copied from `ValidatedKey` at first access
    /// so overrides are reflected after a cache miss.
    rate: f64,
    /// Burst capacity (max tokens).
    burst: f64,
}

impl Bucket {
    fn new(rate: f64, burst: f64) -> Self {
        Bucket {
            tokens: burst, // start full — no cold-start penalty
            last_refill: Instant::now(),
            rate,
            burst,
        }
    }

    /// Refill tokens proportional to elapsed time, then attempt to consume
    /// one token.  Returns `(allowed, remaining_tokens_floor,
    /// reset_millis_from_now)`.
    fn check(&mut self) -> (bool, u32, u64) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_refill).as_secs_f64();
        self.last_refill = now;

        // Refill
        self.tokens = (self.tokens + elapsed * self.rate).min(self.burst);

        let allowed = self.tokens >= 1.0;
        if allowed {
            self.tokens -= 1.0;
        }

        // How many ms until the bucket has at least 1 token again.
        let reset_ms = if allowed || self.tokens >= 1.0 {
            0
        } else {
            let deficit = 1.0 - self.tokens;
            ((deficit / self.rate) * 1_000.0).ceil() as u64
        };

        (allowed, self.tokens.floor() as u32, reset_ms)
    }
}

// ---------------------------------------------------------------------------
// Shared state
// ---------------------------------------------------------------------------

/// Shared rate-limit state.  Add as a field on `AppState`.
pub struct RateLimitState {
    buckets: DashMap<Uuid, Mutex<Bucket>>,
}

impl Default for RateLimitState {
    fn default() -> Self {
        RateLimitState {
            buckets: DashMap::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Outcome of a rate-limit check.
#[derive(Debug, Clone)]
pub struct RateLimitOutcome {
    pub allowed: bool,
    /// Effective rps limit for this key.
    pub limit: u32,
    /// Tokens remaining after this request (floored).
    pub remaining: u32,
    /// Unix timestamp (seconds) when the bucket will have ≥1 token.
    /// Equals `now + reset_ms/1000`.  Always set; 0 when allowed.
    pub reset_unix: u64,
    /// Milliseconds until at least one token is available (0 when allowed).
    pub retry_after_ms: u64,
}

impl RateLimitOutcome {
    /// RFC-103: canonical header set for every 429 (and informational on 2xx).
    /// `Retry-After` is in **seconds** (RFC 6585), ceiling of `retry_after_ms`.
    pub fn headers(&self) -> [(&'static str, String); 4] {
        let retry_after_s = self.retry_after_ms.div_ceil(1_000);
        [
            ("X-RateLimit-Limit", self.limit.to_string()),
            ("X-RateLimit-Remaining", self.remaining.to_string()),
            ("X-RateLimit-Reset", self.reset_unix.to_string()),
            ("Retry-After", retry_after_s.to_string()),
        ]
    }
}

impl RateLimitState {
    /// Check and consume one token for `key_id`.
    ///
    /// `rps_override` / `burst_override` come from `ValidatedKey` columns;
    /// pass `None` to use defaults.
    pub fn check(
        &self,
        key_id: Uuid,
        rps_override: Option<u32>,
        burst_override: Option<u32>,
    ) -> RateLimitOutcome {
        let rate = rps_override.unwrap_or(DEFAULT_RATE_LIMIT_RPS) as f64;
        let burst = burst_override.unwrap_or(DEFAULT_RATE_LIMIT_BURST) as f64;

        // Get-or-insert the bucket.  DashMap::entry is lock-free for misses
        // when the key doesn't exist yet.
        let entry = self
            .buckets
            .entry(key_id)
            .or_insert_with(|| Mutex::new(Bucket::new(rate, burst)));

        // RFC-054: recover from poison instead of crashing the process.
        let mut bucket = entry.lock().unwrap_or_else(|e| {
            tracing::error!("rate-limit bucket Mutex was poisoned — recovering inner value");
            e.into_inner()
        });

        // Update effective rate/burst if the key's override changed since the
        // bucket was created (e.g. after a DB-backed key cache miss).
        bucket.rate = rate;
        bucket.burst = burst;

        let (allowed, remaining, reset_ms) = bucket.check();
        drop(bucket); // release lock before computing unix timestamp

        let reset_unix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() + reset_ms / 1_000 + if reset_ms % 1_000 > 0 { 1 } else { 0 })
            .unwrap_or(0);

        // RFC-098: report the per-second allowance as remaining, not the raw
        // bucket. A full bucket for Free is 20 tokens but limit is 10 r/s —
        // Remaining: 20 with Limit: 10 is a header lie under IETF-draft
        // semantics. The limiter still enforces on the real bucket.
        let limit_header = rps_override.unwrap_or(DEFAULT_RATE_LIMIT_RPS);
        RateLimitOutcome {
            allowed,
            limit: limit_header,
            remaining: remaining.min(limit_header),
            reset_unix,
            retry_after_ms: reset_ms,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_up_to_burst_then_denies() {
        let state = RateLimitState::default();
        let id = Uuid::new_v4();
        // Burst = 5, rate = 1 rps — bucket starts full.
        for i in 0..5 {
            let out = state.check(id, Some(1), Some(5));
            assert!(out.allowed, "request {} should be allowed", i);
        }
        let out = state.check(id, Some(1), Some(5));
        assert!(!out.allowed, "6th request should be denied");
        assert!(out.retry_after_ms > 0);
    }

    #[test]
    fn default_params_applied_when_none() {
        let state = RateLimitState::default();
        let id = Uuid::new_v4();
        let out = state.check(id, None, None);
        assert!(out.allowed);
        assert_eq!(out.limit, DEFAULT_RATE_LIMIT_RPS);
    }

    #[test]
    fn per_key_override_applied() {
        let state = RateLimitState::default();
        let id = Uuid::new_v4();
        let out = state.check(id, Some(200), Some(2_000));
        assert_eq!(out.limit, 200);
        // Burst=2000 means the bucket has 1999 tokens after one request, but
        // RFC-098 clamps the Remaining header to the per-second limit (200).
        assert_eq!(out.remaining, 200);
    }

    /// RFC-098: X-RateLimit-Remaining must never exceed X-RateLimit-Limit.
    /// Internally the bucket still tracks the full burst (so enforcement is
    /// unchanged), but the header reflects the per-second allowance.
    #[test]
    fn remaining_header_clamped_to_limit() {
        let state = RateLimitState::default();
        let id = Uuid::new_v4();
        // Free-plan shape: rps=10, burst=20. Fresh bucket has 20 tokens;
        // after one request, bucket has 19 — but header must report ≤ 10.
        let out = state.check(id, Some(10), Some(20));
        assert!(out.allowed);
        assert_eq!(out.limit, 10);
        assert!(
            out.remaining <= out.limit,
            "remaining {} must not exceed limit {}",
            out.remaining,
            out.limit
        );
    }

    #[test]
    fn rate_limit_headers_fields_present() {
        let state = RateLimitState::default();
        let id = Uuid::new_v4();
        let out = state.check(id, Some(10), Some(10));
        assert!(out.allowed);
        assert!(out.reset_unix > 0);
        assert_eq!(out.retry_after_ms, 0); // allowed → no wait
    }

    /// RFC-103: a denied outcome must produce all four rate-limit headers,
    /// including a non-zero `Retry-After` in seconds (ceil of retry_after_ms).
    #[test]
    fn denied_outcome_emits_four_headers_including_retry_after() {
        let state = RateLimitState::default();
        let id = Uuid::new_v4();
        // Burst=1, rps=1 → second call denied with ~1000ms retry.
        let _ = state.check(id, Some(1), Some(1));
        let denied = state.check(id, Some(1), Some(1));
        assert!(!denied.allowed);
        let headers = denied.headers();
        assert_eq!(headers.len(), 4);
        assert_eq!(headers[0].0, "X-RateLimit-Limit");
        assert_eq!(headers[1].0, "X-RateLimit-Remaining");
        assert_eq!(headers[2].0, "X-RateLimit-Reset");
        assert_eq!(headers[3].0, "Retry-After");
        let retry_after: u64 = headers[3].1.parse().unwrap();
        assert!(retry_after >= 1, "Retry-After must be ≥ 1s when denied");
    }

    /// RFC-043 fix-1: two distinct UUIDs must have independent buckets.
    /// Exhausting one must not affect the other — guards against the old
    /// nil-UUID bug where all JWT users shared a single global bucket.
    #[test]
    fn two_distinct_uuids_have_independent_buckets() {
        let state = RateLimitState::default();
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        // Drain bucket A entirely (burst = 1, rps = 1).
        let out_a = state.check(a, Some(1), Some(1));
        assert!(out_a.allowed, "first request on A must be allowed");
        let out_a2 = state.check(a, Some(1), Some(1));
        assert!(!out_a2.allowed, "A should be exhausted");
        // B must still be untouched.
        let out_b = state.check(b, Some(1), Some(1));
        assert!(out_b.allowed, "B must be unaffected by A being exhausted");
    }
}
