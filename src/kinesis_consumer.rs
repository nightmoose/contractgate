//! RFC-026: Platform-side Kinesis consumer loop.
//!
//! One consumer task per enabled contract, polling `cg-{contract_id}-raw`
//! via `GetRecords`.  Each record is validated by the existing `validate_event()`
//! engine; the result is routed to the clean or quarantine output stream, and an
//! `audit_log` row is written with `source = 'kinesis'`.
//!
//! **Scaling model** (resolved in RFC-026 §4):
//!   - Standard `GetRecords` (not Enhanced Fan-Out) — 5 reads/s per shard.
//!   - Default shard count: 1 (1 MB/s ingest, stored per-row).
//!   - Idle contracts (no records for 5 min) pause to 1 poll/min to avoid
//!     unnecessary API calls.  The loop resumes normal cadence on next record.
//!
//! **Checkpointing**: last processed sequence number per shard stored in the
//! `kinesis_ingress.last_sequence_numbers` JSONB column.  On restart the
//! consumer resumes from the stored sequence numbers (AFTER_SEQUENCE_NUMBER
//! iterator type) rather than TRIM_HORIZON.
//!
//! **Feature gate**: compiled only with `--features kinesis-ingress`.  A stub
//! `ConsumerPool` is always present so `AppState` compiles without the feature.

// ── Feature-gated implementation ─────────────────────────────────────────────

#[cfg(feature = "kinesis-ingress")]
mod inner {
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use aws_config::BehaviorVersion;
    use aws_sdk_kinesis::{
        config::{Credentials, Region},
        types::ShardIteratorType,
        Client as KinesisClient,
    };
    use base64::{engine::general_purpose::STANDARD as B64, Engine};
    use dashmap::DashMap;
    use serde_json::{json, Value as JsonValue};
    use tokio::task::JoinHandle;
    use uuid::Uuid;

    use crate::{
        kinesis_ingress::{decrypt_secret, stream_clean, stream_quarantine, stream_raw},
        storage::{log_audit_entries_batch, AuditEntryInsert},
        transform::apply_transforms,
        validation::validate,
        AppState,
    };

    // ── Pool ─────────────────────────────────────────────────────────────────

    #[derive(Default, Clone)]
    pub struct ConsumerPool {
        tasks: Arc<DashMap<Uuid, JoinHandle<()>>>,
    }

    impl ConsumerPool {
        pub fn new() -> Self {
            Self::default()
        }

        pub async fn start(&self, state: Arc<AppState>, contract_id: Uuid) {
            if self.tasks.contains_key(&contract_id) {
                return;
            }
            let handle = tokio::spawn(run_consumer(state, contract_id));
            self.tasks.insert(contract_id, handle);
            tracing::info!(contract_id = %contract_id, "kinesis consumer started");
        }

        pub fn stop(&self, contract_id: Uuid) {
            if let Some((_, handle)) = self.tasks.remove(&contract_id) {
                handle.abort();
                tracing::info!(contract_id = %contract_id, "kinesis consumer stopped");
            }
        }

        /// Restore consumers for all enabled contracts on server boot.
        pub async fn restore_all(&self, state: Arc<AppState>) {
            let ids = match sqlx::query_scalar::<_, Uuid>(
                "SELECT contract_id FROM kinesis_ingress \
                 WHERE enabled = TRUE AND disabled_at IS NULL",
            )
            .fetch_all(&state.db)
            .await
            {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!("kinesis_consumer restore_all DB error: {e}");
                    return;
                }
            };

            let count = ids.len();
            for id in ids {
                self.start(Arc::clone(&state), id).await;
            }
            tracing::info!(count, "kinesis consumer pool restored");
        }
    }

    // ── Consumer task ─────────────────────────────────────────────────────────

    async fn run_consumer(state: Arc<AppState>, contract_id: Uuid) {
        if let Err(e) = consumer_loop(state, contract_id).await {
            tracing::error!(
                contract_id = %contract_id,
                error = %e,
                "kinesis consumer exited with error"
            );
        }
    }

    async fn consumer_loop(state: Arc<AppState>, contract_id: Uuid) -> anyhow::Result<()> {
        // Load ingress config from DB.
        let row = crate::kinesis_ingress::get_kinesis_ingress_row(&state.db, contract_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("kinesis_ingress row not found for {contract_id}"))?;

        let region = row.aws_region.clone();
        let access_key_id = row.iam_access_key_id.as_deref().unwrap_or_default();
        let secret_enc = row.iam_secret_enc.as_deref().unwrap_or_default();
        let secret = decrypt_secret(secret_enc)?;
        let raw_stream = stream_raw(contract_id);
        let clean_stream = stream_clean(contract_id);
        let quarantine_stream = stream_quarantine(contract_id);

        // Build a Kinesis client using the per-contract IAM credentials.
        let creds = Credentials::new(
            access_key_id,
            &secret,
            None,
            None,
            "contractgate-kinesis-consumer",
        );
        let sdk_config = aws_sdk_kinesis::config::Builder::new()
            .behavior_version(BehaviorVersion::latest())
            .region(Region::new(region.clone()))
            .credentials_provider(creds)
            .build();
        let kinesis = KinesisClient::from_conf(sdk_config);

        // Discover shards.
        let shards = list_shards(&kinesis, &raw_stream).await?;

        // Build initial shard iterators (resume from checkpoint or TRIM_HORIZON).
        let saved: HashMap<String, String> = row
            .last_sequence_numbers
            .as_ref()
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        let mut shard_iterators: HashMap<String, Option<String>> = HashMap::new();
        for shard_id in &shards {
            let iter = get_shard_iterator(
                &kinesis,
                &raw_stream,
                shard_id,
                saved.get(shard_id.as_str()),
            )
            .await?;
            shard_iterators.insert(shard_id.clone(), Some(iter));
        }

        let idle_threshold = Duration::from_secs(300); // 5 min → slow-poll mode
        let normal_poll = Duration::from_millis(200); // ~5 reads/s per-shard limit
        let idle_poll = Duration::from_secs(60); // 1 req/min when idle

        let mut last_record_time = Instant::now();
        let mut sequence_numbers: HashMap<String, String> = saved;

        loop {
            let any_records = poll_all_shards(
                &state,
                &kinesis,
                contract_id,
                &clean_stream,
                &quarantine_stream,
                &mut shard_iterators,
                &mut sequence_numbers,
            )
            .await?;

            if any_records {
                last_record_time = Instant::now();
                checkpoint(&state.db, contract_id, &sequence_numbers).await;
            }

            let idle = last_record_time.elapsed() > idle_threshold;
            tokio::time::sleep(if idle { idle_poll } else { normal_poll }).await;
        }
    }

    /// Poll every shard once, validate records, route to clean/quarantine.
    /// Returns true if any records were processed.
    async fn poll_all_shards(
        state: &AppState,
        kinesis: &KinesisClient,
        contract_id: Uuid,
        clean_stream: &str,
        quarantine_stream: &str,
        shard_iterators: &mut HashMap<String, Option<String>>,
        sequence_numbers: &mut HashMap<String, String>,
    ) -> anyhow::Result<bool> {
        let mut any_records = false;

        // Resolve latest stable version once per poll cycle.
        let version_row =
            match crate::storage::get_latest_stable_version(&state.db, contract_id).await {
                Ok(Some(v)) => v,
                Ok(None) => {
                    tracing::warn!(contract_id = %contract_id, "no stable version; skipping poll");
                    return Ok(false);
                }
                Err(e) => {
                    tracing::error!(contract_id = %contract_id, "version lookup: {e}");
                    return Ok(false);
                }
            };

        let compiled = match state.get_compiled(contract_id, &version_row.version).await {
            Ok(c) => c,
            Err(e) => {
                tracing::error!(contract_id = %contract_id, "compile: {e}");
                return Ok(false);
            }
        };

        for (shard_id, maybe_iter) in shard_iterators.iter_mut() {
            let iter = match maybe_iter {
                Some(i) => i.clone(),
                None => continue, // shard exhausted (shouldn't happen on live streams)
            };

            let resp = kinesis
                .get_records()
                .shard_iterator(&iter)
                .limit(100)
                .send()
                .await;

            let resp = match resp {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!(shard_id, error = %e, "GetRecords failed; will retry");
                    continue;
                }
            };

            // Advance iterator for next poll.
            *maybe_iter = resp.next_shard_iterator().map(|s| s.to_string());

            let records = resp.records();
            if records.is_empty() {
                continue;
            }
            any_records = true;

            let mut audit_entries: Vec<AuditEntryInsert> = Vec::with_capacity(records.len());

            for record in records {
                let seq = record.sequence_number().to_string();
                let data = record.data().as_ref();

                // Parse JSON; quarantine immediately on parse failure.
                let event: JsonValue = match serde_json::from_slice(data) {
                    Ok(v) => v,
                    Err(e) => {
                        tracing::warn!(seq, error = %e, "non-JSON kinesis record; quarantining");
                        route_quarantine(
                            kinesis,
                            quarantine_stream,
                            data,
                            "invalid JSON",
                            "unknown",
                        )
                        .await;
                        continue;
                    }
                };

                // Time + validate.
                let t0 = std::time::Instant::now();
                let result = validate(&compiled, &event);
                let validation_us = t0.elapsed().as_micros() as i64;

                // Apply PII transforms (RFC-004) after validation.
                let transformed = apply_transforms(&compiled, event.clone());

                if result.passed {
                    route_clean(kinesis, clean_stream, &transformed.as_value().clone()).await;
                } else {
                    let reason = result
                        .violations
                        .first()
                        .map(|v| v.message.as_str())
                        .unwrap_or("validation failed")
                        .to_string();
                    route_quarantine(
                        kinesis,
                        quarantine_stream,
                        &serde_json::to_vec(&event).unwrap_or_default(),
                        &reason,
                        &version_row.version,
                    )
                    .await;
                }

                // Audit entry — contract_version = matched version (audit honesty).
                audit_entries.push(AuditEntryInsert {
                    contract_id,
                    org_id: None, // consumer runs platform-side; no HTTP API key context
                    contract_version: version_row.version.clone(),
                    passed: result.passed,
                    violation_count: result.violations.len() as i32,
                    violation_details: serde_json::to_value(&result.violations).unwrap_or_default(),
                    raw_event: transformed,
                    validation_us,
                    source_ip: None,
                    source: "kinesis".to_string(),
                    pre_assigned_id: None,
                    replay_of_quarantine_id: None,
                });

                sequence_numbers.insert(shard_id.clone(), seq);
            }

            if !audit_entries.is_empty() {
                if let Err(e) = log_audit_entries_batch(&state.db, &audit_entries).await {
                    tracing::error!(error = %e, "failed to write kinesis audit log");
                }
            }
        }

        Ok(any_records)
    }

    // ── Route helpers ─────────────────────────────────────────────────────────

    async fn route_clean(kinesis: &KinesisClient, stream: &str, payload: &JsonValue) {
        let data = match serde_json::to_vec(payload) {
            Ok(d) => d,
            Err(e) => {
                tracing::error!(error = %e, "failed to serialize clean record");
                return;
            }
        };
        let _ = kinesis
            .put_record()
            .stream_name(stream)
            .data(aws_sdk_kinesis::primitives::Blob::new(data))
            .partition_key("cg-clean")
            .send()
            .await;
    }

    /// Quarantine envelope: JSON-wraps original bytes + violation reason.
    /// Kinesis has no record-level headers; the envelope is the Kinesis convention.
    async fn route_quarantine(
        kinesis: &KinesisClient,
        stream: &str,
        original_bytes: &[u8],
        violation_reason: &str,
        contract_version: &str,
    ) {
        let original_raw = serde_json::from_slice::<JsonValue>(original_bytes)
            .unwrap_or_else(|_| JsonValue::String(B64.encode(original_bytes)));

        let envelope = json!({
            "cg_violation_reason": violation_reason,
            "cg_contract_version": contract_version,
            "cg_original_payload": original_raw,
        });
        let data = match serde_json::to_vec(&envelope) {
            Ok(d) => d,
            Err(e) => {
                tracing::error!(error = %e, "failed to serialize quarantine envelope");
                return;
            }
        };
        let _ = kinesis
            .put_record()
            .stream_name(stream)
            .data(aws_sdk_kinesis::primitives::Blob::new(data))
            .partition_key("cg-quarantine")
            .send()
            .await;
    }

    // ── Checkpoint ────────────────────────────────────────────────────────────

    async fn checkpoint(pool: &sqlx::PgPool, contract_id: Uuid, seq: &HashMap<String, String>) {
        let val = match serde_json::to_value(seq) {
            Ok(v) => v,
            Err(e) => {
                tracing::error!(error = %e, "failed to serialize checkpoint");
                return;
            }
        };
        let _ = sqlx::query(
            "UPDATE kinesis_ingress \
             SET last_sequence_numbers = $1, updated_at = NOW() \
             WHERE contract_id = $2 AND disabled_at IS NULL",
        )
        .bind(val)
        .bind(contract_id)
        .execute(pool)
        .await;
    }

    // ── Shard helpers ─────────────────────────────────────────────────────────

    async fn list_shards(kinesis: &KinesisClient, stream: &str) -> anyhow::Result<Vec<String>> {
        let resp = kinesis
            .list_shards()
            .stream_name(stream)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("list_shards: {e}"))?;
        Ok(resp
            .shards()
            .iter()
            .map(|s| s.shard_id().to_string())
            .collect())
    }

    async fn get_shard_iterator(
        kinesis: &KinesisClient,
        stream: &str,
        shard_id: &str,
        last_seq: Option<&String>,
    ) -> anyhow::Result<String> {
        let (iter_type, seq) = match last_seq {
            Some(s) => (ShardIteratorType::AfterSequenceNumber, Some(s.as_str())),
            None => (ShardIteratorType::TrimHorizon, None),
        };

        let mut req = kinesis
            .get_shard_iterator()
            .stream_name(stream)
            .shard_id(shard_id)
            .shard_iterator_type(iter_type);

        if let Some(s) = seq {
            req = req.starting_sequence_number(s);
        }

        let resp = req
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("get_shard_iterator: {e}"))?;
        resp.shard_iterator()
            .map(|s| s.to_string())
            .ok_or_else(|| anyhow::anyhow!("no shard iterator returned"))
    }
}

// ── Stub (no feature) — keeps AppState compilable ────────────────────────────

#[cfg(not(feature = "kinesis-ingress"))]
mod inner {
    use crate::AppState;
    use std::sync::Arc;
    use uuid::Uuid;

    #[derive(Default, Clone)]
    pub struct ConsumerPool;

    impl ConsumerPool {
        pub fn new() -> Self {
            Self
        }
        pub async fn start(&self, _state: Arc<AppState>, _contract_id: Uuid) {}
        pub fn stop(&self, _contract_id: Uuid) {}
        pub async fn restore_all(&self, _state: Arc<AppState>) {}
    }
}

pub use inner::ConsumerPool;
