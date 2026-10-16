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

/// Plans for which [`is_paid`] is true. Payload-storage upgrade copy is built
/// from this list so it cannot name a different set than the gate.
///
/// `growth` is DataContractGate Pro, `aisg_pro` is AISchemaGate Pro, and
/// `enterprise` is shared. `free` and `aisg_free` are absent on purpose.
pub const PAID_PLANS: &[&str] = &["growth", "aisg_pro", "enterprise"];

/// Whether a plan is a paid plan. Paid plans unlock payload storage (RFC-086)
/// and the org settings that depend on it.
pub fn is_paid(plan: &str) -> bool {
    PAID_PLANS.contains(&plan)
}

/// English list of [`PAID_PLANS`], e.g. `growth, aisg_pro, or enterprise`.
pub fn paid_plans_phrase() -> String {
    english_or(PAID_PLANS)
}

/// 400 body when a non-paid plan tries to turn payload storage on.
///
/// The plan names are [`PAID_PLANS`] (the same slice [`is_paid`] checks).
pub fn payload_storage_upgrade_message() -> String {
    format!(
        "event payload storage is a paid-plan feature; upgrade to {}",
        paid_plans_phrase()
    )
}

fn english_or(items: &[&str]) -> String {
    match items.split_last() {
        None => String::new(),
        Some((only, [])) => (*only).to_string(),
        Some((last, [only])) => format!("{only} or {last}"),
        Some((last, rest)) => format!("{}, or {last}", rest.join(", ")),
    }
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

/// Effective `store_event_payloads` for contract list / GET responses.
///
/// This is the same decision ingest, egress, and the Kafka/Kinesis consumers
/// make in [`payloads_stored`]: a body is stored only on a paid plan
/// ([`is_paid`]) with the org master switch on and the per-contract override
/// on. `free`, `aisg_free`, and unknown plans report `false` even when both
/// columns are on. A paid plan whose org switch is still off (the column
/// default) also reports `false` — reporting the per-contract column alone
/// told the API the body would be stored while quarantine inserts redacted it.
///
/// The columns are not rewritten. `plan == None` is self-host / dev (no org
/// on the request). Ingest stores in that case regardless of the org switch,
/// so the per-contract column is reported as stored.
pub fn effective_store_event_payloads(plan: Option<&str>, org_switch: bool, stored: bool) -> bool {
    match plan {
        None => stored,
        Some(plan) => payloads_stored(plan, org_switch, stored),
    }
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
    fn effective_store_event_payloads_matches_ingest() {
        // Free / AISchemaGate Free / unknown: columns may both be on, response is false.
        for plan in ["free", "aisg_free", "bogus", ""] {
            assert!(
                !effective_store_event_payloads(Some(plan), true, true),
                "{plan} must report false"
            );
            assert!(
                !effective_store_event_payloads(Some(plan), true, false),
                "{plan}"
            );
            assert!(
                !effective_store_event_payloads(Some(plan), false, true),
                "{plan}"
            );
            assert_eq!(
                effective_store_event_payloads(Some(plan), true, true),
                payloads_stored(plan, true, true),
                "{plan}"
            );
        }
        // Paid + org switch on reports the per-contract override.
        // Paid + org switch off (column default) reports false — ingest redacts.
        for plan in ["growth", "aisg_pro", "enterprise"] {
            assert!(
                effective_store_event_payloads(Some(plan), true, true),
                "{plan} paid+on"
            );
            assert!(
                !effective_store_event_payloads(Some(plan), true, false),
                "{plan} paid + contract off"
            );
            assert!(
                !effective_store_event_payloads(Some(plan), false, true),
                "{plan} paid + org switch off must not claim storage"
            );
            assert_eq!(
                effective_store_event_payloads(Some(plan), true, true),
                payloads_stored(plan, true, true),
                "{plan}"
            );
            assert_eq!(
                effective_store_event_payloads(Some(plan), false, true),
                payloads_stored(plan, false, true),
                "{plan} org switch"
            );
        }
        // Self-host / no org: report the column. Ingest stores (org switch unused).
        assert!(effective_store_event_payloads(None, false, true));
        assert!(!effective_store_event_payloads(None, true, false));
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

    #[test]
    fn upgrade_message_names_exactly_the_paid_plans() {
        let msg = payload_storage_upgrade_message();
        assert!(msg.starts_with("event payload storage is a paid-plan feature; upgrade to "));
        let named: Vec<&str> = msg
            .rsplit_once("upgrade to ")
            .unwrap()
            .1
            .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
            .filter(|w| !w.is_empty() && *w != "or")
            .collect();
        assert_eq!(named, PAID_PLANS);
        for plan in PAID_PLANS {
            assert!(is_paid(plan), "{plan}");
        }
        for plan in ["free", "aisg_free", "pro", "Growth", "Enterprise"] {
            assert!(!is_paid(plan), "{plan} must not qualify");
            assert!(
                !named.contains(&plan),
                "{msg} must not name {plan} separately from the paid-plan list"
            );
        }
    }
}
