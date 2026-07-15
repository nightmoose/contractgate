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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_per_plan() {
        assert_eq!(monthly_event_limit("free"), Some(1_000_000));
        assert_eq!(monthly_event_limit("growth"), Some(50_000_000));
        assert_eq!(monthly_event_limit("enterprise"), None);
        // Unknown → most restrictive.
        assert_eq!(monthly_event_limit("bogus"), Some(1_000_000));
    }
}
