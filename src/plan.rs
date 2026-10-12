//! RFC-083 — canonical plan → monthly event limit mapping.
//!
//! Backend is the single source of truth for tier limits; the `/usage` response
//! returns the limit so the dashboard never hardcodes it.

/// Monthly event limit for a plan. `None` means unlimited (Enterprise).
/// Unknown plan strings fall back to the DataContractGate Free limit.
///
/// One plan ladder for both products (decided 2026-10-12): `free` /
/// `aisg_free` is Free, `growth` / `aisg_pro` is Pro ($199/mo). The `aisg_*`
/// values (migration 036) only record which product the account came from;
/// limits are identical. One LLM output validated is one event.
pub fn monthly_event_limit(plan: &str) -> Option<i64> {
    match plan {
        "free" | "aisg_free" => Some(100_000),
        "growth" | "aisg_pro" => Some(25_000_000),
        "enterprise" => None,
        _ => Some(100_000),
    }
}

/// Default per-key rate limit `(requests/sec, burst)` when the key has no
/// override of its own (RFC-021 `api_keys.rate_limit_rps/burst`). Free keys
/// get a tighter bucket so a runaway loop can't hammer the gateway; 100k a
/// month averages well under 1 request/sec.
pub fn default_rate_limit(plan: &str) -> (u32, u32) {
    if is_paid(plan) {
        (100, 1_000)
    } else {
        (10, 100)
    }
}

/// Whether a plan is a paid plan. Paid plans unlock payload storage (RFC-086)
/// and the org settings that depend on it.
pub fn is_paid(plan: &str) -> bool {
    matches!(plan, "growth" | "enterprise" | "aisg_pro")
}

/// Whether an event body should be durably stored for this org/contract.
///
/// RFC-086: bodies are stored only on a paid plan (see [`is_paid`]) with
/// the org master switch on and the per-contract override on. Free/unknown
/// plans never store, regardless of the flags. Self-host/dev (no org row) is
/// handled by the caller before this is reached.
pub fn payloads_stored(plan: &str, org_switch: bool, contract_switch: bool) -> bool {
    is_paid(plan) && org_switch && contract_switch
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payloads_stored_truth_table() {
        // Free never stores, whatever the flags say.
        assert!(!payloads_stored("free", true, true));
        assert!(!payloads_stored("bogus", true, true));
        // Paid stores only when both switches are on.
        assert!(payloads_stored("growth", true, true));
        assert!(payloads_stored("enterprise", true, true));
        assert!(!payloads_stored("growth", false, true)); // org master off
        assert!(!payloads_stored("growth", true, false)); // per-contract off
        assert!(!payloads_stored("enterprise", false, false));
        // AISchemaGate: Pro is paid, Free is not.
        assert!(payloads_stored("aisg_pro", true, true));
        assert!(!payloads_stored("aisg_free", true, true));
    }

    #[test]
    fn limits_per_plan() {
        // Same ladder for both products.
        for free in ["free", "aisg_free"] {
            assert_eq!(monthly_event_limit(free), Some(100_000));
        }
        for pro in ["growth", "aisg_pro"] {
            assert_eq!(monthly_event_limit(pro), Some(25_000_000));
        }
        assert_eq!(monthly_event_limit("enterprise"), None);
        // Unknown → Free.
        assert_eq!(monthly_event_limit("bogus"), Some(100_000));
    }

    #[test]
    fn rate_limit_defaults_per_plan() {
        assert_eq!(default_rate_limit("free"), (10, 100));
        assert_eq!(default_rate_limit("aisg_free"), (10, 100));
        assert_eq!(default_rate_limit("growth"), (100, 1_000));
        assert_eq!(default_rate_limit("aisg_pro"), (100, 1_000));
        assert_eq!(default_rate_limit("enterprise"), (100, 1_000));
        assert_eq!(default_rate_limit("bogus"), (10, 100));
    }
}
