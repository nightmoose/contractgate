//! Drop mode: what a failed record does, and when the safety cap changes it.
//!
//! Quarantine is the default. Drop counts the failure and stores neither the
//! record nor an audit row. The safety cap ships watch-only: it records that
//! a threshold would have switched the contract to drop, and it does not
//! switch until enforcement is enabled. Thresholds are caller-configured.
//!
//! The account cap replaces the contract cap when the account has a threshold.
//! A tripped enforcing cap forces drop. A request cannot choose quarantine
//! while that cap is tripped. A locked contract rejects a request that asks
//! for a different mode.

use chrono::{DateTime, Duration, Timelike, Utc};
use serde::Serialize;

use crate::contract::{GuardScope, OnFailMode, OnFailSource, QuarantineGuardConfig};

/// How long drop-count buckets are kept. The retention purge uses the same window.
pub const DROP_RETENTION_DAYS: i64 = 30;

/// Metadata-only examples kept per contract, version, direction, and minute.
pub const SAMPLE_CAP: usize = 5;

/// Quarantine inserts already counted, plus the batch under consideration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct VolumeWindow {
    pub minute: u64,
    pub hour: u64,
}

/// A stored enforcing trip. `tripped_at` set means the cap has switched modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StoredTrip {
    pub tripped_at: Option<DateTime<Utc>>,
    pub trip_until: Option<DateTime<Utc>>,
}

impl StoredTrip {
    fn active(self, now: DateTime<Utc>) -> bool {
        match self.tripped_at {
            None => false,
            Some(_) => self.trip_until.is_none_or(|until| until > now),
        }
    }

    fn cooldown_elapsed(self, now: DateTime<Utc>) -> bool {
        match (self.tripped_at, self.trip_until) {
            (Some(_), Some(until)) => until <= now,
            _ => false,
        }
    }
}

/// One scope's configuration, current volume, and stored trip.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ScopeView {
    pub config: QuarantineGuardConfig,
    pub volume: VolumeWindow,
    pub trip: StoredTrip,
}

/// Side effects a live (non-dry-run) request should persist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeEffect {
    pub scope: GuardScope,
    pub trip: bool,
    pub reset: bool,
    pub would_trip: bool,
    pub until: Option<DateTime<Utc>>,
    pub window: Option<&'static str>,
    pub observed: u64,
    pub threshold: u64,
}

/// Reported on the response when a cap is enforcing or would have tripped.
/// Omitted by callers when both flags are false.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutoTripReport {
    pub active: bool,
    pub would_trip: bool,
    pub enforcing: bool,
    pub scope: GuardScope,
    pub until: Option<DateTime<Utc>>,
}

/// Inputs for one batch. `failed` is the number of records that did not pass.
#[derive(Debug, Clone)]
pub struct ResolveInput {
    pub request: Option<OnFailMode>,
    pub contract_mode: OnFailMode,
    /// True when the contract document or the live override set `on_fail`.
    pub contract_explicit: bool,
    pub locked: bool,
    /// Set only when the account cap has a positive threshold.
    pub account: Option<ScopeView>,
    pub contract_guard: ScopeView,
    pub now: DateTime<Utc>,
    pub failed: usize,
    pub dry_run: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OnFailDecision {
    pub mode: OnFailMode,
    pub source: OnFailSource,
    pub auto_trip: Option<AutoTripReport>,
    /// The request asked for a mode the contract owner locked out.
    pub lock_rejected: bool,
    /// Present only for a live request that should write cap state.
    pub effect: Option<ScopeEffect>,
    pub sample_drops: bool,
    pub quarantined: usize,
    pub dropped: usize,
    /// Set by the plan-cap layer. The safety-cap resolver leaves this empty.
    pub quarantine_cap: Option<QuarantineCapView>,
}

/// Pending quarantine rows against the Free caps. Omitted when the plan has no cap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct QuarantineCapView {
    pub limit: i64,
    pub pending: i64,
    /// `ok`, `near`, or `reached`.
    pub state: &'static str,
    /// `org` or `contract`: the cap closest to full.
    pub scope: &'static str,
    pub org_limit: i64,
    pub org_pending: i64,
    pub contract_limit: i64,
    pub contract_pending: i64,
}

/// Counts and limits for one ingest or egress batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlanCapCounts {
    pub org_pending: i64,
    pub org_limit: Option<i64>,
    pub contract_pending: i64,
    pub contract_limit: Option<i64>,
    /// Percent of a cap at which `state` becomes `near`. 100 is the reached line.
    pub near_percent: i64,
}

/// Resolve precedence:
/// 1. `dry_run` writes nothing. It does not skip the mode decision.
/// 2. An enforcing trip forces drop, including the batch that crosses the
///    threshold. A request cannot choose quarantine, and a lock cannot turn
///    that batch into a 400.
/// 3. A locked contract rejects a conflicting request (the caller returns 400).
/// 4. Otherwise the request, then the contract document, then quarantine.
///
/// The account cap replaces the contract cap when the account has a threshold.
/// Watch-only caps set `would_trip` and leave the mode alone.
pub fn resolve_on_fail(input: &ResolveInput) -> OnFailDecision {
    let (base_mode, base_source, lock_rejected) = base_mode(input);
    let (scope, view) = winning_scope(input);
    let reset = view.trip.cooldown_elapsed(input.now);
    let enforcing_clock = view.trip.active(input.now) && view.config.enforce;

    // Only a batch that would insert quarantine rows counts toward the cap.
    // A lock rejection, a dry run, drop mode, and an already-enforcing trip
    // do not insert those rows.
    let counts_toward_cap =
        !input.dry_run && !lock_rejected && !enforcing_clock && base_mode == OnFailMode::Quarantine;
    let add = if counts_toward_cap {
        input.failed as u64
    } else {
        0
    };
    let projected = VolumeWindow {
        minute: view.volume.minute.saturating_add(add),
        hour: view.volume.hour.saturating_add(add),
    };
    let crossed = scope_crossed(&view.config, projected);

    let mut mode = base_mode;
    let mut source = base_source;
    let mut rejected = lock_rejected;
    let mut report: Option<AutoTripReport> = None;
    let mut effect = ScopeEffect {
        scope,
        trip: false,
        reset,
        would_trip: false,
        until: None,
        window: None,
        observed: 0,
        threshold: 0,
    };

    let crossing_enforced = counts_toward_cap && view.config.enforce && crossed.is_some();

    if enforcing_clock || crossing_enforced {
        // The cap is higher precedence than the lock and the request.
        rejected = false;
        mode = OnFailMode::Drop;
        source = OnFailSource::AutoTrip;
        let until = if crossing_enforced {
            effect.trip = true;
            effect.until = cooldown_until(&view.config, input.now);
            if let Some((window, observed, threshold)) = crossed {
                effect.window = Some(window);
                effect.observed = observed;
                effect.threshold = threshold;
            }
            effect.until
        } else {
            view.trip.trip_until
        };
        report = Some(AutoTripReport {
            active: true,
            would_trip: false,
            enforcing: true,
            scope,
            until,
        });
    } else if rejected {
        return finish(input, base_mode, base_source, true, None, None, false);
    } else if base_mode == OnFailMode::Quarantine {
        if let Some((window, observed, threshold)) = crossed {
            effect.window = Some(window);
            effect.observed = observed;
            effect.threshold = threshold;
            effect.would_trip = true;
            report = Some(AutoTripReport {
                active: false,
                would_trip: true,
                enforcing: view.config.enforce,
                scope,
                until: None,
            });
        }
    }

    let persist = if input.dry_run {
        None
    } else if effect.trip || effect.reset || effect.would_trip {
        Some(effect)
    } else {
        None
    };

    let sample = mode == OnFailMode::Drop
        && (view.config.sample_drops || input.contract_guard.config.sample_drops);

    finish(input, mode, source, rejected, report, persist, sample)
}

fn finish(
    input: &ResolveInput,
    mode: OnFailMode,
    source: OnFailSource,
    lock_rejected: bool,
    auto_trip: Option<AutoTripReport>,
    effect: Option<ScopeEffect>,
    sample_drops: bool,
) -> OnFailDecision {
    let (quarantined, dropped) = write_counts(input.failed, mode, input.dry_run);
    OnFailDecision {
        mode,
        source,
        auto_trip,
        lock_rejected,
        effect,
        sample_drops,
        quarantined,
        dropped,
        quarantine_cap: None,
    }
}

/// Precedence above the request: an enforcing safety-cap trip, then a full
/// Free cap. A full cap switches a quarantine decision to the counted drop
/// path. Dry-run and an already-dropped batch are left alone. Marking rows
/// reviewed lowers `pending` immediately; there is no lower watermark.
pub fn apply_plan_cap(mut decision: OnFailDecision, counts: PlanCapCounts) -> OnFailDecision {
    decision.quarantine_cap = quarantine_cap_view(counts);
    if decision.lock_rejected {
        return decision;
    }
    if decision.source == OnFailSource::AutoTrip && decision.mode == OnFailMode::Drop {
        return decision;
    }
    if decision.mode != OnFailMode::Quarantine || decision.quarantined == 0 {
        return decision;
    }
    if cap_reached(counts) {
        let failed = decision.quarantined;
        decision.mode = OnFailMode::Drop;
        decision.source = OnFailSource::PlanCap;
        decision.quarantined = 0;
        decision.dropped = failed;
        decision.sample_drops = false;
    }
    decision
}

fn cap_reached(counts: PlanCapCounts) -> bool {
    scope_reached(counts.org_pending, counts.org_limit)
        || scope_reached(counts.contract_pending, counts.contract_limit)
}

fn scope_reached(pending: i64, limit: Option<i64>) -> bool {
    limit.is_some_and(|limit| limit > 0 && pending >= limit)
}

/// The tighter cap, when either limit is set.
pub fn quarantine_cap_view(counts: PlanCapCounts) -> Option<QuarantineCapView> {
    let org_limit = counts.org_limit.filter(|limit| *limit > 0);
    let contract_limit = counts.contract_limit.filter(|limit| *limit > 0);
    let (limit, pending, scope) = match (org_limit, contract_limit) {
        (None, None) => return None,
        (Some(limit), None) => (limit, counts.org_pending, "org"),
        (None, Some(limit)) => (limit, counts.contract_pending, "contract"),
        (Some(org_limit), Some(contract_limit)) => {
            let org_rank = state_rank(scope_state(
                counts.org_pending,
                org_limit,
                counts.near_percent,
            ));
            let contract_rank = state_rank(scope_state(
                counts.contract_pending,
                contract_limit,
                counts.near_percent,
            ));
            let contract_tighter = contract_rank > org_rank
                || (contract_rank == org_rank
                    && ratio(counts.contract_pending, contract_limit)
                        > ratio(counts.org_pending, org_limit));
            if contract_tighter {
                (contract_limit, counts.contract_pending, "contract")
            } else {
                (org_limit, counts.org_pending, "org")
            }
        }
    };
    Some(QuarantineCapView {
        limit,
        pending,
        state: scope_state(pending, limit, counts.near_percent),
        scope,
        org_limit: org_limit.unwrap_or(0),
        org_pending: counts.org_pending,
        contract_limit: contract_limit.unwrap_or(0),
        contract_pending: counts.contract_pending,
    })
}

fn scope_state(pending: i64, limit: i64, near_percent: i64) -> &'static str {
    if pending >= limit {
        "reached"
    } else if limit > 0 && pending.saturating_mul(100) >= limit.saturating_mul(near_percent.max(0))
    {
        "near"
    } else {
        "ok"
    }
}

fn state_rank(state: &str) -> u8 {
    match state {
        "reached" => 2,
        "near" => 1,
        _ => 0,
    }
}

fn ratio(pending: i64, limit: i64) -> i128 {
    if limit <= 0 {
        0
    } else {
        i128::from(pending) * 1_000_000 / i128::from(limit)
    }
}

fn base_mode(input: &ResolveInput) -> (OnFailMode, OnFailSource, bool) {
    if input.locked {
        if let Some(request) = input.request {
            if request != input.contract_mode {
                return (input.contract_mode, contract_source(input), true);
            }
        }
        return (input.contract_mode, contract_source(input), false);
    }
    if let Some(request) = input.request {
        return (request, OnFailSource::Request, false);
    }
    (input.contract_mode, contract_source(input), false)
}

fn contract_source(input: &ResolveInput) -> OnFailSource {
    if input.contract_explicit {
        OnFailSource::Contract
    } else {
        OnFailSource::Default
    }
}

fn winning_scope(input: &ResolveInput) -> (GuardScope, ScopeView) {
    if let Some(account) = input.account.clone() {
        if account.config.configured() {
            return (GuardScope::Account, account);
        }
    }
    (GuardScope::Contract, input.contract_guard.clone())
}

fn scope_crossed(
    config: &QuarantineGuardConfig,
    volume: VolumeWindow,
) -> Option<(&'static str, u64, u64)> {
    if let Some(max) = config.max_per_minute.filter(|n| *n > 0) {
        if volume.minute >= max {
            return Some(("minute", volume.minute, max));
        }
    }
    if let Some(max) = config.max_per_hour.filter(|n| *n > 0) {
        if volume.hour >= max {
            return Some(("hour", volume.hour, max));
        }
    }
    None
}

fn cooldown_until(config: &QuarantineGuardConfig, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    config
        .cooldown_minutes
        .filter(|minutes| *minutes > 0)
        .map(|minutes| now + Duration::minutes(i64::from(minutes)))
}

/// Quarantine-insert counter. Atomic rejection stores no quarantine rows,
/// so it must not advance the safety cap.
pub fn records_quarantine_volume(quarantined: usize, atomic_rejected: bool) -> bool {
    quarantined > 0 && !atomic_rejected
}

/// How many metadata samples still fit in this minute bucket.
pub fn sample_room(existing: i64) -> usize {
    let filled = usize::try_from(existing).unwrap_or(0);
    SAMPLE_CAP.saturating_sub(filled)
}

/// Stored rows for a batch. Dry-run writes nothing.
/// `failed == quarantined + dropped` except under dry-run, where both are 0.
pub fn write_counts(failed: usize, mode: OnFailMode, dry_run: bool) -> (usize, usize) {
    if dry_run || failed == 0 {
        return (0, 0);
    }
    match mode {
        OnFailMode::Drop => (0, failed),
        OnFailMode::Quarantine => (failed, 0),
    }
}

pub fn writes_audit(passed: bool, mode: OnFailMode, dry_run: bool) -> bool {
    !dry_run && (passed || mode == OnFailMode::Quarantine)
}

pub fn writes_quarantine(passed: bool, mode: OnFailMode, dry_run: bool) -> bool {
    !dry_run && !passed && mode == OnFailMode::Quarantine
}

pub fn writes_ledger(passed: bool, mode: OnFailMode, dry_run: bool) -> bool {
    !dry_run && !passed && mode == OnFailMode::Drop
}

/// Atomic rejection writes one summary audit row only when failures are quarantined.
pub fn writes_atomic_summary(mode: OnFailMode, dry_run: bool, failed: usize) -> bool {
    !dry_run && failed > 0 && mode == OnFailMode::Quarantine
}

pub fn per_record_disposition(passed: bool, mode: OnFailMode) -> Option<&'static str> {
    if passed {
        None
    } else if mode == OnFailMode::Drop {
        Some("dropped")
    } else {
        Some("quarantined")
    }
}

/// `kind` plus field path. Never a value, message, or record body.
pub fn reason_key(kind: &str, field: &str) -> String {
    let kind = kind.trim();
    let field = field.trim();
    let raw = if field.is_empty() {
        if kind.is_empty() {
            "unspecified".to_string()
        } else {
            kind.to_string()
        }
    } else if kind.is_empty() {
        field.to_string()
    } else {
        format!("{kind}:{field}")
    };
    const MAX: usize = 240;
    if raw.len() <= MAX {
        raw
    } else {
        raw.chars().take(MAX).collect()
    }
}

/// One reason per failed record: the first violation. Totals then match drops.
pub fn tally_reasons(records: &[Vec<(&str, &str)>]) -> Vec<(String, u64)> {
    let mut order: Vec<String> = Vec::new();
    let mut counts: Vec<u64> = Vec::new();
    for violations in records {
        let key = match violations.first() {
            Some((kind, field)) => reason_key(kind, field),
            None => reason_key("", ""),
        };
        if let Some(pos) = order.iter().position(|existing| existing == &key) {
            counts[pos] = counts[pos].saturating_add(1);
        } else {
            order.push(key);
            counts.push(1);
        }
    }
    order.into_iter().zip(counts).collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SampleFact {
    pub kind: String,
    pub field: String,
}

/// JSON stored for an opt-in sample. Keys are `kind` and `field` only.
pub fn sample_document(facts: &[SampleFact]) -> serde_json::Value {
    sanitize_sample(&serde_json::json!({
        "violations": facts
            .iter()
            .map(|fact| serde_json::json!({
                "kind": fact.kind,
                "field": fact.field,
            }))
            .collect::<Vec<_>>()
    }))
}

/// Rebuild a sample so only violation kind and field path are stored.
/// Record bodies, messages, and received values are dropped.
pub fn sanitize_sample(value: &serde_json::Value) -> serde_json::Value {
    let violations = value
        .get("violations")
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .map(|item| {
                    serde_json::json!({
                        "kind": clip_meta(item.get("kind").and_then(serde_json::Value::as_str).unwrap_or("")),
                        "field": clip_meta(item.get("field").and_then(serde_json::Value::as_str).unwrap_or("")),
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    serde_json::json!({ "violations": violations })
}

fn clip_meta(value: &str) -> String {
    value.trim().chars().take(240).collect()
}

pub fn minute_bucket(now: DateTime<Utc>) -> DateTime<Utc> {
    now.with_second(0)
        .and_then(|t| t.with_nanosecond(0))
        .unwrap_or(now)
}

pub fn hour_bucket(now: DateTime<Utc>) -> DateTime<Utc> {
    now.with_minute(0)
        .and_then(|t| t.with_second(0))
        .and_then(|t| t.with_nanosecond(0))
        .unwrap_or(now)
}

/// True when a minute bucket is older than the 30-day drop ledger retention.
pub fn drop_bucket_expired(bucket_start: DateTime<Utc>, now: DateTime<Utc>) -> bool {
    bucket_start < now - Duration::days(DROP_RETENTION_DAYS)
}

/// Parse `on_fail` from the query and the `X-On-Fail` header.
/// Empty values are ignored. Disagreement is an error.
pub fn parse_on_fail_args(
    query: Option<&str>,
    header: Option<&str>,
) -> Result<Option<OnFailMode>, String> {
    let query = blank_to_none(query);
    let header = blank_to_none(header);
    match (query, header) {
        (None, None) => Ok(None),
        (Some(value), None) | (None, Some(value)) => value
            .parse::<OnFailMode>()
            .map(Some)
            .map_err(|_| "on_fail must be quarantine or drop".to_string()),
        (Some(left), Some(right)) if left.eq_ignore_ascii_case(right) => left
            .parse::<OnFailMode>()
            .map(Some)
            .map_err(|_| "on_fail must be quarantine or drop".to_string()),
        (Some(_), Some(_)) => Err("on_fail query and X-On-Fail header disagree".into()),
    }
}

fn blank_to_none(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|s| !s.is_empty())
}

/// Top-level `on_fail:` in a contract document. Nested keys do not count.
pub fn yaml_sets_on_fail(yaml: &str) -> bool {
    yaml.lines().any(|line| {
        let trimmed = line.trim_end();
        !trimmed.starts_with(' ')
            && !trimmed.starts_with('\t')
            && trimmed.trim_start().starts_with("on_fail:")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::OnFailMode;

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-10-30T18:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn guard(enforce: bool, per_minute: u64) -> QuarantineGuardConfig {
        QuarantineGuardConfig {
            enforce,
            max_per_minute: Some(per_minute),
            max_per_hour: None,
            cooldown_minutes: Some(60),
            sample_drops: false,
        }
    }

    fn input() -> ResolveInput {
        ResolveInput {
            request: None,
            contract_mode: OnFailMode::Quarantine,
            contract_explicit: false,
            locked: false,
            account: None,
            contract_guard: ScopeView::default(),
            now: now(),
            failed: 4,
            dry_run: false,
        }
    }

    fn over_contract(enforce: bool) -> ScopeView {
        ScopeView {
            config: guard(enforce, 10),
            volume: VolumeWindow { minute: 9, hour: 9 },
            trip: StoredTrip::default(),
        }
    }

    #[test]
    fn default_is_quarantine() {
        let decision = resolve_on_fail(&input());
        assert_eq!(decision.mode, OnFailMode::Quarantine);
        assert_eq!(decision.source, OnFailSource::Default);
        assert!(decision.auto_trip.is_none());
        assert_eq!(decision.quarantined, 4);
        assert_eq!(decision.dropped, 0);
        assert_eq!(decision.quarantined + decision.dropped, 4);
    }

    #[test]
    fn request_overrides_contract_until_locked() {
        let mut open = input();
        open.contract_mode = OnFailMode::Quarantine;
        open.contract_explicit = true;
        open.request = Some(OnFailMode::Drop);
        let decision = resolve_on_fail(&open);
        assert_eq!(decision.mode, OnFailMode::Drop);
        assert_eq!(decision.source, OnFailSource::Request);
        assert!(!decision.lock_rejected);
        assert_eq!(decision.dropped, 4);
        assert_eq!(decision.quarantined, 0);

        let mut locked = open.clone();
        locked.locked = true;
        let rejected = resolve_on_fail(&locked);
        assert!(rejected.lock_rejected);
        assert_eq!(rejected.mode, OnFailMode::Quarantine);
        assert_eq!(rejected.source, OnFailSource::Contract);
        assert!(rejected.effect.is_none());
    }

    #[test]
    fn lock_allows_matching_request() {
        let mut locked = input();
        locked.contract_mode = OnFailMode::Drop;
        locked.contract_explicit = true;
        locked.locked = true;
        locked.request = Some(OnFailMode::Drop);
        let decision = resolve_on_fail(&locked);
        assert!(!decision.lock_rejected);
        assert_eq!(decision.mode, OnFailMode::Drop);
        assert_eq!(decision.source, OnFailSource::Contract);
    }

    #[test]
    fn dry_run_writes_zero_counts() {
        let mut dry = input();
        dry.dry_run = true;
        dry.request = Some(OnFailMode::Drop);
        let decision = resolve_on_fail(&dry);
        assert_eq!(decision.mode, OnFailMode::Drop);
        assert_eq!((decision.quarantined, decision.dropped), (0, 0));
        assert!(decision.effect.is_none());
    }

    #[test]
    fn drop_skips_audit_and_quarantine() {
        assert!(!writes_audit(false, OnFailMode::Drop, false));
        assert!(!writes_quarantine(false, OnFailMode::Drop, false));
        assert!(writes_ledger(false, OnFailMode::Drop, false));
        assert!(!writes_atomic_summary(OnFailMode::Drop, false, 3));
        assert!(writes_audit(true, OnFailMode::Drop, false));
        assert!(!writes_audit(false, OnFailMode::Drop, true));
        assert!(!writes_ledger(false, OnFailMode::Drop, true));
    }

    #[test]
    fn quarantine_still_writes_both_rows() {
        assert!(writes_audit(false, OnFailMode::Quarantine, false));
        assert!(writes_quarantine(false, OnFailMode::Quarantine, false));
        assert!(!writes_ledger(false, OnFailMode::Quarantine, false));
        assert!(writes_atomic_summary(OnFailMode::Quarantine, false, 2));
    }

    #[test]
    fn watch_only_cap_does_not_switch_mode() {
        let mut watch = input();
        watch.contract_guard = over_contract(false);
        let decision = resolve_on_fail(&watch);
        assert_eq!(decision.mode, OnFailMode::Quarantine);
        assert_eq!(decision.source, OnFailSource::Default);
        assert_eq!(decision.quarantined, 4);
        let report = decision.auto_trip.expect("would-trip");
        assert!(!report.active);
        assert!(report.would_trip);
        assert!(!report.enforcing);
        assert_eq!(report.scope, GuardScope::Contract);
        let effect = decision.effect.expect("record would-trip");
        assert!(effect.would_trip);
        assert!(!effect.trip);
    }

    #[test]
    fn enforcing_cap_drops_and_blocks_request_quarantine() {
        let mut tripped = input();
        tripped.request = Some(OnFailMode::Quarantine);
        tripped.contract_guard = over_contract(true);
        let decision = resolve_on_fail(&tripped);
        assert!(!decision.lock_rejected);
        assert_eq!(decision.mode, OnFailMode::Drop);
        assert_eq!(decision.source, OnFailSource::AutoTrip);
        assert_eq!(decision.dropped, 4);
        let report = decision.auto_trip.expect("active trip");
        assert!(report.active);
        assert!(report.enforcing);
        assert_eq!(report.until, Some(now() + Duration::minutes(60)));
        let effect = decision.effect.expect("persist trip");
        assert!(effect.trip);
        assert_eq!(effect.scope, GuardScope::Contract);
    }

    #[test]
    fn active_trip_survives_a_passing_projection() {
        let mut active = input();
        active.failed = 0;
        active.request = Some(OnFailMode::Quarantine);
        active.contract_guard = ScopeView {
            config: guard(true, 10),
            volume: VolumeWindow::default(),
            trip: StoredTrip {
                tripped_at: Some(now() - Duration::minutes(5)),
                trip_until: Some(now() + Duration::minutes(55)),
            },
        };
        let decision = resolve_on_fail(&active);
        assert_eq!(decision.mode, OnFailMode::Drop);
        assert_eq!(decision.source, OnFailSource::AutoTrip);
        assert_eq!((decision.quarantined, decision.dropped), (0, 0));
        assert!(decision.auto_trip.unwrap().active);
        assert!(decision.effect.is_none());
    }

    #[test]
    fn request_cannot_force_quarantine_during_trip_even_on_dry_run_report() {
        let mut dry = input();
        dry.dry_run = true;
        dry.request = Some(OnFailMode::Quarantine);
        dry.contract_guard = ScopeView {
            config: guard(true, 10),
            volume: VolumeWindow::default(),
            trip: StoredTrip {
                tripped_at: Some(now() - Duration::minutes(1)),
                trip_until: Some(now() + Duration::minutes(10)),
            },
        };
        let decision = resolve_on_fail(&dry);
        assert_eq!(decision.mode, OnFailMode::Drop);
        assert_eq!(decision.source, OnFailSource::AutoTrip);
        assert!(decision.effect.is_none());
        assert_eq!((decision.quarantined, decision.dropped), (0, 0));
    }

    #[test]
    fn cooldown_resets_when_volume_is_back_under() {
        let mut expired = input();
        expired.failed = 1;
        expired.contract_guard = ScopeView {
            config: guard(true, 10),
            volume: VolumeWindow { minute: 1, hour: 1 },
            trip: StoredTrip {
                tripped_at: Some(now() - Duration::minutes(90)),
                trip_until: Some(now() - Duration::minutes(1)),
            },
        };
        let decision = resolve_on_fail(&expired);
        assert_eq!(decision.mode, OnFailMode::Quarantine);
        assert_eq!(decision.source, OnFailSource::Default);
        assert!(decision.auto_trip.is_none());
        let effect = decision.effect.expect("clear expired trip");
        assert!(effect.reset);
        assert!(!effect.trip);
    }

    #[test]
    fn cooldown_then_still_over_trips_again() {
        let mut expired = input();
        expired.contract_guard = ScopeView {
            config: guard(true, 10),
            volume: over_contract(true).volume,
            trip: StoredTrip {
                tripped_at: Some(now() - Duration::minutes(90)),
                trip_until: Some(now()),
            },
        };
        let decision = resolve_on_fail(&expired);
        assert_eq!(decision.source, OnFailSource::AutoTrip);
        let effect = decision.effect.expect("re-trip");
        assert!(effect.reset);
        assert!(effect.trip);
    }

    #[test]
    fn account_cap_overrides_contract_cap() {
        let mut both = input();
        both.contract_guard = over_contract(true);
        both.account = Some(ScopeView {
            config: guard(true, 1000),
            volume: VolumeWindow { minute: 3, hour: 3 },
            trip: StoredTrip::default(),
        });
        let decision = resolve_on_fail(&both);
        assert_eq!(decision.mode, OnFailMode::Quarantine);
        assert!(decision.auto_trip.is_none());
        assert!(decision.effect.is_none());

        both.account.as_mut().unwrap().volume.minute = 1000;
        let tripped = resolve_on_fail(&both);
        assert_eq!(tripped.source, OnFailSource::AutoTrip);
        assert_eq!(tripped.auto_trip.unwrap().scope, GuardScope::Account);
        assert_eq!(tripped.effect.unwrap().scope, GuardScope::Account);
    }

    #[test]
    fn account_watch_only_does_not_honor_contract_trip() {
        let mut both = input();
        both.contract_guard = ScopeView {
            config: guard(true, 10),
            volume: VolumeWindow::default(),
            trip: StoredTrip {
                tripped_at: Some(now() - Duration::minutes(1)),
                trip_until: Some(now() + Duration::minutes(30)),
            },
        };
        both.account = Some(ScopeView {
            config: guard(false, 5),
            volume: VolumeWindow { minute: 5, hour: 5 },
            trip: StoredTrip::default(),
        });
        let decision = resolve_on_fail(&both);
        assert_eq!(decision.mode, OnFailMode::Quarantine);
        let report = decision.auto_trip.expect("account would-trip");
        assert_eq!(report.scope, GuardScope::Account);
        assert!(report.would_trip);
        assert!(!report.active);
    }

    #[test]
    fn contract_drop_does_not_trip_the_cap() {
        let mut dropping = input();
        dropping.contract_mode = OnFailMode::Drop;
        dropping.contract_explicit = true;
        dropping.contract_guard = over_contract(true);
        let decision = resolve_on_fail(&dropping);
        assert_eq!(decision.source, OnFailSource::Contract);
        assert!(decision.auto_trip.is_none());
        assert!(decision.effect.is_none());
    }

    #[test]
    fn sample_flag_follows_drop_and_opt_in() {
        let mut opted = input();
        opted.request = Some(OnFailMode::Drop);
        opted.contract_guard.config.sample_drops = true;
        let decision = resolve_on_fail(&opted);
        assert!(decision.sample_drops);

        let mut quarantine = input();
        quarantine.contract_guard.config.sample_drops = true;
        assert!(!resolve_on_fail(&quarantine).sample_drops);
    }

    #[test]
    fn counts_match_failed_except_dry_run() {
        for failed in [0_usize, 1, 7] {
            for mode in [OnFailMode::Quarantine, OnFailMode::Drop] {
                let (q, d) = write_counts(failed, mode, false);
                assert_eq!(q + d, failed);
                let (q, d) = write_counts(failed, mode, true);
                assert_eq!((q, d), (0, 0));
            }
        }
    }

    #[test]
    fn reason_keys_and_samples_hold_no_values() {
        assert_eq!(
            reason_key("missing_required_field", "user_id"),
            "missing_required_field:user_id"
        );
        let tallies = tally_reasons(&[
            vec![
                ("missing_required_field", "user_id"),
                ("type_mismatch", "amount"),
            ],
            vec![("missing_required_field", "user_id")],
            vec![("enum_violation", "status")],
        ]);
        assert_eq!(
            tallies,
            vec![
                ("missing_required_field:user_id".into(), 2),
                ("enum_violation:status".into(), 1),
            ]
        );
        let doc = sample_document(&[SampleFact {
            kind: "type_mismatch".into(),
            field: "amount".into(),
        }]);
        let text = doc.to_string();
        assert!(!text.contains("received"));
        assert!(!text.contains("message"));
        assert_eq!(doc["violations"][0]["kind"], "type_mismatch");
        assert_eq!(doc["violations"][0]["field"], "amount");
    }

    #[test]
    fn retention_is_thirty_days() {
        let t = now();
        assert!(!drop_bucket_expired(t - Duration::days(29), t));
        assert!(!drop_bucket_expired(t - Duration::days(30), t));
        assert!(drop_bucket_expired(
            t - Duration::days(30) - Duration::seconds(1),
            t
        ));
        assert_eq!(DROP_RETENTION_DAYS, 30);
        assert_eq!(SAMPLE_CAP, 5);
    }

    #[test]
    fn on_fail_parse_and_yaml_marker() {
        assert_eq!(
            parse_on_fail_args(Some("drop"), None).unwrap(),
            Some(OnFailMode::Drop)
        );
        assert!(parse_on_fail_args(Some("nope"), None).is_err());
        assert!(parse_on_fail_args(Some("drop"), Some("quarantine")).is_err());
        assert_eq!(
            parse_on_fail_args(Some("drop"), Some("DROP")).unwrap(),
            Some(OnFailMode::Drop)
        );
        assert!(parse_on_fail_args(None, Some("  ")).unwrap().is_none());
        assert!(yaml_sets_on_fail("name: demo\non_fail: drop\n"));
        assert!(!yaml_sets_on_fail("name: demo\n  on_fail: drop\n"));
    }

    #[test]
    fn unset_guard_has_no_threshold() {
        let config = QuarantineGuardConfig::default();
        assert!(!config.configured());
        assert!(!config.enforce);
        assert!(config.validate().is_ok());
        assert!(QuarantineGuardConfig {
            max_per_minute: Some(0),
            ..config
        }
        .validate()
        .is_err());
    }

    #[test]
    fn minute_bucket_truncates() {
        let bucket = minute_bucket(now() + Duration::seconds(42));
        assert_eq!(bucket, now());
        assert_eq!(hour_bucket(now() + Duration::minutes(17)), now());
    }

    fn counts(org_pending: i64, contract_pending: i64) -> PlanCapCounts {
        PlanCapCounts {
            org_pending,
            org_limit: Some(50_000),
            contract_pending,
            contract_limit: Some(10_000),
            near_percent: 80,
        }
    }

    #[test]
    fn plan_cap_table_and_precedence() {
        let open = quarantine_cap_view(counts(12_480, 100)).expect("view");
        assert_eq!(open.state, "ok");
        assert_eq!(open.scope, "org");
        assert_eq!(open.limit, 50_000);

        let near = quarantine_cap_view(counts(40_000, 100)).expect("near");
        assert_eq!(near.state, "near");
        assert_eq!(near.pending, 40_000);

        let contract_full = quarantine_cap_view(counts(100, 10_000)).expect("contract");
        assert_eq!(contract_full.state, "reached");
        assert_eq!(contract_full.scope, "contract");
        assert_eq!(contract_full.limit, 10_000);

        // One under the cap is room again. Reviewed rows are not in `pending`.
        let resumed = quarantine_cap_view(counts(49_999, 9_999)).expect("resumed");
        assert_eq!(resumed.state, "near");
        assert!(!scope_reached(49_999, Some(50_000)));

        let base = input();
        let forced = apply_plan_cap(resolve_on_fail(&base), counts(50_000, 0));
        assert_eq!(forced.mode, OnFailMode::Drop);
        assert_eq!(forced.source, OnFailSource::PlanCap);
        assert_eq!(forced.dropped, 4);
        assert_eq!(forced.quarantined, 0);
        assert!(!forced.sample_drops);
        assert_eq!(forced.quarantine_cap.unwrap().state, "reached");

        let mut dry = input();
        dry.dry_run = true;
        let dry_decision = apply_plan_cap(resolve_on_fail(&dry), counts(50_000, 0));
        assert_eq!(dry_decision.mode, OnFailMode::Quarantine);
        assert_eq!(dry_decision.dropped, 0);
        assert_eq!(dry_decision.source, OnFailSource::Default);

        let mut request_drop = input();
        request_drop.request = Some(OnFailMode::Drop);
        let dropped = apply_plan_cap(resolve_on_fail(&request_drop), counts(50_000, 0));
        assert_eq!(dropped.source, OnFailSource::Request);
        assert_eq!(dropped.mode, OnFailMode::Drop);

        let mut tripped = input();
        tripped.contract_guard.config.enforce = true;
        tripped.contract_guard.config.max_per_minute = Some(1);
        tripped.contract_guard.volume.minute = 5;
        let safety = apply_plan_cap(resolve_on_fail(&tripped), counts(50_000, 0));
        assert_eq!(safety.source, OnFailSource::AutoTrip);
        assert_eq!(safety.mode, OnFailMode::Drop);

        let under = apply_plan_cap(resolve_on_fail(&base), counts(10, 10));
        assert_eq!(under.source, OnFailSource::Default);
        assert_eq!(under.mode, OnFailMode::Quarantine);
        assert_eq!(under.quarantined, 4);

        assert!(quarantine_cap_view(PlanCapCounts {
            org_pending: 1,
            org_limit: None,
            contract_pending: 1,
            contract_limit: None,
            near_percent: 80,
        })
        .is_none());
    }

    #[test]
    fn request_cannot_force_quarantine_past_a_full_cap() {
        let mut force = input();
        force.contract_mode = OnFailMode::Drop;
        force.contract_explicit = true;
        force.request = Some(OnFailMode::Quarantine);
        let forced = apply_plan_cap(resolve_on_fail(&force), counts(50_000, 0));
        assert_eq!(forced.mode, OnFailMode::Drop);
        assert_eq!(forced.source, OnFailSource::PlanCap);
        assert_eq!(forced.quarantined, 0);
        assert_eq!(forced.dropped, 4);
        assert!(!forced.sample_drops);

        // The contract cap is full even though the shared org cap is not.
        let mut locked = input();
        locked.locked = true;
        locked.contract_explicit = true;
        locked.request = Some(OnFailMode::Quarantine);
        let at_contract_cap = apply_plan_cap(resolve_on_fail(&locked), counts(10_000, 10_000));
        assert!(!at_contract_cap.lock_rejected);
        assert_eq!(at_contract_cap.source, OnFailSource::PlanCap);
        assert_eq!(at_contract_cap.mode, OnFailMode::Drop);

        let under = apply_plan_cap(resolve_on_fail(&force), counts(49_999, 9_999));
        assert_eq!(under.source, OnFailSource::Request);
        assert_eq!(under.mode, OnFailMode::Quarantine);
        assert_eq!(under.quarantined, 4);
    }
}

/// Independent checks for precedence, storage, the lock, the watch-only cap,
/// counters, retention, and metadata-only samples.
#[cfg(test)]
mod review_invariants {
    use super::*;
    use crate::contract::{Contract, OnFailMode};

    const SECRET: &str = "SECRET-ssn-999-00-1234";

    fn at() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-10-30T18:00:30Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn minute_guard(enforce: bool, per_minute: u64) -> QuarantineGuardConfig {
        QuarantineGuardConfig {
            enforce,
            max_per_minute: Some(per_minute),
            max_per_hour: None,
            cooldown_minutes: Some(60),
            sample_drops: false,
        }
    }

    fn base() -> ResolveInput {
        ResolveInput {
            request: None,
            contract_mode: OnFailMode::Quarantine,
            contract_explicit: true,
            locked: false,
            account: None,
            contract_guard: ScopeView::default(),
            now: at(),
            failed: 3,
            dry_run: false,
        }
    }

    fn hot_contract(enforce: bool) -> ScopeView {
        ScopeView {
            config: minute_guard(enforce, 10),
            volume: VolumeWindow { minute: 8, hour: 8 },
            trip: StoredTrip::default(),
        }
    }

    fn active_trip(enforce: bool) -> ScopeView {
        ScopeView {
            config: minute_guard(enforce, 10),
            volume: VolumeWindow::default(),
            trip: StoredTrip {
                tripped_at: Some(at() - Duration::minutes(2)),
                trip_until: Some(at() + Duration::minutes(40)),
            },
        }
    }

    #[test]
    fn precedence_matrix_dry_run_trip_lock_request_contract_default() {
        // dry_run writes nothing but still reports an enforcing trip.
        let mut dry = base();
        dry.dry_run = true;
        dry.contract_mode = OnFailMode::Drop;
        dry.request = Some(OnFailMode::Quarantine);
        dry.locked = true;
        dry.contract_guard = active_trip(true);
        let decision = resolve_on_fail(&dry);
        assert_eq!(decision.mode, OnFailMode::Drop);
        assert_eq!(decision.source, OnFailSource::AutoTrip);
        assert!(!decision.lock_rejected);
        assert_eq!((decision.quarantined, decision.dropped), (0, 0));
        assert!(decision.effect.is_none());
        assert!(!writes_audit(false, decision.mode, true));
        assert!(!writes_quarantine(false, decision.mode, true));
        assert!(!writes_ledger(false, decision.mode, true));

        // Unlocked request beats the contract.
        let mut request = base();
        request.contract_mode = OnFailMode::Quarantine;
        request.request = Some(OnFailMode::Drop);
        let decision = resolve_on_fail(&request);
        assert_eq!(decision.mode, OnFailMode::Drop);
        assert_eq!(decision.source, OnFailSource::Request);
        assert_eq!((decision.quarantined, decision.dropped), (0, 3));

        // Contract beats the default when the request omits on_fail.
        let mut contract = base();
        contract.contract_mode = OnFailMode::Drop;
        let decision = resolve_on_fail(&contract);
        assert_eq!(decision.source, OnFailSource::Contract);
        assert_eq!(decision.dropped, 3);

        // Omitted contract value is quarantine.
        let mut unset = base();
        unset.contract_explicit = false;
        unset.contract_mode = OnFailMode::default();
        let decision = resolve_on_fail(&unset);
        assert_eq!(decision.mode, OnFailMode::Quarantine);
        assert_eq!(decision.source, OnFailSource::Default);
        assert_eq!(decision.quarantined, 3);
    }

    #[test]
    fn enforcing_trip_beats_the_lock_and_blocks_forced_quarantine() {
        // Locked to quarantine, request asks for drop, cap already enforcing.
        let mut locked_drop_request = base();
        locked_drop_request.locked = true;
        locked_drop_request.request = Some(OnFailMode::Drop);
        locked_drop_request.contract_guard = active_trip(true);
        let decision = resolve_on_fail(&locked_drop_request);
        assert!(!decision.lock_rejected);
        assert_eq!(decision.mode, OnFailMode::Drop);
        assert_eq!(decision.source, OnFailSource::AutoTrip);
        assert_eq!(decision.dropped, 3);
        assert_eq!(decision.quarantined, 0);

        // Locked to drop, request tries to force quarantine during the trip.
        let mut force = base();
        force.contract_mode = OnFailMode::Drop;
        force.locked = true;
        force.request = Some(OnFailMode::Quarantine);
        force.contract_guard = active_trip(true);
        let decision = resolve_on_fail(&force);
        assert!(!decision.lock_rejected);
        assert_eq!(decision.mode, OnFailMode::Drop);
        assert_eq!(decision.source, OnFailSource::AutoTrip);

        // Same conflict with no trip is a 400, and it does not arm the cap.
        let mut rejected = base();
        rejected.locked = true;
        rejected.request = Some(OnFailMode::Drop);
        rejected.contract_guard = hot_contract(true);
        let decision = resolve_on_fail(&rejected);
        assert!(decision.lock_rejected);
        assert_eq!(decision.mode, OnFailMode::Quarantine);
        assert!(decision.effect.is_none());
        assert!(decision.auto_trip.is_none());

        // A matching locked request is accepted, then the crossing cap drops it.
        let mut matched = base();
        matched.locked = true;
        matched.request = Some(OnFailMode::Quarantine);
        matched.contract_guard = hot_contract(true);
        let decision = resolve_on_fail(&matched);
        assert!(!decision.lock_rejected);
        assert_eq!(decision.source, OnFailSource::AutoTrip);
        assert_eq!(decision.dropped, 3);
        assert!(decision.effect.unwrap().trip);
    }

    #[test]
    fn drop_writes_no_audit_or_quarantine_rows() {
        assert!(!writes_audit(false, OnFailMode::Drop, false));
        assert!(!writes_quarantine(false, OnFailMode::Drop, false));
        assert!(writes_ledger(false, OnFailMode::Drop, false));
        assert!(!writes_atomic_summary(OnFailMode::Drop, false, 5));
        assert!(writes_audit(true, OnFailMode::Drop, false));
        assert!(!writes_audit(true, OnFailMode::Drop, true));
        assert!(!records_quarantine_volume(5, true));
        assert!(records_quarantine_volume(5, false));
        assert!(!records_quarantine_volume(0, false));

        let mut dropping = base();
        dropping.request = Some(OnFailMode::Drop);
        let decision = resolve_on_fail(&dropping);
        assert_eq!((decision.quarantined, decision.dropped), (0, 3));
        assert_eq!(
            per_record_disposition(false, decision.mode),
            Some("dropped")
        );
        assert!(per_record_disposition(true, decision.mode).is_none());
    }

    #[test]
    fn watch_only_is_the_default_and_does_not_drop() {
        let parsed: QuarantineGuardConfig = serde_json::from_str("{}").unwrap();
        assert!(!parsed.enforce);
        assert!(!parsed.configured());
        assert!(!parsed.sample_drops);
        assert_eq!(OnFailMode::default(), OnFailMode::Quarantine);

        let mut watch = base();
        watch.contract_guard = hot_contract(false);
        let decision = resolve_on_fail(&watch);
        assert_eq!(decision.mode, OnFailMode::Quarantine);
        assert_eq!(decision.source, OnFailSource::Contract);
        assert_eq!(decision.quarantined, 3);
        assert_eq!(decision.dropped, 0);
        let report = decision.auto_trip.expect("would-trip");
        assert!(report.would_trip);
        assert!(!report.active);
        assert!(!report.enforcing);
        let effect = decision.effect.expect("record would-trip");
        assert!(effect.would_trip);
        assert!(!effect.trip);

        // A threshold with enforce left unset stays watch-only.
        let yaml = "version: \"1.0\"\nname: demo\nontology:\n  entities: []\nquarantine_guard:\n  max_per_minute: 10\n";
        let contract: Contract = serde_yaml::from_str(yaml).unwrap();
        assert!(!contract.quarantine_guard.enforce);
        assert!(contract.quarantine_guard.configured());
        assert_eq!(contract.on_fail, OnFailMode::Quarantine);
        assert!(!contract.on_fail_locked);
    }

    #[test]
    fn account_cap_replaces_contract_cap_until_reset() {
        let mut both = base();
        both.request = Some(OnFailMode::Quarantine);
        both.contract_guard = active_trip(true);
        both.account = Some(ScopeView {
            config: QuarantineGuardConfig {
                enforce: true,
                max_per_minute: None,
                max_per_hour: Some(100),
                cooldown_minutes: Some(15),
                sample_drops: false,
            },
            volume: VolumeWindow { minute: 4, hour: 4 },
            trip: StoredTrip::default(),
        });
        let decision = resolve_on_fail(&both);
        assert_eq!(decision.mode, OnFailMode::Quarantine);
        assert_eq!(decision.source, OnFailSource::Request);
        assert!(decision.auto_trip.is_none());

        both.account.as_mut().unwrap().volume.hour = 100;
        let tripped = resolve_on_fail(&both);
        assert_eq!(tripped.source, OnFailSource::AutoTrip);
        assert_eq!(tripped.auto_trip.unwrap().scope, GuardScope::Account);
        assert_eq!(tripped.effect.unwrap().scope, GuardScope::Account);
        assert_eq!(tripped.dropped, 3);

        // Cooldown elapsed and the hour is back under: the trip clears.
        both.account.as_mut().unwrap().volume.hour = 1;
        both.account.as_mut().unwrap().trip = StoredTrip {
            tripped_at: Some(at() - Duration::minutes(20)),
            trip_until: Some(at() - Duration::seconds(1)),
        };
        let reset = resolve_on_fail(&both);
        assert_eq!(reset.mode, OnFailMode::Quarantine);
        assert!(reset.effect.unwrap().reset);
        assert!(reset.auto_trip.is_none());
    }

    #[test]
    fn counters_are_per_record_and_samples_stop_at_five() {
        let records = [
            vec![("type_mismatch", "amount"), ("enum_violation", "status")],
            vec![("type_mismatch", "amount")],
            vec![("missing_required_field", "user_id")],
        ];
        let tallies = tally_reasons(&records);
        let total: u64 = tallies.iter().map(|(_, n)| *n).sum();
        assert_eq!(total, records.len() as u64);
        assert_eq!(tallies[0], ("type_mismatch:amount".into(), 2));

        assert_eq!(sample_room(0), SAMPLE_CAP);
        assert_eq!(sample_room(4), 1);
        assert_eq!(sample_room(SAMPLE_CAP as i64), 0);
        assert_eq!(sample_room(9), 0);
        assert_eq!(sample_room(-3), SAMPLE_CAP);

        let bucket = minute_bucket(at());
        assert_eq!(bucket.timestamp() % 60, 0);
        let later = bucket + Duration::seconds(59);
        assert_eq!(minute_bucket(later), bucket);
        assert_ne!(minute_bucket(bucket + Duration::seconds(60)), bucket);
    }

    #[test]
    fn retention_keeps_thirty_days_and_migration_matches() {
        let t = at();
        assert!(!drop_bucket_expired(t - Duration::days(30), t));
        assert!(drop_bucket_expired(
            t - Duration::days(30) - Duration::seconds(1),
            t
        ));

        let sql = include_str!("../supabase/migrations/045_drop_mode.sql");
        assert!(sql.contains("add column if not exists on_fail text not null default 'quarantine'"));
        assert!(
            sql.contains("add column if not exists on_fail_locked boolean not null default false")
        );
        assert!(sql.contains("add column if not exists fail_policy jsonb"));
        assert!(sql.contains("add column if not exists quarantine_guard jsonb"));
        assert!(sql.contains("constraint dropped_event_counts_bucket"));
        assert!(sql.contains("constraint quarantine_guard_state_scope"));
        assert!(sql.contains("sample_idx >= 0 and sample_idx < 5"));
        assert!(sql.contains("bucket_start < now() - interval '30 days'"));
        assert!(sql.contains("delete from public.audit_log"));
        assert!(sql.contains("delete from public.quarantine_events"));
        assert!(!sql.to_ascii_lowercase().contains("drop table"));
        assert!(!sql.contains("raw_event"));
        assert!(!sql.contains("payload"));
        assert!(sql.contains("Rollback"));
        assert_eq!(sql.matches("enable row level security").count(), 5);
    }

    #[test]
    fn samples_and_reason_keys_drop_record_contents() {
        let dirty = serde_json::json!({
            "violations": [{
                "kind": "type_mismatch",
                "field": "amount",
                "message": format!("got {SECRET}"),
                "received": SECRET,
                "expected": SECRET,
                "suggestion": SECRET,
                "payload": {"ssn": SECRET},
                "raw_event": {"amount": SECRET}
            }],
            "raw_event": SECRET,
            "payload": SECRET
        });
        let clean = sanitize_sample(&dirty);
        let text = clean.to_string();
        assert!(!text.contains(SECRET));
        assert!(!text.contains("message"));
        assert!(!text.contains("received"));
        assert!(!text.contains("payload"));
        assert!(!text.contains("raw_event"));
        assert_eq!(clean["violations"][0]["kind"], "type_mismatch");
        assert_eq!(clean["violations"][0]["field"], "amount");
        assert_eq!(clean["violations"][0].as_object().unwrap().len(), 2);

        let doc = sample_document(&[SampleFact {
            kind: "enum_violation".into(),
            field: "status".into(),
        }]);
        assert_eq!(sanitize_sample(&doc), doc);

        let key = reason_key("type_mismatch", "amount");
        assert_eq!(key, "type_mismatch:amount");
        assert!(!key.contains(SECRET));
        assert!(reason_key(&format!("type_mismatch {SECRET}"), "amount").len() <= 240);
    }

    #[test]
    fn contract_yaml_round_trip_rejects_unknown_on_fail() {
        let yaml = "version: \"1.0\"\nname: demo\nontology:\n  entities: []\non_fail: drop\non_fail_locked: true\n";
        let contract: Contract = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(contract.on_fail, OnFailMode::Drop);
        assert!(contract.on_fail_locked);
        assert!(yaml_sets_on_fail(yaml));
        let again = serde_yaml::to_string(&contract).unwrap();
        let parsed: Contract = serde_yaml::from_str(&again).unwrap();
        assert_eq!(parsed.on_fail, OnFailMode::Drop);
        assert!(parsed.on_fail_locked);

        let invalid = "version: \"1.0\"\nname: demo\nontology:\n  entities: []\non_fail: discard\n";
        assert!(serde_yaml::from_str::<Contract>(invalid).is_err());
        assert!(parse_on_fail_args(Some("discard"), None).is_err());
    }
}
