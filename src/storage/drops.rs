//! Drop-mode ledger, quarantine volume buckets, and safety-cap state.

use chrono::{DateTime, Timelike, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::contract::{FailPolicyOverride, QuarantineGuardConfig};
use crate::error::{AppResult, DbOpContext};

#[derive(Debug, Clone, Copy, Default)]
pub struct VolumeCounts {
    pub contract_minute: i64,
    pub contract_hour: i64,
    pub account_minute: i64,
    pub account_hour: i64,
}

#[derive(Debug, Clone, Copy)]
pub struct TripClock {
    pub tripped_at: Option<DateTime<Utc>>,
    pub trip_until: Option<DateTime<Utc>>,
}

pub async fn org_quarantine_guard(
    pool: &PgPool,
    org_id: Uuid,
) -> AppResult<Option<QuarantineGuardConfig>> {
    let row: Option<(Option<serde_json::Value>,)> =
        sqlx::query_as("SELECT quarantine_guard FROM orgs WHERE id = $1")
            .bind(org_id)
            .fetch_optional(pool)
            .await
            .db_op("org_quarantine_guard")?;
    Ok(row.and_then(|(value,)| parse_guard(value)))
}

pub async fn volume_counts(
    pool: &PgPool,
    org_id: Option<Uuid>,
    contract_id: Uuid,
    minute: DateTime<Utc>,
    hour: DateTime<Utc>,
) -> AppResult<VolumeCounts> {
    let contract: (i64, i64) = sqlx::query_as(
        r#"
        SELECT
            COALESCE(SUM(quarantined) FILTER (WHERE bucket_start = $2), 0)::bigint,
            COALESCE(SUM(quarantined), 0)::bigint
        FROM quarantine_volume_minute
        WHERE contract_id = $1 AND bucket_start >= $3
        "#,
    )
    .bind(contract_id)
    .bind(minute)
    .bind(hour)
    .fetch_one(pool)
    .await
    .db_op("volume_counts:contract")?;

    let account = if let Some(org_id) = org_id {
        sqlx::query_as(
            r#"
            SELECT
                COALESCE(SUM(quarantined) FILTER (WHERE bucket_start = $2), 0)::bigint,
                COALESCE(SUM(quarantined), 0)::bigint
            FROM quarantine_volume_minute
            WHERE org_id = $1 AND bucket_start >= $3
            "#,
        )
        .bind(org_id)
        .bind(minute)
        .bind(hour)
        .fetch_one(pool)
        .await
        .db_op("volume_counts:account")?
    } else {
        (0, 0)
    };

    Ok(VolumeCounts {
        contract_minute: contract.0,
        contract_hour: contract.1,
        account_minute: account.0,
        account_hour: account.1,
    })
}

pub async fn trip_clocks(
    pool: &PgPool,
    org_id: Uuid,
    contract_id: Uuid,
) -> AppResult<(Option<TripClock>, Option<TripClock>)> {
    type TripRow = (Option<Uuid>, Option<DateTime<Utc>>, Option<DateTime<Utc>>);
    let rows: Vec<TripRow> = sqlx::query_as(
        r#"
        SELECT contract_id, tripped_at, trip_until
        FROM quarantine_guard_state
        WHERE org_id = $1 AND (contract_id IS NULL OR contract_id = $2)
        "#,
    )
    .bind(org_id)
    .bind(contract_id)
    .fetch_all(pool)
    .await
    .db_op("trip_clocks")?;

    let mut account = None;
    let mut contract = None;
    for (id, tripped_at, trip_until) in rows {
        let clock = TripClock {
            tripped_at,
            trip_until,
        };
        if id.is_none() {
            account = Some(clock);
        } else {
            contract = Some(clock);
        }
    }
    Ok((account, contract))
}

pub async fn record_quarantine_volume(
    pool: &PgPool,
    org_id: Option<Uuid>,
    contract_id: Uuid,
    bucket: DateTime<Utc>,
    count: i64,
) -> AppResult<()> {
    if count <= 0 {
        return Ok(());
    }
    sqlx::query(
        r#"
        INSERT INTO quarantine_volume_minute (org_id, contract_id, bucket_start, quarantined)
        VALUES ($1, $2, $3, $4)
        ON CONFLICT (contract_id, bucket_start)
        DO UPDATE SET quarantined = quarantine_volume_minute.quarantined + EXCLUDED.quarantined
        "#,
    )
    .bind(org_id)
    .bind(contract_id)
    .bind(bucket)
    .bind(count)
    .execute(pool)
    .await
    .db_op("record_quarantine_volume")?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn record_drop_counts(
    pool: &PgPool,
    org_id: Option<Uuid>,
    contract_id: Uuid,
    contract_version: &str,
    direction: &str,
    bucket: DateTime<Utc>,
    source: &str,
    reasons: &[(String, u64)],
) -> AppResult<()> {
    for (reason, count) in reasons {
        if *count == 0 {
            continue;
        }
        sqlx::query(
            r#"
            INSERT INTO dropped_event_counts (
                org_id, contract_id, contract_version, direction,
                bucket_start, source, reason_key, dropped
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            ON CONFLICT (org_id, contract_id, contract_version, direction, bucket_start, source, reason_key)
            DO UPDATE SET dropped = dropped_event_counts.dropped + EXCLUDED.dropped
            "#,
        )
        .bind(org_id)
        .bind(contract_id)
        .bind(contract_version)
        .bind(direction)
        .bind(bucket)
        .bind(source)
        .bind(reason)
        .bind(*count as i64)
        .execute(pool)
        .await
        .db_op("record_drop_counts")?;
    }
    Ok(())
}

pub async fn record_drop_samples(
    pool: &PgPool,
    org_id: Option<Uuid>,
    contract_id: Uuid,
    contract_version: &str,
    direction: &str,
    bucket: DateTime<Utc>,
    samples: &[serde_json::Value],
) -> AppResult<()> {
    if samples.is_empty() {
        return Ok(());
    }
    let existing: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)::bigint FROM dropped_event_samples
        WHERE contract_id = $1 AND contract_version = $2
          AND direction = $3 AND bucket_start = $4
        "#,
    )
    .bind(contract_id)
    .bind(contract_version)
    .bind(direction)
    .bind(bucket)
    .fetch_one(pool)
    .await
    .db_op("record_drop_samples:count")?;

    let room = (5 - existing).max(0) as usize;
    for (offset, sample) in samples.iter().take(room).enumerate() {
        let idx = existing as i16 + offset as i16;
        let reason = sample["violations"][0]["kind"]
            .as_str()
            .unwrap_or("unspecified");
        let field = sample["violations"][0]["field"].as_str().unwrap_or("");
        let reason_key = if field.is_empty() {
            reason.to_string()
        } else {
            format!("{reason}:{field}")
        };
        sqlx::query(
            r#"
            INSERT INTO dropped_event_samples (
                org_id, contract_id, contract_version, direction,
                bucket_start, sample_idx, reason_key, metadata
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            ON CONFLICT (contract_id, contract_version, direction, bucket_start, sample_idx)
            DO NOTHING
            "#,
        )
        .bind(org_id)
        .bind(contract_id)
        .bind(contract_version)
        .bind(direction)
        .bind(bucket)
        .bind(idx)
        .bind(reason_key)
        .bind(sample)
        .execute(pool)
        .await
        .db_op("record_drop_samples:insert")?;
    }
    Ok(())
}

pub async fn trip_guard(
    pool: &PgPool,
    org_id: Uuid,
    contract_id: Option<Uuid>,
    until: Option<DateTime<Utc>>,
    reason: &str,
) -> AppResult<()> {
    sqlx::query(
        r#"
        INSERT INTO quarantine_guard_state (
            org_id, contract_id, tripped_at, trip_until, reason, trip_count
        )
        VALUES ($1, $2, NOW(), $3, $4, 1)
        ON CONFLICT (org_id, contract_id)
        DO UPDATE SET
            tripped_at = NOW(),
            trip_until = EXCLUDED.trip_until,
            reason = EXCLUDED.reason,
            trip_count = quarantine_guard_state.trip_count + 1
        "#,
    )
    .bind(org_id)
    .bind(contract_id)
    .bind(until)
    .bind(reason)
    .execute(pool)
    .await
    .db_op("trip_guard")?;
    Ok(())
}

pub async fn clear_guard(pool: &PgPool, org_id: Uuid, contract_id: Option<Uuid>) -> AppResult<u64> {
    let result = sqlx::query(
        r#"
        UPDATE quarantine_guard_state
        SET tripped_at = NULL, trip_until = NULL, reason = NULL
        WHERE org_id = $1
          AND contract_id IS NOT DISTINCT FROM $2
          AND tripped_at IS NOT NULL
        "#,
    )
    .bind(org_id)
    .bind(contract_id)
    .execute(pool)
    .await
    .db_op("clear_guard")?;
    Ok(result.rows_affected())
}

#[allow(clippy::too_many_arguments)]
pub async fn record_guard_event(
    pool: &PgPool,
    org_id: Uuid,
    contract_id: Option<Uuid>,
    kind: &str,
    scope: &str,
    window_name: Option<&str>,
    observed: Option<i64>,
    threshold: Option<i64>,
) -> AppResult<()> {
    if kind == "would_trip" {
        let minute = chrono::Utc::now()
            .with_second(0)
            .and_then(|t| t.with_nanosecond(0));
        let already: bool = sqlx::query_scalar(
            r#"
            SELECT EXISTS (
                SELECT 1 FROM quarantine_guard_events
                WHERE org_id = $1
                  AND contract_id IS NOT DISTINCT FROM $2
                  AND kind = 'would_trip'
                  AND observed_at >= $3
            )
            "#,
        )
        .bind(org_id)
        .bind(contract_id)
        .bind(minute)
        .fetch_one(pool)
        .await
        .db_op("record_guard_event:exists")?;
        if already {
            return Ok(());
        }
    }
    sqlx::query(
        r#"
        INSERT INTO quarantine_guard_events (
            org_id, contract_id, kind, scope, window_name, observed, threshold
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        "#,
    )
    .bind(org_id)
    .bind(contract_id)
    .bind(kind)
    .bind(scope)
    .bind(window_name)
    .bind(observed)
    .bind(threshold)
    .execute(pool)
    .await
    .db_op("record_guard_event")?;
    Ok(())
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct DropCountRow {
    pub reason_key: String,
    pub source: String,
    pub dropped: i64,
}

pub async fn drop_counts_since(
    pool: &PgPool,
    contract_id: Uuid,
    since: DateTime<Utc>,
) -> AppResult<Vec<DropCountRow>> {
    let rows = sqlx::query_as(
        r#"
        SELECT reason_key, source, SUM(dropped)::bigint AS dropped
        FROM dropped_event_counts
        WHERE contract_id = $1 AND bucket_start >= $2
        GROUP BY reason_key, source
        ORDER BY SUM(dropped) DESC
        LIMIT 50
        "#,
    )
    .bind(contract_id)
    .bind(since)
    .fetch_all(pool)
    .await
    .db_op("drop_counts_since")?;
    Ok(rows)
}

pub async fn set_fail_policy(
    pool: &PgPool,
    contract_id: Uuid,
    org_id: Option<Uuid>,
    policy: &FailPolicyOverride,
) -> AppResult<bool> {
    let value = serde_json::to_value(policy)
        .map_err(|e| crate::error::AppError::Internal(format!("fail_policy encode: {e}")))?;
    let result = sqlx::query(
        r#"
        UPDATE contracts
        SET fail_policy = $2, updated_at = NOW()
        WHERE id = $1 AND deleted_at IS NULL AND ($3::uuid IS NULL OR org_id = $3)
        "#,
    )
    .bind(contract_id)
    .bind(value)
    .bind(org_id)
    .execute(pool)
    .await
    .db_op("set_fail_policy")?;
    Ok(result.rows_affected() > 0)
}

pub async fn set_org_quarantine_guard(
    pool: &PgPool,
    org_id: Uuid,
    guard: Option<&QuarantineGuardConfig>,
) -> AppResult<()> {
    let value = match guard {
        Some(guard) => Some(serde_json::to_value(guard).map_err(|e| {
            crate::error::AppError::Internal(format!("quarantine_guard encode: {e}"))
        })?),
        None => None,
    };
    sqlx::query("UPDATE orgs SET quarantine_guard = $2 WHERE id = $1")
        .bind(org_id)
        .bind(value)
        .execute(pool)
        .await
        .db_op("set_org_quarantine_guard")?;
    Ok(())
}

fn parse_guard(value: Option<serde_json::Value>) -> Option<QuarantineGuardConfig> {
    let value = value?;
    match serde_json::from_value::<QuarantineGuardConfig>(value) {
        Ok(config) => Some(config),
        Err(err) => {
            tracing::warn!("ignoring invalid quarantine_guard: {err}");
            None
        }
    }
}
