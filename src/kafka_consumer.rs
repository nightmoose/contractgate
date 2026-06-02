//! RFC-025: Platform-side Kafka consumer pool.
//!
//! One `rdkafka` `StreamConsumer` group per enabled contract, subscribed to
//! `cg-{contract_id}-raw`.  Each message is validated by the existing
//! `validate_event()` engine; the result is routed to either the clean or
//! quarantine output topic, and an `audit_log` row is written with
//! `source = 'kafka'`.
//!
//! **Feature gate**: the rdkafka consumer loop is compiled only with
//! `--features kafka-ingress`.  A zero-cost stub is always present so
//! `AppState` compiles without the feature.
//!
//! ## Scaling model
//!
//! Each contract gets one Tokio task driving a `StreamConsumer`.  The consumer
//! is assigned `partition_count` (default 3) partitions by the broker.  Idle
//! polls yield the thread naturally — `StreamConsumer::recv()` is async and
//! wakes only when a message arrives.
//!
//! A `ConsumerPool` wraps a `DashMap<Uuid, JoinHandle<()>>` so the enable/
//! disable handlers can start and stop individual contract consumers without
//! touching others.

// ── Feature-gated implementation ────────────────────────────────────────────

#[cfg(feature = "kafka-ingress")]
mod inner {
    use std::sync::Arc;
    use std::time::Duration;

    use dashmap::DashMap;
    use rdkafka::{
        config::ClientConfig,
        consumer::{Consumer, StreamConsumer},
        message::{Header, Message, OwnedHeaders},
        producer::{FutureProducer, FutureRecord},
    };
    use serde_json::Value;
    use tokio::task::JoinHandle;
    use uuid::Uuid;

    use crate::{
        kafka_ingress::decrypt_secret,
        storage::{log_audit_entries_batch, AuditEntryInsert},
        transform::{apply_transforms, TransformedPayload},
        validation::validate,
        AppState,
    };

    // ── Pool ─────────────────────────────────────────────────────────────────

    /// Shared consumer pool.
    #[derive(Default, Clone)]
    pub struct ConsumerPool {
        tasks: Arc<DashMap<Uuid, JoinHandle<()>>>,
    }

    impl ConsumerPool {
        pub fn new() -> Self {
            Self::default()
        }

        /// Start a consumer for `contract_id` if one is not already running.
        pub async fn start(&self, state: Arc<AppState>, contract_id: Uuid) {
            if self.tasks.contains_key(&contract_id) {
                return;
            }
            let handle = tokio::spawn(run_consumer(state, contract_id));
            self.tasks.insert(contract_id, handle);
            tracing::info!(contract_id = %contract_id, "kafka consumer started");
        }

        /// Stop the consumer for `contract_id` (cancels the Tokio task).
        pub fn stop(&self, contract_id: Uuid) {
            if let Some((_, handle)) = self.tasks.remove(&contract_id) {
                handle.abort();
                tracing::info!(contract_id = %contract_id, "kafka consumer stopped");
            }
        }

        /// Re-start consumers for all contracts that have ingress enabled.
        /// Called on server boot so consumers survive restarts.
        pub async fn restore_all(&self, state: Arc<AppState>) {
            let ids = match sqlx::query_scalar::<_, Uuid>(
                "SELECT contract_id FROM kafka_ingress \
                 WHERE enabled = TRUE AND disabled_at IS NULL",
            )
            .fetch_all(&state.db)
            .await
            {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!("kafka_consumer restore_all DB error: {e}");
                    return;
                }
            };

            for id in ids {
                self.start(Arc::clone(&state), id).await;
            }
        }
    }

    // ── Consumer task ────────────────────────────────────────────────────────

    async fn run_consumer(state: Arc<AppState>, contract_id: Uuid) {
        loop {
            // Reload config on each (re)start so credential rotations are picked up.
            let row = match crate::kafka_ingress::get_kafka_ingress_row(&state.db, contract_id)
                .await
            {
                Ok(Some(r)) => r,
                Ok(None) => {
                    tracing::info!(contract_id = %contract_id, "kafka ingress row gone; consumer exiting");
                    return;
                }
                Err(e) => {
                    tracing::error!(contract_id = %contract_id, "DB error loading ingress config: {e}; retry in 30s");
                    tokio::time::sleep(Duration::from_secs(30)).await;
                    continue;
                }
            };

            let api_secret = match decrypt_secret(&row.confluent_api_secret_enc) {
                Ok(s) => s,
                Err(e) => {
                    tracing::error!(contract_id = %contract_id, "decrypt error: {e}; retry in 30s");
                    tokio::time::sleep(Duration::from_secs(30)).await;
                    continue;
                }
            };

            let raw_topic = format!("cg-{}-raw", contract_id);
            let group_id = format!("cg-consumer-{}", contract_id);

            let consumer: StreamConsumer = match ClientConfig::new()
                .set("bootstrap.servers", &row.confluent_bootstrap)
                .set("security.protocol", "SASL_SSL")
                .set("sasl.mechanisms", "PLAIN")
                .set("sasl.username", &row.confluent_api_key)
                .set("sasl.password", &api_secret)
                .set("group.id", &group_id)
                .set("auto.offset.reset", "earliest")
                .set("enable.auto.commit", "true")
                .create()
            {
                Ok(c) => c,
                Err(e) => {
                    tracing::error!(contract_id = %contract_id, "consumer create error: {e}; retry in 30s");
                    tokio::time::sleep(Duration::from_secs(30)).await;
                    continue;
                }
            };

            if let Err(e) = consumer.subscribe(&[raw_topic.as_str()]) {
                tracing::error!(contract_id = %contract_id, "subscribe error: {e}; retry in 30s");
                tokio::time::sleep(Duration::from_secs(30)).await;
                continue;
            }

            let producer: FutureProducer = match ClientConfig::new()
                .set("bootstrap.servers", &row.confluent_bootstrap)
                .set("security.protocol", "SASL_SSL")
                .set("sasl.mechanisms", "PLAIN")
                .set("sasl.username", &row.confluent_api_key)
                .set("sasl.password", &api_secret)
                .set("message.timeout.ms", "5000")
                .create()
            {
                Ok(p) => p,
                Err(e) => {
                    tracing::error!(contract_id = %contract_id, "producer create error: {e}; retry in 30s");
                    tokio::time::sleep(Duration::from_secs(30)).await;
                    continue;
                }
            };

            tracing::info!(contract_id = %contract_id, topic = %raw_topic, "consumer polling");

            loop {
                match consumer.recv().await {
                    Err(e) => {
                        tracing::warn!(contract_id = %contract_id, "recv error: {e}; reconnecting in 10s");
                        tokio::time::sleep(Duration::from_secs(10)).await;
                        break; // outer loop reconnects with fresh credentials
                    }
                    Ok(msg) => {
                        let payload_bytes = match msg.payload() {
                            Some(b) => b.to_vec(),
                            None => continue,
                        };
                        process_message(&state, &producer, contract_id, payload_bytes).await;
                    }
                }
            }
        }
    }

    // ── Message processing ───────────────────────────────────────────────────

    async fn process_message(
        state: &AppState,
        producer: &FutureProducer,
        contract_id: Uuid,
        payload_bytes: Vec<u8>,
    ) {
        // 1. Parse JSON — route to quarantine on failure.
        let event: Value = match serde_json::from_slice(&payload_bytes) {
            Ok(v) => v,
            Err(e) => {
                let reason = format!("json_parse_error: {e}");
                route_to_quarantine_raw(producer, contract_id, &payload_bytes, &reason).await;
                return;
            }
        };

        // 2. Resolve contract version (latest stable).
        let version_row =
            match crate::storage::get_latest_stable_version(&state.db, contract_id).await {
                Ok(Some(v)) => v,
                Ok(None) => {
                    tracing::warn!(contract_id = %contract_id, "no stable version; skipping");
                    return;
                }
                Err(e) => {
                    tracing::error!(contract_id = %contract_id, "version lookup: {e}");
                    return;
                }
            };

        let compiled = match state.get_compiled(contract_id, &version_row.version).await {
            Ok(c) => c,
            Err(e) => {
                tracing::error!(contract_id = %contract_id, "compile: {e}");
                return;
            }
        };

        // 3. Validate — same function the HTTP path calls; <15ms p99.
        let t0 = std::time::Instant::now();
        let result = validate(&compiled, &event);
        let validation_us = t0.elapsed().as_micros() as i64;

        // 4. Apply transforms (RFC-004).
        let transformed: TransformedPayload = apply_transforms(&compiled, event);

        // 5. Route to clean or quarantine.
        let output_topic = if result.passed {
            format!("cg-{}-clean", contract_id)
        } else {
            format!("cg-{}-quarantine", contract_id)
        };

        let payload_str = transformed.as_value().to_string();
        let violation_json = serde_json::to_string(&result.violations).unwrap_or_default();

        let mut headers = OwnedHeaders::new()
            .insert(Header {
                key: "cg-contract-id",
                value: Some(contract_id.to_string().as_bytes()),
            })
            .insert(Header {
                key: "cg-contract-version",
                value: Some(version_row.version.as_bytes()),
            });

        if !result.passed {
            headers = headers.insert(Header {
                key: "cg-violation-reason",
                value: Some(violation_json.as_bytes()),
            });
        }

        let _ = producer
            .send(
                FutureRecord::to(&output_topic)
                    .payload(payload_str.as_bytes())
                    .key("")
                    .headers(headers),
                Duration::from_secs(5),
            )
            .await;

        // 6. Write audit log — source = 'kafka', version = matched version (audit honesty).
        let audit = AuditEntryInsert {
            contract_id,
            org_id: None, // consumer runs platform-side; no HTTP API key context
            contract_version: version_row.version.clone(),
            passed: result.passed,
            violation_count: result.violations.len() as i32,
            violation_details: serde_json::to_value(&result.violations).unwrap_or_default(),
            raw_event: transformed,
            validation_us,
            source_ip: None,
            source: "kafka".to_string(),
            pre_assigned_id: None,
            replay_of_quarantine_id: None,
            direction: "ingress".to_string(),
        };

        if let Err(e) = log_audit_entries_batch(&state.db, &[audit]).await {
            tracing::error!(contract_id = %contract_id, "audit log write: {e}");
        }
    }

    /// Route raw (unparseable) bytes to quarantine with a cg-violation-reason header.
    async fn route_to_quarantine_raw(
        producer: &FutureProducer,
        contract_id: Uuid,
        payload_bytes: &[u8],
        reason: &str,
    ) {
        let topic = format!("cg-{}-quarantine", contract_id);
        let headers = OwnedHeaders::new().insert(Header {
            key: "cg-violation-reason",
            value: Some(reason.as_bytes()),
        });
        let _ = producer
            .send(
                FutureRecord::to(&topic)
                    .payload(payload_bytes)
                    .key("")
                    .headers(headers),
                Duration::from_secs(5),
            )
            .await;
    }
}

// ── Stub (no kafka-ingress feature) ─────────────────────────────────────────

#[cfg(not(feature = "kafka-ingress"))]
mod inner {
    use crate::AppState;
    use std::sync::Arc;
    use uuid::Uuid;

    /// Zero-cost stub — compiles without rdkafka.
    #[derive(Default, Clone)]
    pub struct ConsumerPool;

    impl ConsumerPool {
        pub fn new() -> Self {
            Self
        }
        pub async fn start(&self, _state: Arc<AppState>, _id: Uuid) {}
        pub fn stop(&self, _id: Uuid) {}
        pub async fn restore_all(&self, _state: Arc<AppState>) {}
    }
}

// ── Public re-export ─────────────────────────────────────────────────────────

pub use inner::ConsumerPool;
