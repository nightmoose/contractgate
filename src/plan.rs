//! RFC-083 — canonical plan → monthly event limit mapping.
//!
//! Backend is the single source of truth for tier limits; the `/usage` response
//! returns the limit so the dashboard never hardcodes it.

/// Monthly event limit for a plan. `None` means unlimited (Enterprise).
/// Unknown plan strings fall back to the most restrictive (Free) limit.
pub fn monthly_event_limit(plan: &str) -> Option<i64> {
    match plan {
        "free" => Some(1_000_000),
        "growth" => Some(50_000_000),
        "enterprise" => None,
        _ => Some(1_000_000),
    }
}

/// Whether an event body should be durably stored for this org/contract.
///
/// RFC-086: bodies are stored only on a paid plan (`growth`/`enterprise`) with
/// the org master switch on and the per-contract override on. Free/unknown
/// plans never store, regardless of the flags. Self-host/dev (no org row) is
/// handled by the caller before this is reached.
pub fn payloads_stored(plan: &str, org_switch: bool, contract_switch: bool) -> bool {
    let paid = matches!(plan, "growth" | "enterprise");
    paid && org_switch && contract_switch
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
    }

    #[test]
    fn limits_per_plan() {
        assert_eq!(monthly_event_limit("free"), Some(1_000_000));
        assert_eq!(monthly_event_limit("growth"), Some(50_000_000));
        assert_eq!(monthly_event_limit("enterprise"), None);
        // Unknown → most restrictive.
        assert_eq!(monthly_event_limit("bogus"), Some(1_000_000));
    }
}
