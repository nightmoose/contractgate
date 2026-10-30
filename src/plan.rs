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

/// Monthly cap used for metering, `/usage`, and `/v1/me`.
///
/// `self_hosted` is [`SELF_HOSTED_ENV`]. Hosted (`false`) uses
/// [`monthly_event_limit`]. Self-host is unlimited, the Enterprise cap.
pub fn monthly_event_limit_for(self_hosted: bool, plan: &str) -> Option<i64> {
    if self_hosted {
        None
    } else {
        monthly_event_limit(plan)
    }
}

/// Default per-key rate limit. Self-host uses the paid bucket. A key's own
/// `rate_limit_rps` / `rate_limit_burst` still wins over this default.
pub fn default_rate_limit_for(self_hosted: bool, plan: &str) -> (u32, u32) {
    if self_hosted {
        default_rate_limit("enterprise")
    } else {
        default_rate_limit(plan)
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

/// Explicit opt-in that marks this process as a self-hosted deployment.
/// Hosted production and staging leave it unset. A missing Stripe key is not
/// this signal.
pub const SELF_HOSTED_ENV: &str = "CONTRACTGATE_SELF_HOSTED";

/// `true` only for `1`, `true`, or `TRUE`. Unset, blank, and every other
/// value stay hosted.
pub fn self_hosted_flag(raw: Option<&str>) -> bool {
    matches!(raw.map(str::trim), Some("1" | "true" | "TRUE"))
}

/// Paid for passed-event export, Connect Fivetran, and payload storage.
///
/// `self_hosted` is [`SELF_HOSTED_ENV`]. Hosted (`false`) uses [`is_paid`].
pub fn gate_paid(self_hosted: bool, plan: &str) -> bool {
    self_hosted || is_paid(plan)
}

/// Cursor retention for a passed-event export that [`gate_paid`] allowed.
///
/// Hosted uses the stored plan. Self-host uses the Enterprise window so a
/// free org is not capped at the Pro or Free retention.
pub fn gate_retention_days(self_hosted: bool, plan: &str) -> i32 {
    if self_hosted {
        log_retention_days("enterprise")
    } else {
        log_retention_days(plan)
    }
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

/// 403 body when a free plan calls `GET /v1/contracts/{id}/passed`.
///
/// The plan names are [`PAID_PLANS`] (the same slice [`is_paid`] checks).
pub fn passed_events_upgrade_message() -> String {
    format!(
        "passed-event export is a paid-plan feature; upgrade to {}",
        paid_plans_phrase()
    )
}

/// 403 body when a free plan calls `POST /v1/integrations/fivetran/connect`.
///
/// The plan names are [`PAID_PLANS`] (the same slice [`is_paid`] checks).
pub fn fivetran_connect_upgrade_message() -> String {
    format!(
        "Connect Fivetran is a paid-plan feature; upgrade to {}",
        paid_plans_phrase()
    )
}

/// Audit, quarantine, and passed-event cursor retention, in days.
///
/// Matches `log_retention_days` in migration 037. `growth` and `aisg_pro`
/// are Pro on the two products (90). `enterprise` is 365. Every other plan,
/// including `free` and `aisg_free`, is 7.
pub fn log_retention_days(plan: &str) -> i32 {
    match plan {
        "growth" | "aisg_pro" => 90,
        "enterprise" => 365,
        _ => 7,
    }
}

/// Placeholder pending-quarantine cap for one organization.
///
/// DataContractGate and AISchemaGate share this count. Change the constant
/// to resize the cap. Paid plans and self-host have no cap.
pub const FREE_QUARANTINE_ORG_CAP: i64 = 50_000;

/// Placeholder pending-quarantine cap for one contract on Free.
pub const FREE_QUARANTINE_CONTRACT_CAP: i64 = 10_000;

/// `near` starts at this percent of a cap. `reached` is the cap itself.
/// Reviewed rows stop counting immediately, so quarantine resumes as soon
/// as pending drops below the cap.
pub const QUARANTINE_CAP_NEAR_PERCENT: i64 = 80;

/// How long stored bodies remain after a paid-to-free downgrade.
///
/// The hourly purge reads the same interval from migration 046. This
/// constant is the Rust side of that pair and is checked by the migration test.
#[cfg_attr(not(test), allow(dead_code))]
pub const DOWNGRADE_BODY_GRACE_DAYS: i64 = 30;

/// Placeholder ceiling for one bulk-replay job.
///
/// Reuses [`FREE_QUARANTINE_CONTRACT_CAP`] so the bound is not a separate
/// figure. Set `CONTRACTGATE_BULK_REPLAY_JOB_LIMIT` to a positive integer
/// to override it. A job still replays in batches of 1,000.
pub const BULK_REPLAY_JOB_LIMIT: i64 = FREE_QUARANTINE_CONTRACT_CAP;

/// Short in-process cache for pending counts. Not a customer-facing limit.
pub const QUARANTINE_COUNT_CACHE_SECS: u64 = 5;

pub const BULK_REPLAY_JOB_LIMIT_ENV: &str = "CONTRACTGATE_BULK_REPLAY_JOB_LIMIT";

/// Pending rows a plan may hold. `None` means no cap.
///
/// Unknown plan strings use the Free cap, the same fallback as
/// [`monthly_event_limit`].
pub fn quarantine_row_cap(plan: &str) -> Option<i64> {
    if is_paid(plan) {
        None
    } else {
        Some(FREE_QUARANTINE_ORG_CAP)
    }
}

pub fn per_contract_quarantine_cap(plan: &str) -> Option<i64> {
    if is_paid(plan) {
        None
    } else {
        Some(FREE_QUARANTINE_CONTRACT_CAP)
    }
}

pub fn quarantine_row_cap_for(self_hosted: bool, plan: &str) -> Option<i64> {
    if self_hosted {
        None
    } else {
        quarantine_row_cap(plan)
    }
}

pub fn per_contract_quarantine_cap_for(self_hosted: bool, plan: &str) -> Option<i64> {
    if self_hosted {
        None
    } else {
        per_contract_quarantine_cap(plan)
    }
}

pub fn bulk_replay_allowed(self_hosted: bool, plan: &str) -> bool {
    gate_paid(self_hosted, plan)
}

/// Positive override, or [`BULK_REPLAY_JOB_LIMIT`]. A bad value is ignored
/// and not logged.
pub fn bulk_replay_job_limit(raw: Option<&str>) -> i64 {
    match raw.map(str::trim).filter(|value| !value.is_empty()) {
        Some(value) => value
            .parse::<i64>()
            .ok()
            .filter(|n| *n > 0)
            .unwrap_or(BULK_REPLAY_JOB_LIMIT),
        None => BULK_REPLAY_JOB_LIMIT,
    }
}

pub fn plan_required_message(feature: &str) -> &'static str {
    match feature {
        "bulk_replay" => {
            "Bulk replay is included with Pro. It runs in the background and uses the stored record body."
        }
        "quarantine_replay" => {
            "Replay is included with Pro. Quarantined records on Free keep the failure details without the record body."
        }
        _ => "This action is included with Pro.",
    }
}

/// 403 `plan_required` unless the org is paid or this process is self-hosted.
/// No org (local dev without tenancy) is allowed.
pub async fn require_plan_feature(
    state: &crate::AppState,
    org_id: Option<uuid::Uuid>,
    feature: &'static str,
) -> Result<(), crate::error::AppError> {
    if state.self_hosted || org_id.is_none() {
        return Ok(());
    }
    let org_id = org_id.expect("org checked");
    let plan = crate::storage::get_org_plan(&state.db, org_id)
        .await?
        .unwrap_or_else(|| "free".to_string());
    let allowed = if feature == "bulk_replay" {
        bulk_replay_allowed(false, &plan)
    } else {
        gate_paid(false, &plan)
    };
    if allowed {
        return Ok(());
    }
    Err(crate::error::AppError::PlanRequired {
        feature,
        current_plan: plan,
        message: plan_required_message(feature),
    })
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
#[cfg(test)]
pub fn payloads_stored(plan: &str, org_switch: bool, contract_switch: bool) -> bool {
    payloads_stored_for(false, plan, org_switch, contract_switch)
}

/// Same rule as [`payloads_stored`]. `self_hosted` is [`SELF_HOSTED_ENV`].
pub fn payloads_stored_for(
    self_hosted: bool,
    plan: &str,
    org_switch: bool,
    contract_switch: bool,
) -> bool {
    gate_paid(self_hosted, plan) && org_switch && contract_switch
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
#[cfg(test)]
pub fn effective_store_event_payloads(plan: Option<&str>, org_switch: bool, stored: bool) -> bool {
    effective_store_event_payloads_for(false, plan, org_switch, stored)
}

/// Same report as [`effective_store_event_payloads`]. `self_hosted` is
/// [`SELF_HOSTED_ENV`].
pub fn effective_store_event_payloads_for(
    self_hosted: bool,
    plan: Option<&str>,
    org_switch: bool,
    stored: bool,
) -> bool {
    match plan {
        None => stored,
        Some(plan) => payloads_stored_for(self_hosted, plan, org_switch, stored),
    }
}

/// Message returned (403) when a below-Enterprise org tries to turn on
/// Kafka/Kinesis streaming ingress.
pub const STREAMING_REQUIRES_ENTERPRISE: &str =
    "Kafka/Kinesis streaming ingress requires the Enterprise plan (available on request). \
     Use POST /v1/ingest instead, or contact datacontractgate@nightmoose.com.";

/// Whether a plan may enable Kafka/Kinesis streaming ingress. Enterprise only:
/// streaming is not part of any self-serve plan. Self-host allows it for
/// every stored plan.
pub fn streaming_allowed(plan: &str) -> bool {
    plan == "enterprise"
}

#[cfg(test)]
fn streaming_allowed_for(self_hosted: bool, plan: &str) -> bool {
    self_hosted || streaming_allowed(plan)
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
    if state.self_hosted {
        return Ok(());
    }
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

    #[test]
    fn passed_events_upgrade_message_names_exactly_the_paid_plans() {
        let msg = passed_events_upgrade_message();
        assert!(msg.starts_with("passed-event export is a paid-plan feature; upgrade to "));
        let named: Vec<&str> = msg
            .rsplit_once("upgrade to ")
            .unwrap()
            .1
            .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
            .filter(|w| !w.is_empty() && *w != "or")
            .collect();
        assert_eq!(named, PAID_PLANS);
    }

    #[test]
    fn self_hosted_flag_is_explicit_and_free_is_gated_as_paid() {
        assert!(!self_hosted_flag(None));
        assert!(!self_hosted_flag(Some("")));
        assert!(!self_hosted_flag(Some("   ")));
        assert!(!self_hosted_flag(Some("false")));
        assert!(!self_hosted_flag(Some("0")));
        assert!(!self_hosted_flag(Some("yes")));
        assert!(self_hosted_flag(Some("true")));
        assert!(self_hosted_flag(Some("TRUE")));
        assert!(self_hosted_flag(Some("1")));
        assert!(self_hosted_flag(Some("  true  ")));

        for plan in ["free", "aisg_free"] {
            assert!(gate_paid(true, plan), "{plan}");
            assert!(!gate_paid(false, plan), "{plan}");
            assert_eq!(gate_retention_days(true, plan), 365, "{plan}");
            assert_eq!(monthly_event_limit_for(true, plan), None, "{plan}");
            assert_eq!(default_rate_limit_for(true, plan), (100, 200), "{plan}");
            assert_eq!(
                monthly_event_limit_for(false, plan),
                Some(100_000),
                "{plan}"
            );
            assert_eq!(default_rate_limit_for(false, plan), (10, 20), "{plan}");
            assert!(streaming_allowed_for(true, plan), "{plan}");
            assert!(!streaming_allowed_for(false, plan), "{plan}");
            assert!(payloads_stored_for(true, plan, true, true), "{plan}");
            assert!(!payloads_stored_for(false, plan, true, true), "{plan}");
            assert!(!payloads_stored_for(true, plan, false, true), "{plan}");
            assert!(effective_store_event_payloads_for(
                true,
                Some(plan),
                true,
                true
            ));
            assert!(!effective_store_event_payloads_for(
                false,
                Some(plan),
                true,
                true
            ));
        }
        assert!(gate_paid(false, "growth"));
        assert_eq!(gate_retention_days(false, "growth"), 90);
        assert_eq!(gate_retention_days(true, "enterprise"), 365);
        assert_eq!(monthly_event_limit_for(false, "growth"), Some(25_000_000));
        assert!(!streaming_allowed_for(false, "growth"));
        assert!(streaming_allowed_for(false, "enterprise"));
        assert!(!is_paid("free"));
    }

    #[test]
    fn log_retention_matches_migration_037() {
        assert_eq!(log_retention_days("free"), 7);
        assert_eq!(log_retention_days("aisg_free"), 7);
        assert_eq!(log_retention_days("growth"), 90);
        assert_eq!(log_retention_days("aisg_pro"), 90);
        assert_eq!(log_retention_days("enterprise"), 365);
        assert_eq!(log_retention_days("bogus"), 7);
    }

    #[test]
    fn quarantine_caps_are_shared_placeholders() {
        for free in ["free", "aisg_free", "bogus"] {
            assert_eq!(quarantine_row_cap(free), Some(FREE_QUARANTINE_ORG_CAP));
            assert_eq!(
                per_contract_quarantine_cap(free),
                Some(FREE_QUARANTINE_CONTRACT_CAP)
            );
            assert!(!bulk_replay_allowed(false, free), "{free}");
            assert!(bulk_replay_allowed(true, free), "{free}");
            assert_eq!(quarantine_row_cap_for(true, free), None);
            assert_eq!(per_contract_quarantine_cap_for(true, free), None);
        }
        for paid in ["growth", "aisg_pro", "enterprise"] {
            assert_eq!(quarantine_row_cap(paid), None, "{paid}");
            assert_eq!(per_contract_quarantine_cap(paid), None, "{paid}");
            assert!(bulk_replay_allowed(false, paid), "{paid}");
        }
        assert_eq!(FREE_QUARANTINE_ORG_CAP, 50_000);
        assert_eq!(FREE_QUARANTINE_CONTRACT_CAP, 10_000);
        assert_eq!(QUARANTINE_CAP_NEAR_PERCENT, 80);
        assert_eq!(DOWNGRADE_BODY_GRACE_DAYS, 30);
        assert_eq!(BULK_REPLAY_JOB_LIMIT, FREE_QUARANTINE_CONTRACT_CAP);
        assert_eq!(bulk_replay_job_limit(None), BULK_REPLAY_JOB_LIMIT);
        assert_eq!(bulk_replay_job_limit(Some("2500")), 2500);
        assert_eq!(bulk_replay_job_limit(Some("0")), BULK_REPLAY_JOB_LIMIT);
        assert_eq!(bulk_replay_job_limit(Some("nope")), BULK_REPLAY_JOB_LIMIT);
        assert!(plan_required_message("quarantine_replay").contains("Pro"));
        assert!(!plan_required_message("bulk_replay").contains("ContractGate"));
    }

    #[test]
    fn migration_046_redacts_free_bodies_and_keeps_rows() {
        let sql = include_str!("../supabase/migrations/046_quarantine_plan_split.sql");
        let redaction = section(sql, "BEGIN free body redaction", "END free body redaction");
        assert!(redaction.contains("payload = null"));
        assert!(redaction.contains("payload_redacted = true"));
        assert!(redaction.contains("'free'"));
        assert!(redaction.contains("'aisg_free'"));
        assert!(
            !redaction.to_ascii_lowercase().contains("delete from"),
            "free body cleanup must keep the rows"
        );
        let grace = section(sql, "BEGIN grace redaction", "END grace redaction");
        assert!(grace.contains(&format!("interval '{} days'", DOWNGRADE_BODY_GRACE_DAYS)));
        assert!(grace.contains("payload_redacted = true"));
        assert!(!grace.to_ascii_lowercase().contains("delete from"));
        assert!(sql.contains("idx_quarantine_org_status_created"));
        assert!(sql.contains("quarantine_replay_jobs"));
    }

    #[test]
    fn migration_046_is_additive_after_045() {
        let sql = include_str!("../supabase/migrations/046_quarantine_plan_split.sql");
        let prior = include_str!("../supabase/migrations/045_drop_mode.sql");
        let lower = sql.to_ascii_lowercase();
        assert!(!lower.contains("drop table"));
        assert!(!lower.contains("drop column"));
        assert!(!lower.contains("truncate"));
        assert!(!lower.contains("concurrently"));
        assert_eq!(lower.matches("create table").count(), 1);
        assert!(lower.contains("add column if not exists org_id uuid"));
        assert!(lower.contains("add column if not exists payload_redact_after"));
        assert!(lower.contains("create table if not exists public.quarantine_replay_jobs"));
        assert!(lower.contains("create index if not exists"));
        assert!(lower.contains("payload = 'null'::jsonb"));
        assert!(lower.contains("interval '30 days'"));
        assert!(!lower.contains("pg_net"));
        assert!(!lower.contains("send_email"));
        let signature =
            "returns table (audit_deleted integer, quarantine_deleted integer, drops_deleted integer)";
        assert!(lower.contains(signature));
        assert!(prior.to_ascii_lowercase().contains(signature));
        for kept in [
            "delete from public.dropped_event_counts",
            "delete from public.dropped_event_samples",
            "delete from public.quarantine_guard_events",
            "delete from public.quarantine_volume_minute",
            "delete from public.quarantine_events where id = any",
            "delete from public.audit_log where id = any",
            "perform public.redact_elapsed_payload_grace(p_batch)",
        ] {
            assert!(lower.contains(kept), "{kept}");
        }
        let counts = crate::storage::PENDING_QUARANTINE_COUNTS_SQL;
        assert!(counts.contains("status = 'pending'"));
        assert!(!counts.to_ascii_lowercase().contains("direction"));
    }

    fn section<'a>(sql: &'a str, start: &str, end: &str) -> &'a str {
        let from = sql.find(start).expect(start) + start.len();
        let to = sql[from..].find(end).expect(end) + from;
        &sql[from..to]
    }
}
