//! Failure routing for the hosted Kafka and Kinesis ingest consumers.
//!
//! Both consumers call [`route_failures`]. That uses [`crate::fail_policy::decide`],
//! which applies the safety cap and then [`crate::quarantine_cap::apply`].
//! A quarantine decision is published to the quarantine topic. A drop
//! decision is the counted drop path (`dropped_event_counts`) and is not
//! published. Passing records are unaffected.
//!
//! A contract or policy lookup failure leaves the record on the previous
//! route: publish to the quarantine topic.
//!
//! The entry points are called from the `kafka-ingress` and `kinesis-ingress`
//! consumers. A default build does not compile those callers.

#![allow(dead_code)]

use uuid::Uuid;

#[cfg(test)]
use crate::contract::OnFailMode;
use crate::contract::{Contract, OnFailSource};
use crate::drop_mode::{self, OnFailDecision};
use crate::error::AppResult;
use crate::fail_policy::{self, FailureFacts, PolicyRequest};
use crate::AppState;

/// Live on-fail override for one contract, or [`ContractPolicy::Unavailable`]
/// when the identity row could not be read.
pub enum ContractPolicy {
    Ready(Option<serde_json::Value>),
    Unavailable,
}

impl ContractPolicy {
    pub fn is_ready(&self) -> bool {
        matches!(self, ContractPolicy::Ready(_))
    }
}

/// What a failed stream record does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureRoute {
    /// Publish to the quarantine topic.
    PublishQuarantine,
    /// Count the failure. Do not publish it.
    CountedDrop { source: OnFailSource, sample: bool },
}

impl FailureRoute {
    pub fn publishes_quarantine(self) -> bool {
        matches!(self, FailureRoute::PublishQuarantine)
    }
}

pub async fn load_contract_policy(state: &AppState, contract_id: Uuid) -> ContractPolicy {
    match crate::storage::get_contract_identity(&state.db, contract_id, None).await {
        Ok(identity) => ContractPolicy::Ready(identity.fail_policy),
        Err(err) => {
            tracing::warn!("stream on-fail left unchanged; contract lookup failed: {err}");
            ContractPolicy::Unavailable
        }
    }
}

/// Resolve one batch of failures. `failed` is the number of records that did
/// not pass, including JSON that did not parse. There is no request override
/// on these routes.
pub async fn route_failures(
    state: &AppState,
    org_id: Option<Uuid>,
    contract_id: Uuid,
    contract: &Contract,
    yaml: &str,
    policy: &ContractPolicy,
    failed: usize,
) -> FailureRoute {
    let ContractPolicy::Ready(fail_policy) = policy else {
        return FailureRoute::PublishQuarantine;
    };
    match resolve(
        state,
        org_id,
        contract_id,
        contract,
        yaml,
        fail_policy.as_ref(),
        failed,
    )
    .await
    {
        Ok(decision) => failure_route(&decision),
        Err(err) => {
            tracing::warn!("stream on-fail left unchanged; policy lookup failed: {err}");
            FailureRoute::PublishQuarantine
        }
    }
}

async fn resolve(
    state: &AppState,
    org_id: Option<Uuid>,
    contract_id: Uuid,
    contract: &Contract,
    yaml: &str,
    fail_policy: Option<&serde_json::Value>,
    failed: usize,
) -> AppResult<OnFailDecision> {
    fail_policy::decide(
        state,
        &PolicyRequest {
            org_id,
            contract_id,
            contract: contract.clone(),
            yaml: yaml.to_string(),
            fail_policy: fail_policy.cloned(),
            request_on_fail: None,
            dry_run: false,
            failed,
        },
    )
    .await
}

pub fn failure_route(decision: &OnFailDecision) -> FailureRoute {
    if drop_mode::writes_quarantine(false, decision.mode, false) {
        FailureRoute::PublishQuarantine
    } else {
        FailureRoute::CountedDrop {
            source: decision.source,
            sample: decision.sample_drops,
        }
    }
}

pub fn parse_failure() -> FailureFacts {
    FailureFacts {
        violations: vec![("json_parse_error".into(), String::new())],
    }
}

pub async fn record_counted_drops(
    state: &AppState,
    org_id: Option<Uuid>,
    contract_id: Uuid,
    contract_version: &str,
    source: OnFailSource,
    sample: bool,
    failures: &[FailureFacts],
) {
    if let Err(err) = fail_policy::record_drops(
        state,
        org_id,
        contract_id,
        contract_version,
        "ingress",
        source.as_str(),
        sample,
        failures,
    )
    .await
    {
        tracing::warn!("stream drop ledger write failed: {err}");
    }
}

pub async fn record_published_quarantine(
    state: &AppState,
    org_id: Option<Uuid>,
    contract_id: Uuid,
    count: usize,
) {
    if count == 0 {
        return;
    }
    if let Err(err) = fail_policy::record_quarantined(state, org_id, contract_id, count).await {
        tracing::warn!("stream quarantine volume write failed: {err}");
    }
}

/// The pure half of [`route_failures`]: safety cap, then the Free plan cap.
/// Stream consumers pass no request override and are never a dry run.
#[cfg(test)]
pub(crate) fn route_resolved(
    input: &drop_mode::ResolveInput,
    counts: drop_mode::PlanCapCounts,
) -> FailureRoute {
    failure_route(&drop_mode::apply_plan_cap(
        drop_mode::resolve_on_fail(input),
        counts,
    ))
}

#[cfg(test)]
pub(crate) fn free_cap_counts(org_pending: i64, contract_pending: i64) -> drop_mode::PlanCapCounts {
    drop_mode::PlanCapCounts {
        org_pending,
        org_limit: Some(crate::plan::FREE_QUARANTINE_ORG_CAP),
        contract_pending,
        contract_limit: Some(crate::plan::FREE_QUARANTINE_CONTRACT_CAP),
        near_percent: crate::plan::QUARANTINE_CAP_NEAR_PERCENT,
    }
}

#[cfg(test)]
pub(crate) fn stream_input(mode: OnFailMode, explicit: bool) -> drop_mode::ResolveInput {
    drop_mode::ResolveInput {
        request: None,
        contract_mode: mode,
        contract_explicit: explicit,
        locked: false,
        account: None,
        contract_guard: drop_mode::ScopeView::default(),
        now: chrono::DateTime::parse_from_rfc3339("2026-10-30T18:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc),
        failed: 1,
        dry_run: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quarantine_publishes_and_drop_does_not() {
        let publish = route_resolved(
            &stream_input(OnFailMode::Quarantine, false),
            free_cap_counts(1, 1),
        );
        assert_eq!(publish, FailureRoute::PublishQuarantine);
        assert!(publish.publishes_quarantine());

        let dropped = route_resolved(&stream_input(OnFailMode::Drop, true), free_cap_counts(1, 1));
        assert_eq!(
            dropped,
            FailureRoute::CountedDrop {
                source: OnFailSource::Contract,
                sample: false,
            }
        );
        assert!(!dropped.publishes_quarantine());
    }
}
