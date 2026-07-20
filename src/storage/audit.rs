//! Ingest-path audit/quarantine/forward writes + reporting reads.
//!
//! Split out of the original monolithic `storage.rs` (2026-07-10, RFC/worklist
//! item 3). No writes here touch the hot ingest validation path directly —
//! these are the post-validation persistence + reporting queries.

use crate::error::{AppResult, DbOpContext};
use crate::transform::TransformedPayload;
use sqlx::PgPool;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Internal row helpers for aggregate stats.
// ---------------------------------------------------------------------------

/// Maps the COUNT/SUM/AVG aggregate query for ingestion stats.
/// All fields use Option<> for safety across different row-count scenarios.
#[derive(sqlx::FromRow)]
struct StatsRow {
    total: Option<i64>,
    passed: Option<i64>,
    avg_us: Option<f64>,
}

/// Maps the percentile_disc aggregate query results.
#[derive(sqlx::FromRow)]
struct PercRow {
    p50: Option<i64>,
    p95: Option<i64>,
    p99: Option<i64>,
}

// ---------------------------------------------------------------------------
// Quarantine events
// ---------------------------------------------------------------------------

/// Write a failed event to the quarantine table.
///
/// Kept as a single-event helper for ad-hoc use (scripts, manual replays).
/// The live ingest handler uses [`quarantine_events_batch`] — see RFC-001.
///
/// `contract_version` must be the exact version that rejected the event
/// (audit honesty — see `feedback_audit_honesty` memory).
///
/// `payload` is [`TransformedPayload`] (RFC-004 §6) — raw PII never reaches
/// `quarantine_events.payload`.  Callers that already hold a stored value
/// (replay re-quarantine, etc.) can wrap it via
/// [`TransformedPayload::from_stored`].
#[allow(clippy::too_many_arguments, dead_code)]
pub async fn quarantine_event(
    pool: &PgPool,
    contract_id: Uuid,
    contract_version: &str,
    payload: TransformedPayload,
    violation_count: i32,
    violation_details: serde_json::Value,
    validation_us: i64,
    source_ip: Option<&str>,
    direction: &str,
) -> AppResult<()> {
    sqlx::query(
        r#"
        INSERT INTO quarantine_events
            (id, contract_id, contract_version, payload, violation_count,
             violation_details, validation_us, source_ip, direction, status, created_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 'pending', NOW())
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(contract_id)
    .bind(contract_version)
    .bind(payload.into_inner())
    .bind(violation_count)
    .bind(violation_details)
    .bind(validation_us)
    .bind(source_ip)
    .bind(direction)
    .execute(pool)
    .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Audit log
// ---------------------------------------------------------------------------

/// Insert a record into the audit log after each ingestion attempt.
///
/// `contract_version` must reflect the version that actually produced the
/// decision — never a default or guess (audit honesty).
///
/// `raw_event` is [`TransformedPayload`] (RFC-004 §6) — raw PII never
/// reaches `audit_log.raw_event`.  Summary-audit callers that carry
/// synthetic bookkeeping JSON (batch-rejected, deprecated-pin) wrap via
/// [`TransformedPayload::from_stored`].
#[allow(clippy::too_many_arguments)]
pub async fn log_audit_entry(
    pool: &PgPool,
    contract_id: Uuid,
    org_id: Option<Uuid>,
    contract_version: &str,
    passed: bool,
    violation_count: i32,
    violation_details: serde_json::Value,
    raw_event: TransformedPayload,
    validation_us: i64,
    source_ip: Option<&str>,
    source: &str,
    direction: &str,
) -> AppResult<()> {
    sqlx::query(
        r#"
        INSERT INTO audit_log
            (id, contract_id, org_id, contract_version, passed, violation_count,
             violation_details, raw_event, validation_us, source_ip, source, direction, created_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, NOW())
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(contract_id)
    .bind(org_id)
    .bind(contract_version)
    .bind(passed)
    .bind(violation_count)
    .bind(violation_details)
    .bind(raw_event.into_inner())
    .bind(validation_us)
    .bind(source_ip)
    .bind(source)
    .bind(direction)
    .execute(pool)
    .await?;
    Ok(())
}

/// Fetch recent audit entries for the dashboard monitor.
/// `org_id` scopes results to one org when Some; `contract_id` further filters
/// to one contract within that org. Pass `org_id = None` in dev-mode only.
pub async fn recent_audit_entries(
    pool: &PgPool,
    org_id: Option<Uuid>,
    contract_id: Option<Uuid>,
    limit: i64,
    offset: i64,
) -> AppResult<Vec<AuditEntry>> {
    let rows = match (org_id, contract_id) {
        (Some(oid), Some(cid)) => {
            sqlx::query_as::<_, AuditEntry>(
                r#"
                SELECT id, contract_id, contract_version, passed, violation_count,
                       violation_details, raw_event, validation_us, source_ip, created_at
                FROM audit_log
                WHERE org_id = $1 AND contract_id = $2
                ORDER BY created_at DESC
                LIMIT $3 OFFSET $4
                "#,
            )
            .bind(oid)
            .bind(cid)
            .bind(limit)
            .bind(offset)
            .fetch_all(pool)
            .await?
        }
        (Some(oid), None) => {
            sqlx::query_as::<_, AuditEntry>(
                r#"
                SELECT id, contract_id, contract_version, passed, violation_count,
                       violation_details, raw_event, validation_us, source_ip, created_at
                FROM audit_log
                WHERE org_id = $1
                ORDER BY created_at DESC
                LIMIT $2 OFFSET $3
                "#,
            )
            .bind(oid)
            .bind(limit)
            .bind(offset)
            .fetch_all(pool)
            .await?
        }
        (None, Some(cid)) => {
            // Dev-mode: no org, but still filter by contract
            sqlx::query_as::<_, AuditEntry>(
                r#"
                SELECT id, contract_id, contract_version, passed, violation_count,
                       violation_details, raw_event, validation_us, source_ip, created_at
                FROM audit_log
                WHERE contract_id = $1
                ORDER BY created_at DESC
                LIMIT $2 OFFSET $3
                "#,
            )
            .bind(cid)
            .bind(limit)
            .bind(offset)
            .fetch_all(pool)
            .await?
        }
        (None, None) => {
            // Dev-mode: no scoping at all
            sqlx::query_as::<_, AuditEntry>(
                r#"
                SELECT id, contract_id, contract_version, passed, violation_count,
                       violation_details, raw_event, validation_us, source_ip, created_at
                FROM audit_log
                ORDER BY created_at DESC
                LIMIT $1 OFFSET $2
                "#,
            )
            .bind(limit)
            .bind(offset)
            .fetch_all(pool)
            .await?
        }
    };

    Ok(rows)
}

/// Fetch summary statistics for the dashboard monitor, including p50/p95/p99 latency.
///
/// `org_id` scopes all aggregates to one org when Some.  Pass `None` only in
/// dev-mode (no auth configured).  `contract_id` further narrows within the org.
pub async fn ingestion_stats(
    pool: &PgPool,
    org_id: Option<Uuid>,
    contract_id: Option<Uuid>,
) -> AppResult<IngestionStats> {
    // -----------------------------------------------------------------------
    // Query 1: aggregate counts + average latency
    // -----------------------------------------------------------------------
    let stats: StatsRow = match (org_id, contract_id) {
        (Some(oid), Some(cid)) => {
            sqlx::query_as::<_, StatsRow>(
                r#"SELECT COUNT(*)::bigint AS total,
                      COALESCE(SUM(CASE WHEN passed THEN 1 ELSE 0 END), 0)::bigint AS passed,
                      COALESCE(AVG(validation_us::float8), 0.0) AS avg_us
               FROM audit_log WHERE org_id = $1 AND contract_id = $2"#,
            )
            .bind(oid)
            .bind(cid)
            .fetch_one(pool)
            .await?
        }
        (Some(oid), None) => {
            sqlx::query_as::<_, StatsRow>(
                r#"SELECT COUNT(*)::bigint AS total,
                      COALESCE(SUM(CASE WHEN passed THEN 1 ELSE 0 END), 0)::bigint AS passed,
                      COALESCE(AVG(validation_us::float8), 0.0) AS avg_us
               FROM audit_log WHERE org_id = $1"#,
            )
            .bind(oid)
            .fetch_one(pool)
            .await?
        }
        (None, Some(cid)) => {
            sqlx::query_as::<_, StatsRow>(
                r#"SELECT COUNT(*)::bigint AS total,
                      COALESCE(SUM(CASE WHEN passed THEN 1 ELSE 0 END), 0)::bigint AS passed,
                      COALESCE(AVG(validation_us::float8), 0.0) AS avg_us
               FROM audit_log WHERE contract_id = $1"#,
            )
            .bind(cid)
            .fetch_one(pool)
            .await?
        }
        (None, None) => {
            sqlx::query_as::<_, StatsRow>(
                r#"SELECT COUNT(*)::bigint AS total,
                      COALESCE(SUM(CASE WHEN passed THEN 1 ELSE 0 END), 0)::bigint AS passed,
                      COALESCE(AVG(validation_us::float8), 0.0) AS avg_us
               FROM audit_log"#,
            )
            .fetch_one(pool)
            .await?
        }
    };

    let total = stats.total.unwrap_or(0);
    let passed = stats.passed.unwrap_or(0);
    let avg_us = stats.avg_us.unwrap_or(0.0);

    // -----------------------------------------------------------------------
    // Query 2: percentile latencies (p50 / p95 / p99)
    // -----------------------------------------------------------------------
    let perc: PercRow = match (org_id, contract_id) {
        (Some(oid), Some(cid)) => {
            sqlx::query_as::<_, PercRow>(
                r#"SELECT percentile_disc(0.50) WITHIN GROUP (ORDER BY validation_us) AS p50,
                      percentile_disc(0.95) WITHIN GROUP (ORDER BY validation_us) AS p95,
                      percentile_disc(0.99) WITHIN GROUP (ORDER BY validation_us) AS p99
               FROM audit_log WHERE org_id = $1 AND contract_id = $2"#,
            )
            .bind(oid)
            .bind(cid)
            .fetch_one(pool)
            .await?
        }
        (Some(oid), None) => {
            sqlx::query_as::<_, PercRow>(
                r#"SELECT percentile_disc(0.50) WITHIN GROUP (ORDER BY validation_us) AS p50,
                      percentile_disc(0.95) WITHIN GROUP (ORDER BY validation_us) AS p95,
                      percentile_disc(0.99) WITHIN GROUP (ORDER BY validation_us) AS p99
               FROM audit_log WHERE org_id = $1"#,
            )
            .bind(oid)
            .fetch_one(pool)
            .await?
        }
        (None, Some(cid)) => {
            sqlx::query_as::<_, PercRow>(
                r#"SELECT percentile_disc(0.50) WITHIN GROUP (ORDER BY validation_us) AS p50,
                      percentile_disc(0.95) WITHIN GROUP (ORDER BY validation_us) AS p95,
                      percentile_disc(0.99) WITHIN GROUP (ORDER BY validation_us) AS p99
               FROM audit_log WHERE contract_id = $1"#,
            )
            .bind(cid)
            .fetch_one(pool)
            .await?
        }
        (None, None) => {
            sqlx::query_as::<_, PercRow>(
                r#"SELECT percentile_disc(0.50) WITHIN GROUP (ORDER BY validation_us) AS p50,
                      percentile_disc(0.95) WITHIN GROUP (ORDER BY validation_us) AS p95,
                      percentile_disc(0.99) WITHIN GROUP (ORDER BY validation_us) AS p99
               FROM audit_log"#,
            )
            .fetch_one(pool)
            .await?
        }
    };

    Ok(IngestionStats {
        total_events: total,
        passed_events: passed,
        failed_events: total - passed,
        pass_rate: if total > 0 {
            passed as f64 / total as f64
        } else {
            0.0
        },
        avg_validation_us: avg_us,
        p50_validation_us: perc.p50.unwrap_or(0),
        p95_validation_us: perc.p95.unwrap_or(0),
        p99_validation_us: perc.p99.unwrap_or(0),
    })
}

// ---------------------------------------------------------------------------
// Data models returned from the DB
// ---------------------------------------------------------------------------

#[derive(Debug, serde::Serialize, sqlx::FromRow)]
pub struct AuditEntry {
    pub id: Uuid,
    pub contract_id: Uuid,
    /// The exact contract version that accepted or rejected this event.
    /// Optional at the schema level (legacy rows pre-RFC-002 may be NULL),
    /// but RFC-002 writes always populate it.
    pub contract_version: Option<String>,
    pub passed: bool,
    pub violation_count: i32,
    pub violation_details: serde_json::Value,
    pub raw_event: serde_json::Value,
    pub validation_us: i64,
    pub source_ip: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, serde::Serialize)]
pub struct IngestionStats {
    pub total_events: i64,
    pub passed_events: i64,
    pub failed_events: i64,
    pub pass_rate: f64,
    pub avg_validation_us: f64,
    /// Median (p50) validation latency in microseconds
    pub p50_validation_us: i64,
    /// 95th-percentile validation latency in microseconds
    pub p95_validation_us: i64,
    /// 99th-percentile validation latency in microseconds (target: <15 000 µs)
    pub p99_validation_us: i64,
}

// ---------------------------------------------------------------------------
// Batch insert helpers (RFC-001)
//
// At 1 000 events per request, the old pattern of `tokio::spawn` per event
// would burn through 1 000+ concurrent Postgres connections per batch.  The
// helpers below collapse each side-effect kind (audit / quarantine / forward)
// into a SINGLE multi-row INSERT via PostgreSQL's `UNNEST`, which means a
// batch of 1 000 events uses at most 3 connections to durably record the
// outcome.
//
// The insert structs are plain data: the ingest handler fills them in once
// per event after validation and then hands the vectors to the helpers.
// ---------------------------------------------------------------------------

/// One row to insert into `audit_log`.  Everything needed to reconstruct the
/// ingestion decision; IDs and timestamps are filled in server-side.
///
/// `contract_version` must be the exact version that produced the decision
/// (audit honesty).  Under fallback resolution, this is the version that
/// actually accepted (or, for the final fail, rejected) the event.
///
/// `raw_event` is typed as [`TransformedPayload`] (RFC-004 §6) so the
/// compiler enforces "raw PII never reaches `audit_log`".  The only legal
/// ways to produce a value are [`crate::transform::apply_transforms`] at
/// the ingest boundary or [`TransformedPayload::from_stored`] for rows
/// whose payload was already durable-stored in post-transform form
/// (replay, summary audits).
#[derive(Debug, Clone)]
pub struct AuditEntryInsert {
    pub contract_id: Uuid,
    /// Org that owns this contract. None in dev-mode only.
    pub org_id: Option<Uuid>,
    pub contract_version: String,
    pub passed: bool,
    pub violation_count: i32,
    pub violation_details: serde_json::Value,
    pub raw_event: TransformedPayload,
    pub validation_us: i64,
    pub source_ip: Option<String>,
    /// Ingestion source tag.  "http" for the REST ingest path; "kafka" for
    /// the RFC-025 consumer pool.  Defaults to "http" for existing callers.
    pub source: String,
    /// Optional app-generated UUID.  Replay uses this so the caller can
    /// link the source quarantine row to the exact audit row it produced
    /// *before* the INSERT round-trip completes.  Fresh ingest leaves
    /// this `None` and lets Postgres generate a UUID via
    /// `uuid_generate_v4()`.
    pub pre_assigned_id: Option<Uuid>,
    /// For replay-pass audit rows: the source quarantine row that was
    /// re-validated.  NULL on fresh ingest.  RFC-003.
    pub replay_of_quarantine_id: Option<Uuid>,
    /// RFC-029: traffic direction.  `"ingress"` for all ingest paths;
    /// `"egress"` for the `POST /egress/{contract}` egress-validation path.
    pub direction: String,
}

/// One row to insert into `quarantine_events`.  Only failed events get one.
///
/// `contract_version` is the version that rejected the event.
///
/// `payload` is [`TransformedPayload`] — same invariant as
/// `AuditEntryInsert::raw_event`, RFC-004 §6.
#[derive(Debug, Clone)]
pub struct QuarantineEventInsert {
    pub contract_id: Uuid,
    pub contract_version: String,
    pub payload: TransformedPayload,
    pub violation_count: i32,
    pub violation_details: serde_json::Value,
    pub validation_us: i64,
    pub source_ip: Option<String>,
    /// For quarantine rows created by a *failed* replay attempt: the
    /// source quarantine row whose payload we re-validated.  NULL for
    /// ingest-time quarantine rows.  RFC-003.
    pub replay_of_quarantine_id: Option<Uuid>,
    /// Pre-assigned UUID for the quarantine row.  `None` → Postgres generates
    /// via `uuid_generate_v4()`.  Set by RFC-021 v1 ingest handler so callers
    /// can return the quarantine ID in the response without a `SELECT` round-trip.
    pub pre_assigned_id: Option<Uuid>,
    /// RFC-029: traffic direction.  `"ingress"` for all ingest paths;
    /// `"egress"` for the `POST /egress/{contract}` egress-validation path.
    pub direction: String,
}

/// One row to insert into `forwarded_events`.  Only passing events get one.
///
/// `contract_version` is the version that accepted the event.
///
/// `payload` is [`TransformedPayload`] so the forward destination also
/// only ever sees the post-transform form (RFC-004 §6).
#[derive(Debug, Clone)]
pub struct ForwardEventInsert {
    pub contract_id: Uuid,
    pub contract_version: String,
    pub payload: TransformedPayload,
}

/// Batch-insert audit log entries in a single round-trip.
///
/// Uses `UNNEST` of typed arrays so one SQL statement handles the whole batch
/// regardless of size.  Rows in the input slice keep their order; the
/// database assigns each a fresh UUID via `uuid_generate_v4()`.
///
/// RFC-086: `store_payloads` gates the event body. When `false`, every row's
/// `raw_event` is written as `'{}'` and `raw_event_redacted` is set — the audit
/// record (contract, version, pass/fail, violations, timing) is retained, the
/// source body is not. A single ingest batch is one contract/org, so the
/// decision is uniform across the batch.
pub async fn log_audit_entries_batch(
    pool: &PgPool,
    entries: &[AuditEntryInsert],
    store_payloads: bool,
) -> AppResult<()> {
    if entries.is_empty() {
        return Ok(());
    }

    // Split the struct-of-arrays columns for UNNEST.  Columns are aligned
    // positionally — every Vec is the same length as `entries`.
    let contract_ids: Vec<Uuid> = entries.iter().map(|e| e.contract_id).collect();
    // org_id: None entries use nil UUID as sentinel; NULLIF converts to SQL NULL.
    let org_ids: Vec<Uuid> = entries
        .iter()
        .map(|e| e.org_id.unwrap_or(Uuid::nil()))
        .collect();
    let contract_versions: Vec<String> =
        entries.iter().map(|e| e.contract_version.clone()).collect();
    let passed: Vec<bool> = entries.iter().map(|e| e.passed).collect();
    let violation_counts: Vec<i32> = entries.iter().map(|e| e.violation_count).collect();
    let violation_details: Vec<serde_json::Value> = entries
        .iter()
        .map(|e| e.violation_details.clone())
        .collect();
    // RFC-004 §6: extract the underlying JSON only at the SQL bind boundary.
    // RFC-086: when storage is gated off, redact the body to `'{}'` and mark it.
    let raw_events: Vec<serde_json::Value> = entries
        .iter()
        .map(|e| {
            if store_payloads {
                e.raw_event.as_value().clone()
            } else {
                serde_json::json!({})
            }
        })
        .collect();
    let raw_event_redacted: Vec<bool> = vec![!store_payloads; entries.len()];
    let validation_us: Vec<i64> = entries.iter().map(|e| e.validation_us).collect();
    let source_ips: Vec<String> = entries
        .iter()
        .map(|e| e.source_ip.clone().unwrap_or_default())
        .collect();
    // RFC-025: 'http' for REST ingest, 'kafka' for platform-side consumer.
    let sources: Vec<String> = entries.iter().map(|e| e.source.clone()).collect();
    let pre_assigned_ids: Vec<Uuid> = entries
        .iter()
        .map(|e| e.pre_assigned_id.unwrap_or(Uuid::nil()))
        .collect();
    let replay_of: Vec<Uuid> = entries
        .iter()
        .map(|e| e.replay_of_quarantine_id.unwrap_or(Uuid::nil()))
        .collect();
    // RFC-029: 'ingress' for all ingest paths, 'egress' for egress validation.
    let directions: Vec<String> = entries.iter().map(|e| e.direction.clone()).collect();

    sqlx::query(
        r#"
        INSERT INTO audit_log
            (id, contract_id, org_id, contract_version, passed, violation_count,
             violation_details, raw_event, validation_us, source_ip, source,
             replay_of_quarantine_id, direction, raw_event_redacted, created_at)
        SELECT
            COALESCE(NULLIF(pre_assigned_id, '00000000-0000-0000-0000-000000000000'::uuid),
                     uuid_generate_v4()),
            contract_id,
            NULLIF(org_id, '00000000-0000-0000-0000-000000000000'::uuid),
            contract_version, passed, violation_count,
            violation_details, raw_event, validation_us,
            NULLIF(source_ip, ''),
            source,
            NULLIF(replay_of_quarantine_id, '00000000-0000-0000-0000-000000000000'::uuid),
            direction,
            raw_event_redacted,
            NOW()
        FROM UNNEST(
            $1::uuid[], $2::uuid[], $3::text[], $4::bool[], $5::int[], $6::jsonb[],
            $7::jsonb[], $8::bigint[], $9::text[], $10::text[], $11::uuid[], $12::uuid[],
            $13::text[], $14::bool[]
        ) AS t(contract_id, org_id, contract_version, passed, violation_count,
               violation_details, raw_event, validation_us, source_ip, source,
               pre_assigned_id, replay_of_quarantine_id, direction, raw_event_redacted)
        "#,
    )
    .bind(&contract_ids)
    .bind(&org_ids)
    .bind(&contract_versions)
    .bind(&passed)
    .bind(&violation_counts)
    .bind(&violation_details)
    .bind(&raw_events)
    .bind(&validation_us)
    .bind(&source_ips)
    .bind(&sources)
    .bind(&pre_assigned_ids)
    .bind(&replay_of)
    .bind(&directions)
    .bind(&raw_event_redacted)
    .execute(pool)
    .await
    .db_op("log_audit_entries_batch")?;

    Ok(())
}

/// Batch-insert quarantine entries in a single round-trip.
///
/// RFC-086: `store_payloads` gates the event body. When `false`, every row's
/// `payload` is written as SQL `NULL` and `payload_redacted` is set — the
/// quarantine record (contract, version, violations, timing) is retained but
/// the source body is not, so the row is non-replayable. One ingest batch is
/// one contract/org, so the decision is uniform across the batch.
pub async fn quarantine_events_batch(
    pool: &PgPool,
    entries: &[QuarantineEventInsert],
    store_payloads: bool,
) -> AppResult<()> {
    if entries.is_empty() {
        return Ok(());
    }

    let contract_ids: Vec<Uuid> = entries.iter().map(|e| e.contract_id).collect();
    let contract_versions: Vec<String> =
        entries.iter().map(|e| e.contract_version.clone()).collect();
    // RFC-004 §6: same pattern as `log_audit_entries_batch` — unwrap to
    // `Value` only at the SQL boundary. RFC-086: NULL body when gated off.
    let payloads: Vec<Option<serde_json::Value>> = entries
        .iter()
        .map(|e| store_payloads.then(|| e.payload.as_value().clone()))
        .collect();
    let payload_redacted: Vec<bool> = vec![!store_payloads; entries.len()];
    let violation_counts: Vec<i32> = entries.iter().map(|e| e.violation_count).collect();
    let violation_details: Vec<serde_json::Value> = entries
        .iter()
        .map(|e| e.violation_details.clone())
        .collect();
    let validation_us: Vec<i64> = entries.iter().map(|e| e.validation_us).collect();
    let source_ips: Vec<String> = entries
        .iter()
        .map(|e| e.source_ip.clone().unwrap_or_default())
        .collect();
    // Failed-replay rows link back to their source quarantine row; fresh
    // ingest rows pass NULL (sentinel zero UUID → NULLIF).
    let replay_of: Vec<Uuid> = entries
        .iter()
        .map(|e| e.replay_of_quarantine_id.unwrap_or(Uuid::nil()))
        .collect();
    // RFC-021: pre-assigned IDs let the v1 handler return quarantine UUIDs
    // in the HTTP response without a round-trip SELECT.  None → nil UUID →
    // COALESCE picks uuid_generate_v4() (matches prior behaviour).
    let pre_assigned_ids: Vec<Uuid> = entries
        .iter()
        .map(|e| e.pre_assigned_id.unwrap_or(Uuid::nil()))
        .collect();
    // RFC-029: 'ingress' for all ingest paths, 'egress' for egress validation.
    let directions: Vec<String> = entries.iter().map(|e| e.direction.clone()).collect();

    sqlx::query(
        r#"
        INSERT INTO quarantine_events
            (id, contract_id, contract_version, payload, violation_count,
             violation_details, validation_us, source_ip,
             replay_of_quarantine_id, direction, payload_redacted, status, created_at)
        SELECT
            COALESCE(NULLIF(pre_assigned_id, '00000000-0000-0000-0000-000000000000'::uuid),
                     uuid_generate_v4()),
            contract_id, contract_version, payload, violation_count,
            violation_details, validation_us,
            NULLIF(source_ip, ''),
            NULLIF(replay_of_quarantine_id, '00000000-0000-0000-0000-000000000000'::uuid),
            direction,
            payload_redacted,
            'pending',
            NOW()
        FROM UNNEST(
            $1::uuid[], $2::text[], $3::jsonb[], $4::int[], $5::jsonb[],
            $6::bigint[], $7::text[], $8::uuid[], $9::uuid[], $10::text[], $11::bool[]
        ) AS t(contract_id, contract_version, payload, violation_count,
               violation_details, validation_us, source_ip,
               replay_of_quarantine_id, pre_assigned_id, direction, payload_redacted)
        "#,
    )
    .bind(&contract_ids)
    .bind(&contract_versions)
    .bind(&payloads)
    .bind(&violation_counts)
    .bind(&violation_details)
    .bind(&validation_us)
    .bind(&source_ips)
    .bind(&replay_of)
    .bind(&pre_assigned_ids)
    .bind(&directions)
    .bind(&payload_redacted)
    .execute(pool)
    .await
    .db_op("quarantine_events_batch")?;

    Ok(())
}

/// RFC-086: redact stored event bodies **in place** — org-wide when
/// `contract_id` is `None`, or for a single contract when `Some`. Every
/// `audit_log` and `quarantine_events` row and all its metadata is retained;
/// only the body column is nulled and the `*_redacted` marker set. This never
/// deletes a row. Idempotent: already-redacted rows are skipped.
///
/// Returns `(quarantine_rows_redacted, audit_rows_redacted)`.
pub async fn purge_bodies(
    pool: &PgPool,
    org_id: Option<Uuid>,
    contract_id: Option<Uuid>,
) -> AppResult<(u64, u64)> {
    // quarantine_events has no org_id column — scope via the owning contract.
    let q = sqlx::query(
        r#"
        UPDATE quarantine_events qe
        SET payload = NULL, payload_redacted = true
        FROM contracts c
        WHERE qe.contract_id = c.id
          AND ($1::uuid IS NULL OR c.org_id = $1)
          AND ($2::uuid IS NULL OR qe.contract_id = $2)
          AND qe.payload IS NOT NULL
        "#,
    )
    .bind(org_id)
    .bind(contract_id)
    .execute(pool)
    .await
    .db_op("purge_bodies.quarantine")?;

    let a = sqlx::query(
        r#"
        UPDATE audit_log
        SET raw_event = '{}'::jsonb, raw_event_redacted = true
        WHERE ($1::uuid IS NULL OR org_id = $1)
          AND ($2::uuid IS NULL OR contract_id = $2)
          AND raw_event_redacted = false
        "#,
    )
    .bind(org_id)
    .bind(contract_id)
    .execute(pool)
    .await
    .db_op("purge_bodies.audit")?;

    Ok((q.rows_affected(), a.rows_affected()))
}

/// Batch-insert forwarded events in a single round-trip.
///
/// Unlike the other two helpers this one is awaited inline from the ingest
/// handler so the response can mark individual events as `forwarded: true`.
pub async fn forward_events_batch(pool: &PgPool, entries: &[ForwardEventInsert]) -> AppResult<()> {
    if entries.is_empty() {
        return Ok(());
    }

    let contract_ids: Vec<Uuid> = entries.iter().map(|e| e.contract_id).collect();
    let contract_versions: Vec<String> =
        entries.iter().map(|e| e.contract_version.clone()).collect();
    // RFC-004 §6: forward destination only ever sees post-transform payloads.
    let payloads: Vec<serde_json::Value> = entries
        .iter()
        .map(|e| e.payload.as_value().clone())
        .collect();

    sqlx::query(
        r#"
        INSERT INTO forwarded_events
            (id, contract_id, contract_version, payload, created_at)
        SELECT uuid_generate_v4(), contract_id, contract_version, payload, NOW()
        FROM UNNEST($1::uuid[], $2::text[], $3::jsonb[])
            AS t(contract_id, contract_version, payload)
        "#,
    )
    .bind(&contract_ids)
    .bind(&contract_versions)
    .bind(&payloads)
    .execute(pool)
    .await
    .db_op("forward_events_batch")?;

    Ok(())
}
