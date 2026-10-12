//! RFC-083 — canonical plan → monthly event limit mapping.
//!
//! Backend is the single source of truth for tier limits; the `/usage` response
//! returns the limit so the dashboard never hardcodes it.

/// Monthly event limit for a plan. `None` means unlimited (Enterprise).
/// Unknown plan strings fall back to the DataContractGate Free limit.
///
/// `aisg_*` are AISchemaGate's plans (migration 036). Same backend and
/// tenant, its own tiers: one LLM output validated through egress or ingest
/// is one event.
pub fn monthly_event_limit(plan: &str) -> Option<i64> {
    match plan {
        "free" => Some(1_000_000),
        "growth" => Some(50_000_000),
        "enterprise" => None,
        "aisg_free" => Some(10_000),
        "aisg_pro" => Some(250_000),
        _ => Some(1_000_000),
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
        assert_eq!(monthly_event_limit("free"), Some(1_000_000));
        assert_eq!(monthly_event_limit("growth"), Some(50_000_000));
        assert_eq!(monthly_event_limit("enterprise"), None);
        assert_eq!(monthly_event_limit("aisg_free"), Some(10_000));
        assert_eq!(monthly_event_limit("aisg_pro"), Some(250_000));
        // Unknown → DataContractGate Free.
        assert_eq!(monthly_event_limit("bogus"), Some(1_000_000));
    }
}
