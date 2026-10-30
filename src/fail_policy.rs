//! Live on-fail resolution for ingest and egress.
//!
//! Quarantine is the default. Drop writes neither a quarantine row nor an
//! audit row for the failure. The safety cap is watch-only until `enforce`
//! is set. An account cap with a threshold replaces the contract cap.

use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::Json;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::contract::{
    Contract, FailPolicyOverride, GuardScope, OnFailMode, QuarantineGuardConfig,
};
use crate::drop_mode::{
    self, minute_bucket, AutoTripReport, OnFailDecision, ResolveInput, SampleFact, ScopeView,
    StoredTrip, VolumeWindow,
};
use crate::error::{AppError, AppResult};
use crate::storage::{self, TripClock};
use crate::{require_owner, AppState, OrgId};

#[derive(Debug, Clone)]
pub struct PolicyRequest {
    pub org_id: Option<Uuid>,
    pub contract_id: Uuid,
    pub contract: Contract,
    pub yaml: String,
    pub fail_policy: Option<serde_json::Value>,
    pub request_on_fail: Option<OnFailMode>,
    pub dry_run: bool,
    pub failed: usize,
}

pub fn on_fail_from_request(
    query: Option<&str>,
    headers: &HeaderMap,
) -> AppResult<Option<OnFailMode>> {
    let header = headers
        .get("x-on-fail")
        .and_then(|value| value.to_str().ok());
    drop_mode::parse_on_fail_args(query, header).map_err(AppError::BadRequest)
}

pub async fn decide(state: &AppState, req: &PolicyRequest) -> AppResult<OnFailDecision> {
    let now = Utc::now();
    let (mode, explicit, locked, guard) = effective_contract(req);
    let account_config = match req.org_id {
        Some(org_id) => storage::org_quarantine_guard(&state.db, org_id).await?,
        None => None,
    };
    let minute = minute_bucket(now);
    let hour = drop_mode::hour_bucket(now);
    let volume =
        storage::volume_counts(&state.db, req.org_id, req.contract_id, minute, hour).await?;
    let (account_clock, contract_clock) = match req.org_id {
        Some(org_id) => storage::trip_clocks(&state.db, org_id, req.contract_id).await?,
        None => (None, None),
    };

    let decision = drop_mode::resolve_on_fail(&ResolveInput {
        request: req.request_on_fail,
        contract_mode: mode,
        contract_explicit: explicit,
        locked,
        account: account_config.map(|config| ScopeView {
            config,
            volume: VolumeWindow {
                minute: volume.account_minute.max(0) as u64,
                hour: volume.account_hour.max(0) as u64,
            },
            trip: clock_to_trip(account_clock),
        }),
        contract_guard: ScopeView {
            config: guard,
            volume: VolumeWindow {
                minute: volume.contract_minute.max(0) as u64,
                hour: volume.contract_hour.max(0) as u64,
            },
            trip: clock_to_trip(contract_clock),
        },
        now,
        failed: req.failed,
        dry_run: req.dry_run,
    });

    if decision.lock_rejected {
        return Ok(decision);
    }
    if let Some(effect) = &decision.effect {
        if let Some(org_id) = req.org_id {
            persist_effect(&state.db, org_id, req.contract_id, effect).await?;
        }
    }
    record_trip_metrics(req.contract_id, &decision);
    Ok(decision)
}

fn effective_contract(req: &PolicyRequest) -> (OnFailMode, bool, bool, QuarantineGuardConfig) {
    if let Some(value) = req.fail_policy.as_ref() {
        match serde_json::from_value::<FailPolicyOverride>(value.clone()) {
            Ok(over) => {
                return (
                    over.on_fail,
                    true,
                    over.on_fail_locked,
                    over.quarantine_guard,
                )
            }
            Err(err) => tracing::warn!("ignoring invalid fail_policy: {err}"),
        }
    }
    let explicit = drop_mode::yaml_sets_on_fail(&req.yaml) || req.contract.on_fail_locked;
    (
        req.contract.on_fail,
        explicit,
        req.contract.on_fail_locked,
        req.contract.quarantine_guard.clone(),
    )
}

fn clock_to_trip(clock: Option<TripClock>) -> StoredTrip {
    match clock {
        Some(clock) => StoredTrip {
            tripped_at: clock.tripped_at,
            trip_until: clock.trip_until,
        },
        None => StoredTrip::default(),
    }
}

async fn persist_effect(
    pool: &sqlx::PgPool,
    org_id: Uuid,
    contract_id: Uuid,
    effect: &drop_mode::ScopeEffect,
) -> AppResult<()> {
    let scope_id = match effect.scope {
        GuardScope::Account => None,
        GuardScope::Contract => Some(contract_id),
    };
    let scope_name = effect.scope.as_str();
    if effect.trip {
        let reason = match effect.window {
            Some(window) => format!("{window} quarantine volume crossed the safety cap"),
            None => "safety cap tripped".to_string(),
        };
        storage::trip_guard(pool, org_id, scope_id, effect.until, &reason).await?;
        storage::record_guard_event(
            pool,
            org_id,
            scope_id,
            "tripped",
            scope_name,
            effect.window,
            Some(effect.observed as i64),
            Some(effect.threshold as i64),
        )
        .await?;
        metrics::counter!(
            "contractgate_quarantine_guard_trip_total",
            "scope" => scope_name,
        )
        .increment(1);
    } else if effect.reset {
        storage::clear_guard(pool, org_id, scope_id).await?;
        storage::record_guard_event(
            pool, org_id, scope_id, "reset", scope_name, None, None, None,
        )
        .await?;
    }
    if effect.would_trip {
        storage::record_guard_event(
            pool,
            org_id,
            scope_id,
            "would_trip",
            scope_name,
            effect.window,
            Some(effect.observed as i64),
            Some(effect.threshold as i64),
        )
        .await?;
        metrics::counter!(
            "contractgate_quarantine_guard_would_trip_total",
            "scope" => scope_name,
        )
        .increment(1);
    }
    Ok(())
}

fn record_trip_metrics(contract_id: Uuid, decision: &OnFailDecision) {
    if let Some(report) = &decision.auto_trip {
        let active = if report.active && report.enforcing {
            1.0
        } else {
            0.0
        };
        metrics::gauge!(
            "contractgate_quarantine_guard_tripped",
            "scope" => report.scope.as_str(),
            "contract_id" => contract_id.to_string(),
        )
        .set(active);
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn resolve_request(
    state: &AppState,
    org_id: Option<Uuid>,
    contract_id: Uuid,
    contract: &Contract,
    yaml: &str,
    fail_policy: Option<&serde_json::Value>,
    query_on_fail: Option<&str>,
    headers: &HeaderMap,
    dry_run: bool,
    failed: usize,
) -> AppResult<OnFailDecision> {
    let request_on_fail = on_fail_from_request(query_on_fail, headers)?;
    let decision = decide(
        state,
        &PolicyRequest {
            org_id,
            contract_id,
            contract: contract.clone(),
            yaml: yaml.to_string(),
            fail_policy: fail_policy.cloned(),
            request_on_fail,
            dry_run,
            failed,
        },
    )
    .await?;
    reject_if_locked(&decision)?;
    Ok(decision)
}

pub fn reject_if_locked(decision: &OnFailDecision) -> AppResult<()> {
    if decision.lock_rejected {
        Err(AppError::BadRequest(
            "on_fail is locked on this contract".into(),
        ))
    } else {
        Ok(())
    }
}

pub fn auto_trip_body(report: Option<&AutoTripReport>) -> Option<AutoTripBody> {
    report.map(|report| AutoTripBody {
        active: report.active,
        would_trip: report.would_trip,
        enforcing: report.enforcing,
        scope: report.scope.as_str(),
        until: report.until,
    })
}

#[derive(Debug, Serialize)]
pub struct AutoTripBody {
    pub active: bool,
    pub would_trip: bool,
    pub enforcing: bool,
    pub scope: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub until: Option<DateTime<Utc>>,
}

/// One failed record, reduced to kind and field path.
#[derive(Clone)]
pub struct FailureFacts {
    pub violations: Vec<(String, String)>,
}

#[allow(clippy::too_many_arguments)]
pub async fn record_drops(
    state: &AppState,
    org_id: Option<Uuid>,
    contract_id: Uuid,
    contract_version: &str,
    direction: &str,
    source: &str,
    sample: bool,
    failures: &[FailureFacts],
) -> AppResult<()> {
    if failures.is_empty() {
        return Ok(());
    }
    let pairs: Vec<Vec<(&str, &str)>> = failures
        .iter()
        .map(|failure| {
            failure
                .violations
                .iter()
                .map(|(kind, field)| (kind.as_str(), field.as_str()))
                .collect()
        })
        .collect();
    let tallies = drop_mode::tally_reasons(&pairs);
    let bucket = minute_bucket(Utc::now());
    storage::record_drop_counts(
        &state.db,
        org_id,
        contract_id,
        contract_version,
        direction,
        bucket,
        source,
        &tallies,
    )
    .await?;
    for (reason, count) in &tallies {
        let kind = reason.split(':').next().unwrap_or(reason);
        metrics::counter!(
            "contractgate_dropped_total",
            "reason" => kind.to_string(),
            "source" => source.to_string(),
        )
        .increment(*count);
    }
    if sample {
        let docs: Vec<serde_json::Value> = failures
            .iter()
            .take(drop_mode::SAMPLE_CAP)
            .map(|failure| {
                let facts: Vec<SampleFact> = failure
                    .violations
                    .iter()
                    .map(|(kind, field)| SampleFact {
                        kind: kind.clone(),
                        field: field.clone(),
                    })
                    .collect();
                drop_mode::sample_document(&facts)
            })
            .collect();
        storage::record_drop_samples(
            &state.db,
            org_id,
            contract_id,
            contract_version,
            direction,
            bucket,
            &docs,
        )
        .await?;
    }
    crate::metering::record_batch_usage_blocking(&state.db, org_id, failures.len()).await;
    Ok(())
}

pub async fn record_quarantined(
    state: &AppState,
    org_id: Option<Uuid>,
    contract_id: Uuid,
    count: usize,
) -> AppResult<()> {
    storage::record_quarantine_volume(
        &state.db,
        org_id,
        contract_id,
        minute_bucket(Utc::now()),
        count as i64,
    )
    .await
}

fn facts_from_violations(violations: &[crate::validation::Violation]) -> Vec<(String, String)> {
    violations
        .iter()
        .map(|violation| {
            let kind = serde_json::to_string(&violation.kind)
                .unwrap_or_else(|_| "\"unspecified\"".into())
                .trim_matches('"')
                .to_string();
            (kind, violation.field.clone())
        })
        .collect()
}

pub fn failure_facts(violations: &[crate::validation::Violation]) -> FailureFacts {
    FailureFacts {
        violations: facts_from_violations(violations),
    }
}

// ---------------------------------------------------------------------------
// Dashboard / admin HTTP
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct FailPolicyBody {
    pub on_fail: String,
    pub on_fail_locked: bool,
    pub quarantine_guard: QuarantineGuardConfig,
    pub from_override: bool,
    pub account_guard: Option<QuarantineGuardConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_trip: Option<AutoTripBody>,
}

#[derive(Debug, Deserialize)]
pub struct SetFailPolicyBody {
    pub on_fail: OnFailMode,
    #[serde(default)]
    pub on_fail_locked: bool,
    #[serde(default)]
    pub quarantine_guard: QuarantineGuardConfig,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SetAccountGuardBody {
    pub quarantine_guard: Option<QuarantineGuardConfig>,
}

#[derive(Debug, Deserialize)]
pub struct DropCountQuery {
    /// Hours of minute buckets to sum. Defaults to 24. Capped at 24 * 30.
    #[serde(default)]
    pub hours: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct ResetBody {
    pub reset: bool,
    pub scope: &'static str,
}

fn ensure_org(state: &AppState, org_id: Option<Uuid>) -> AppResult<Uuid> {
    match org_id {
        Some(org_id) => Ok(org_id),
        None if !state.auth_configured() => Err(AppError::BadRequest(
            "account safety cap needs an org".into(),
        )),
        None => Err(AppError::Unauthorized),
    }
}

struct ConfiguredPolicy {
    on_fail: OnFailMode,
    locked: bool,
    guard: QuarantineGuardConfig,
    from_override: bool,
}

fn configured_policy(
    contract: &Contract,
    fail_policy: Option<&serde_json::Value>,
) -> ConfiguredPolicy {
    if let Some(value) = fail_policy {
        if let Ok(over) = serde_json::from_value::<FailPolicyOverride>(value.clone()) {
            return ConfiguredPolicy {
                on_fail: over.on_fail,
                locked: over.on_fail_locked,
                guard: over.quarantine_guard,
                from_override: true,
            };
        }
    }
    ConfiguredPolicy {
        on_fail: contract.on_fail,
        locked: contract.on_fail_locked,
        guard: contract.quarantine_guard.clone(),
        from_override: false,
    }
}

async fn load_policy_body(
    state: &AppState,
    org_id: Option<Uuid>,
    contract_id: Uuid,
) -> AppResult<FailPolicyBody> {
    let identity = storage::get_contract_identity(&state.db, contract_id, org_id).await?;
    let version = match storage::get_latest_stable_version(&state.db, contract_id).await? {
        Some(version) => version,
        None => {
            let versions = storage::list_versions(&state.db, contract_id, org_id).await?;
            let summary = versions
                .into_iter()
                .next()
                .ok_or(AppError::NoStableVersion { contract_id })?;
            storage::get_version(&state.db, contract_id, &summary.version, org_id).await?
        }
    };
    let contract: Contract = serde_yaml::from_str(&version.yaml_content)
        .map_err(|err| AppError::InvalidContractYaml(err.to_string()))?;
    let configured = configured_policy(&contract, identity.fail_policy.as_ref());
    let decision = decide(
        state,
        &PolicyRequest {
            org_id,
            contract_id,
            contract,
            yaml: version.yaml_content,
            fail_policy: identity.fail_policy,
            request_on_fail: None,
            dry_run: true,
            failed: 0,
        },
    )
    .await?;
    let account_guard = match org_id {
        Some(org_id) => storage::org_quarantine_guard(&state.db, org_id).await?,
        None => None,
    };
    Ok(FailPolicyBody {
        on_fail: configured.on_fail.as_str().to_string(),
        on_fail_locked: configured.locked,
        quarantine_guard: configured.guard,
        from_override: configured.from_override,
        account_guard,
        auto_trip: auto_trip_body(decision.auto_trip.as_ref()),
    })
}

pub async fn get_fail_policy_handler(
    State(state): State<Arc<AppState>>,
    OrgId(org_id): OrgId,
    Path(contract_id): Path<Uuid>,
) -> AppResult<Json<FailPolicyBody>> {
    Ok(Json(load_policy_body(&state, org_id, contract_id).await?))
}

pub async fn set_fail_policy_handler(
    State(state): State<Arc<AppState>>,
    OrgId(org_id): OrgId,
    Path(contract_id): Path<Uuid>,
    Json(body): Json<SetFailPolicyBody>,
) -> AppResult<Json<FailPolicyBody>> {
    require_owner(&state, contract_id, org_id).await?;
    body.quarantine_guard
        .validate()
        .map_err(AppError::BadRequest)?;
    let policy = FailPolicyOverride {
        on_fail: body.on_fail,
        on_fail_locked: body.on_fail_locked,
        quarantine_guard: body.quarantine_guard,
    };
    let updated = storage::set_fail_policy(&state.db, contract_id, org_id, &policy).await?;
    if !updated {
        return Err(AppError::ContractNotFound(contract_id));
    }
    Ok(Json(load_policy_body(&state, org_id, contract_id).await?))
}

pub async fn get_account_guard_handler(
    State(state): State<Arc<AppState>>,
    OrgId(org_id): OrgId,
) -> AppResult<Json<SetAccountGuardBody>> {
    let org_id = ensure_org(&state, org_id)?;
    let quarantine_guard = storage::org_quarantine_guard(&state.db, org_id).await?;
    Ok(Json(SetAccountGuardBody { quarantine_guard }))
}

pub async fn set_account_guard_handler(
    State(state): State<Arc<AppState>>,
    OrgId(org_id): OrgId,
    Json(body): Json<SetAccountGuardBody>,
) -> AppResult<Json<SetAccountGuardBody>> {
    let org_id = ensure_org(&state, org_id)?;
    if let Some(guard) = &body.quarantine_guard {
        guard.validate().map_err(AppError::BadRequest)?;
    }
    storage::set_org_quarantine_guard(&state.db, org_id, body.quarantine_guard.as_ref()).await?;
    Ok(Json(body))
}

pub async fn drop_counts_handler(
    State(state): State<Arc<AppState>>,
    OrgId(org_id): OrgId,
    Path(contract_id): Path<Uuid>,
    Query(query): Query<DropCountQuery>,
) -> AppResult<Json<Vec<storage::DropCountRow>>> {
    let _ = storage::get_contract_identity(&state.db, contract_id, org_id).await?;
    let hours = query.hours.unwrap_or(24).clamp(1, 24 * 30);
    let since = Utc::now() - Duration::hours(hours);
    Ok(Json(
        storage::drop_counts_since(&state.db, contract_id, since).await?,
    ))
}

pub async fn reset_contract_guard_handler(
    State(state): State<Arc<AppState>>,
    OrgId(org_id): OrgId,
    Path(contract_id): Path<Uuid>,
) -> AppResult<Json<ResetBody>> {
    require_owner(&state, contract_id, org_id).await?;
    let Some(org_id) = org_id else {
        return Ok(Json(ResetBody {
            reset: false,
            scope: "contract",
        }));
    };
    let cleared = storage::clear_guard(&state.db, org_id, Some(contract_id)).await?;
    if cleared > 0 {
        storage::record_guard_event(
            &state.db,
            org_id,
            Some(contract_id),
            "reset",
            "contract",
            None,
            None,
            None,
        )
        .await?;
    }
    Ok(Json(ResetBody {
        reset: cleared > 0,
        scope: "contract",
    }))
}

pub async fn reset_account_guard_handler(
    State(state): State<Arc<AppState>>,
    OrgId(org_id): OrgId,
) -> AppResult<Json<ResetBody>> {
    let org_id = ensure_org(&state, org_id)?;
    let cleared = storage::clear_guard(&state.db, org_id, None).await?;
    if cleared > 0 {
        storage::record_guard_event(
            &state.db, org_id, None, "reset", "account", None, None, None,
        )
        .await?;
    }
    Ok(Json(ResetBody {
        reset: cleared > 0,
        scope: "account",
    }))
}
