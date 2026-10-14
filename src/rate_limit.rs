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
    /// retry_after_millis, full_in_millis)`.
    fn check(&mut self) -> (bool, u32, u64, u64) {
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

        // How long until the bucket is full again (IETF draft
        // RateLimit-Reset semantics: when the whole quota is back).
        let full_ms = (((self.burst - self.tokens).max(0.0) / self.rate) * 1_000.0).ceil() as u64;

        (allowed, self.tokens.floor() as u32, reset_ms, full_ms)
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
    /// Sustained refill rate for this key, requests per second.
    pub rate: u32,
    /// Bucket capacity: the most requests that can go through back to back.
    /// Reported as `X-RateLimit-Limit`.
    pub limit: u32,
    /// Requests that can still go through right now (bucket tokens, floored).
    pub remaining: u32,
    /// Unix timestamp (seconds) when the bucket is full again, i.e. when
    /// `remaining` is back to `limit`. Equals "now" only for a full bucket.
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

        let (allowed, remaining, reset_ms, full_ms) = bucket.check();
        drop(bucket); // release lock before computing unix timestamp

        let reset_unix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() + full_ms.div_ceil(1_000))
            .unwrap_or(0);

        // X-RateLimit-Limit is the bucket capacity and Remaining the tokens
        // left, so a fresh burst counts down 19, 18, …, 0 and Limit matches
        // how many requests really get through. (RFC-098 reported the
        // per-second rate as Limit and clamped Remaining to it, so a Free
        // burst showed "Limit: 10, Remaining: 10" for the first ten of the
        // twenty requests it allowed — 0.4.0 retest, 2026-10-13.) The
        // sustained rate is in `rate` for error messages.
        RateLimitOutcome {
            allowed,
            rate: rate as u32,
            limit: burst as u32,
            remaining,
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
        assert_eq!(out.rate, DEFAULT_RATE_LIMIT_RPS);
        assert_eq!(out.limit, DEFAULT_RATE_LIMIT_BURST);
    }

    #[test]
    fn per_key_override_applied() {
        let state = RateLimitState::default();
        let id = Uuid::new_v4();
        let out = state.check(id, Some(200), Some(2_000));
        assert_eq!(out.rate, 200);
        assert_eq!(out.limit, 2_000);
        assert_eq!(out.remaining, 1_999);
    }

    /// Free-plan shape (10 r/s, burst 20): Limit is the 20 requests a burst
    /// really allows, and Remaining counts down by one per request.
    #[test]
    fn burst_headers_count_down_from_capacity() {
        let state = RateLimitState::default();
        let id = Uuid::new_v4();
        let remaining: Vec<u32> = (0..20)
            .map(|_| {
                let out = state.check(id, Some(10), Some(20));
                assert!(out.allowed);
                assert_eq!((out.rate, out.limit), (10, 20));
                out.remaining
            })
            .collect();
        assert_eq!(remaining, (0..20).rev().collect::<Vec<u32>>());
        let denied = state.check(id, Some(10), Some(20));
        assert!(!denied.allowed);
        assert_eq!(denied.remaining, 0);
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
