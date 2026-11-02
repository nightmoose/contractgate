//! Live Free quarantine cap. Pending counts are cached briefly per org and
//! contract. A lookup failure leaves the batch on its existing on-fail mode.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use uuid::Uuid;

use crate::drop_mode::{self, OnFailDecision, PlanCapCounts};
use crate::error::AppResult;
use crate::plan;
use crate::storage;
use crate::AppState;

struct Entry {
    at: Instant,
    org_pending: i64,
    contract_pending: i64,
}

fn cache() -> &'static Mutex<HashMap<(Uuid, Uuid), Entry>> {
    static CACHE: std::sync::OnceLock<Mutex<HashMap<(Uuid, Uuid), Entry>>> =
        std::sync::OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn lock() -> std::sync::MutexGuard<'static, HashMap<(Uuid, Uuid), Entry>> {
    cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Adjust cached pending counts after a quarantine insert or a review.
/// A miss is left for the next database read.
pub fn note_pending_delta(org_id: Uuid, contract_id: Uuid, delta: i64) {
    for ((org, contract), entry) in lock().iter_mut() {
        if *org != org_id {
            continue;
        }
        entry.org_pending = (entry.org_pending + delta).max(0);
        if *contract == contract_id {
            entry.contract_pending = (entry.contract_pending + delta).max(0);
        }
    }
}

pub async fn apply(
    state: &AppState,
    org_id: Option<Uuid>,
    contract_id: Uuid,
    decision: OnFailDecision,
) -> OnFailDecision {
    let Some(org_id) = org_id else {
        return decision;
    };
    if state.self_hosted {
        return decision;
    }
    let plan_name = match storage::get_org_plan(&state.db, org_id).await {
        Ok(plan) => plan.unwrap_or_else(|| "free".to_string()),
        Err(err) => {
            tracing::warn!("quarantine cap left unchanged; plan lookup failed: {err}");
            return decision;
        }
    };
    let org_limit = plan::quarantine_row_cap(&plan_name);
    let contract_limit = plan::per_contract_quarantine_cap(&plan_name);
    if org_limit.is_none() && contract_limit.is_none() {
        return decision;
    }
    let (org_pending, contract_pending) = match cached_counts(state, org_id, contract_id).await {
        Ok(counts) => counts,
        Err(err) => {
            tracing::warn!("quarantine cap left unchanged; pending count failed: {err}");
            return decision;
        }
    };
    let decision = drop_mode::apply_plan_cap(
        decision,
        PlanCapCounts {
            org_pending,
            org_limit,
            contract_pending,
            contract_limit,
            near_percent: plan::QUARANTINE_CAP_NEAR_PERCENT,
        },
    );
    if let Some(view) = &decision.quarantine_cap {
        let code = match view.state {
            "reached" => 2.0,
            "near" => 1.0,
            _ => 0.0,
        };
        metrics::gauge!(
            "contractgate_quarantine_cap_state",
            "scope" => view.scope,
            "state" => view.state,
        )
        .set(code);
    }
    // Pending is adjusted when rows are actually inserted. An atomic
    // rejection reports quarantined records and stores none, so it must
    // not consume cap space.
    decision
}

async fn cached_counts(state: &AppState, org_id: Uuid, contract_id: Uuid) -> AppResult<(i64, i64)> {
    let key = (org_id, contract_id);
    let ttl = Duration::from_secs(plan::QUARANTINE_COUNT_CACHE_SECS);
    if let Some(entry) = lock().get(&key) {
        if entry.at.elapsed() < ttl {
            return Ok((entry.org_pending, entry.contract_pending));
        }
    }
    let counts = storage::pending_quarantine_counts(&state.db, org_id, contract_id).await?;
    let mut guard = lock();
    if guard.len() > 1024 {
        guard.clear();
    }
    guard.insert(
        key,
        Entry {
            at: Instant::now(),
            org_pending: counts.0,
            contract_pending: counts.1,
        },
    );
    Ok(counts)
}
