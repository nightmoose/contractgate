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
///
/// RFC-098: burst is 2× rps (was 10×). The 10× factor let a Free key spike
/// momentarily at 100 r/s, 10× the documented 10 r/s, and failed the 2026-10-12
/// E2E rate-limit test. 2× keeps room for a 20-event SDK batch without denials.
pub fn default_rate_limit(plan: &str) -> (u32, u32) {
    if is_paid(plan) {
        (100, 200)
    } else {
        (10, 20)
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

/// Message returned (403) when a below-Enterprise org tries to turn on
/// Kafka/Kinesis streaming ingress.
pub const STREAMING_REQUIRES_ENTERPRISE: &str =
    "Kafka/Kinesis streaming ingress requires the Enterprise plan (available on request). \
     Use POST /v1/ingest instead, or contact datacontractgate@nightmoose.com.";

/// Whether a plan may enable Kafka/Kinesis streaming ingress. Enterprise only:
/// streaming is not part of any self-serve plan.
pub fn streaming_allowed(plan: &str) -> bool {
    plan == "enterprise"
}

/// Pure gate: `Ok` for Enterprise, 403 [`crate::error::AppError::Forbidden`]
/// otherwise. `None` (org row missing) is treated as below Enterprise.
pub fn check_streaming_plan(plan: Option<&str>) -> Result<(), crate::error::AppError> {
    match plan {
        Some(p) if streaming_allowed(p) => Ok(()),
        _ => Err(crate::error::AppError::Forbidden(
            STREAMING_REQUIRES_ENTERPRISE.to_string(),
        )),
    }
}

/// Server-side gate for routes that turn streaming ingress on.
///
/// - Auth configured and no org on the request → 401.
/// - No org and auth off (self-host / dev, no tenancy) → allowed; there is no
///   plan to check.
/// - Org present → its plan must be Enterprise, else 403.
pub async fn require_streaming_plan(
    state: &crate::AppState,
    org_id: Option<uuid::Uuid>,
) -> Result<(), crate::error::AppError> {
    let Some(oid) = org_id else {
        if state.auth_configured() {
            return Err(crate::error::AppError::Unauthorized);
        }
        return Ok(());
    };
    let plan = crate::storage::get_org_plan(&state.db, oid).await?;
    check_streaming_plan(plan.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streaming_gate_status_codes() {
        use axum::{http::StatusCode, response::IntoResponse};
        assert!(check_streaming_plan(Some("enterprise")).is_ok());
        for p in [
            Some("free"),
            Some("growth"),
            Some("aisg_free"),
            Some("aisg_pro"),
            Some("bogus"),
            None,
        ] {
            let err = check_streaming_plan(p).expect_err("below enterprise must be denied");
            assert!(err.to_string().contains("requires the Enterprise plan"));
            assert_eq!(err.into_response().status(), StatusCode::FORBIDDEN, "{p:?}");
        }
    }

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
        assert_eq!(default_rate_limit("free"), (10, 20));
        assert_eq!(default_rate_limit("aisg_free"), (10, 20));
        assert_eq!(default_rate_limit("growth"), (100, 200));
        assert_eq!(default_rate_limit("aisg_pro"), (100, 200));
        assert_eq!(default_rate_limit("enterprise"), (100, 200));
        assert_eq!(default_rate_limit("bogus"), (10, 20));
    }
}
