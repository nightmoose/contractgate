//! Per-IP limit on failed authentication attempts.
//!
//! A missing or rejected credential used to be a Postgres lookup (for any
//! string that looks like a key) plus a warning on every request. The key
//! cache absorbs repeats. This limiter absorbs a client that rotates junk
//! keys: after [`AUTH_FAIL_LIMIT`] failures in [`WINDOW`] the next failure
//! from that address is HTTP 429 with `Retry-After`.
//!
//! The check runs only after the credential has already been rejected. A key
//! or session that validates is not counted and is not blocked, including
//! when the address is already over the limit.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use contractgate::rate_limit::RateLimitOutcome;
use uuid::Uuid;

/// Failures answered with 401 before the next one is 429.
pub const AUTH_FAIL_LIMIT: u32 = 30;

/// Window the failure count is measured over.
pub const WINDOW: Duration = Duration::from_secs(60);

/// One bucket per address. Past this, the oldest window is dropped.
const MAX_BUCKETS: usize = 10_000;

struct Window {
    count: u32,
    started: Instant,
}

/// What the auth middleware should do with a rejection.
pub struct AuthFailDecision {
    /// `Some` when this failure must be HTTP 429 instead of 401.
    pub outcome: Option<RateLimitOutcome>,
    /// True on the request that first crosses the limit in this window.
    pub warn: bool,
}

pub struct AuthFailLimiter {
    inner: Mutex<HashMap<Uuid, Window>>,
}

impl Default for AuthFailLimiter {
    fn default() -> Self {
        AuthFailLimiter {
            inner: Mutex::new(HashMap::new()),
        }
    }
}

impl AuthFailLimiter {
    fn lock(&self) -> MutexGuard<'_, HashMap<Uuid, Window>> {
        self.inner.lock().unwrap_or_else(|e| {
            tracing::error!("auth-fail limiter Mutex was poisoned — recovering inner value");
            e.into_inner()
        })
    }

    /// Count one rejected credential for `ip` (a stable id derived from the
    /// client address, not the address itself).
    pub fn record(&self, ip: Uuid) -> AuthFailDecision {
        self.record_at(ip, Instant::now())
    }

    fn record_at(&self, ip: Uuid, now: Instant) -> AuthFailDecision {
        let mut map = self.lock();
        if map.len() >= MAX_BUCKETS && !map.contains_key(&ip) {
            map.retain(|_, window| now.saturating_duration_since(window.started) < WINDOW);
            while map.len() >= MAX_BUCKETS {
                let Some(oldest) = map
                    .iter()
                    .min_by_key(|(_, window)| window.started)
                    .map(|(key, _)| *key)
                else {
                    break;
                };
                map.remove(&oldest);
            }
        }
        let window = map.entry(ip).or_insert(Window {
            count: 0,
            started: now,
        });
        if now.saturating_duration_since(window.started) >= WINDOW {
            window.count = 0;
            window.started = now;
        }
        window.count = window.count.saturating_add(1);
        let count = window.count;
        let started = window.started;
        drop(map);

        let limited = count > AUTH_FAIL_LIMIT;
        let elapsed = now.saturating_duration_since(started);
        let remain = WINDOW.saturating_sub(elapsed);
        let retry_after_ms = u64::try_from(remain.as_millis())
            .unwrap_or(60_000)
            .max(1_000);
        let reset_unix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs().saturating_add(remain.as_secs().max(1)))
            .unwrap_or(0);
        AuthFailDecision {
            outcome: limited.then_some(RateLimitOutcome {
                allowed: false,
                rate: AUTH_FAIL_LIMIT,
                limit: AUTH_FAIL_LIMIT,
                remaining: 0,
                reset_unix,
                retry_after_ms,
            }),
            warn: count == AUTH_FAIL_LIMIT + 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    #[test]
    fn limit_is_thirty_failures_per_minute() {
        let limiter = AuthFailLimiter::default();
        let now = Instant::now();
        for i in 1..=AUTH_FAIL_LIMIT {
            let decision = limiter.record_at(ip(1), now);
            assert!(decision.outcome.is_none(), "failure {i} is still 401");
            assert!(!decision.warn);
        }
        let blocked = limiter.record_at(ip(1), now);
        assert!(blocked.outcome.is_some());
        assert!(blocked.warn, "the crossing is the one warn");
        let again = limiter.record_at(ip(1), now);
        assert!(again.outcome.is_some());
        assert!(!again.warn, "later 429s stay at debug");
        let retry_ms = blocked.outcome.unwrap().retry_after_ms;
        assert!((1_000..=60_000).contains(&retry_ms));
    }

    #[test]
    fn a_different_address_is_not_blocked() {
        let limiter = AuthFailLimiter::default();
        let now = Instant::now();
        for _ in 0..=AUTH_FAIL_LIMIT {
            limiter.record_at(ip(1), now);
        }
        assert!(limiter.record_at(ip(2), now).outcome.is_none());
    }

    #[test]
    fn the_window_resets() {
        let limiter = AuthFailLimiter::default();
        let start = Instant::now();
        for _ in 0..=AUTH_FAIL_LIMIT {
            limiter.record_at(ip(7), start);
        }
        assert!(limiter.record_at(ip(7), start).outcome.is_some());
        let later = start + WINDOW;
        let decision = limiter.record_at(ip(7), later);
        assert!(
            decision.outcome.is_none(),
            "a failure on a new window is 401 again"
        );
    }
}
