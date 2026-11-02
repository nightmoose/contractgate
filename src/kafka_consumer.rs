//! RFC-025: Platform-side Kafka consumer pool.
//!
//! One `rdkafka` `StreamConsumer` group per enabled contract, subscribed to
//! `cg-{contract_id}-raw`.  Each message is validated by the existing
//! `validate_event()` engine; the result is routed to either the clean or
//! quarantine output topic, and an `audit_log` row is written with
//! `source = 'kafka'`. A failure that resolves to drop — contract `on_fail`,
//! an enforcing safety cap, or the Free quarantine cap — is counted in
//! `dropped_event_counts` and is not published. Passing records still go to
//! the clean topic.
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

            // RFC-083 billing integrity: resolve the owning org once per (re)connect
            // so stream audit rows carry org_id. The periodic up-only usage reconcile
            // counts audit rows per-org (audit_log is the source of truth), so streamed
            // events are metered without a per-message counter write. A NULL org_id here
            // makes stream events invisible to /usage (reconcile filters org_id IS NOT NULL).
            let meter_org: Option<Uuid> =
                sqlx::query_scalar::<_, Option<Uuid>>("SELECT org_id FROM contracts WHERE id = $1")
                    .bind(contract_id)
                    .fetch_one(&state.db)
                    .await
                    .unwrap_or(None);

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
                        process_message(&state, &producer, contract_id, meter_org, payload_bytes)
                            .await;
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
        meter_org: Option<Uuid>,
        payload_bytes: Vec<u8>,
    ) {
        // 1. Parse JSON. A parse failure is still a failure for on-fail routing.
        let event: Value = match serde_json::from_slice(&payload_bytes) {
            Ok(v) => v,
            Err(e) => {
                let reason = format!("json_parse_error: {e}");
                route_parse_failure(
                    state,
                    producer,
                    contract_id,
                    meter_org,
                    &payload_bytes,
                    &reason,
                )
                .await;
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

        // 5. Passing records go to the clean topic. Failures follow on-fail:
        //    quarantine publishes, drop counts and does not publish.
        if result.passed {
            let payload_str = transformed.as_value().to_string();
            let topic = format!("cg-{contract_id}-clean");
            publish_kafka(
                producer,
                &topic,
                payload_str.as_bytes(),
                &version_row.version,
                contract_id,
                None,
            )
            .await;
            write_kafka_audit(
                state,
                kafka_audit(
                    contract_id,
                    meter_org,
                    &version_row.version,
                    &result,
                    transformed,
                    validation_us,
                ),
            )
            .await;
            return;
        }

        let policy = crate::stream_fail::load_contract_policy(state, contract_id).await;
        let route = crate::stream_fail::route_failures(
            state,
            meter_org,
            contract_id,
            &compiled.contract,
            &version_row.yaml_content,
            &policy,
            1,
        )
        .await;
        let facts = crate::fail_policy::failure_facts(&result.violations);
        match route {
            crate::stream_fail::FailureRoute::PublishQuarantine => {
                let violation_json = serde_json::to_string(&result.violations).unwrap_or_default();
                let payload_str = transformed.as_value().to_string();
                let topic = format!("cg-{contract_id}-quarantine");
                publish_kafka(
                    producer,
                    &topic,
                    payload_str.as_bytes(),
                    &version_row.version,
                    contract_id,
                    Some(violation_json.as_bytes()),
                )
                .await;
                write_kafka_audit(
                    state,
                    kafka_audit(
                        contract_id,
                        meter_org,
                        &version_row.version,
                        &result,
                        transformed,
                        validation_us,
                    ),
                )
                .await;
                if policy.is_ready() {
                    crate::stream_fail::record_published_quarantine(
                        state,
                        meter_org,
                        contract_id,
                        1,
                    )
                    .await;
                }
            }
            crate::stream_fail::FailureRoute::CountedDrop { source, sample } => {
                crate::stream_fail::record_counted_drops(
                    state,
                    meter_org,
                    contract_id,
                    &version_row.version,
                    source,
                    sample,
                    &[facts],
                )
                .await;
            }
        }
    }

    async fn route_parse_failure(
        state: &AppState,
        producer: &FutureProducer,
        contract_id: Uuid,
        meter_org: Option<Uuid>,
        payload_bytes: &[u8],
        reason: &str,
    ) {
        let version_row = match crate::storage::get_latest_stable_version(&state.db, contract_id)
            .await
        {
            Ok(Some(v)) => v,
            Ok(None) => {
                tracing::warn!(contract_id = %contract_id, "no stable version; quarantining unparsed record");
                route_to_quarantine_raw(producer, contract_id, payload_bytes, reason).await;
                return;
            }
            Err(e) => {
                tracing::error!(contract_id = %contract_id, "version lookup: {e}");
                route_to_quarantine_raw(producer, contract_id, payload_bytes, reason).await;
                return;
            }
        };
        let compiled = match state.get_compiled(contract_id, &version_row.version).await {
            Ok(c) => c,
            Err(e) => {
                tracing::error!(contract_id = %contract_id, "compile: {e}");
                route_to_quarantine_raw(producer, contract_id, payload_bytes, reason).await;
                return;
            }
        };
        let policy = crate::stream_fail::load_contract_policy(state, contract_id).await;
        let route = crate::stream_fail::route_failures(
            state,
            meter_org,
            contract_id,
            &compiled.contract,
            &version_row.yaml_content,
            &policy,
            1,
        )
        .await;
        match route {
            crate::stream_fail::FailureRoute::PublishQuarantine => {
                route_to_quarantine_raw(producer, contract_id, payload_bytes, reason).await;
                if policy.is_ready() {
                    crate::stream_fail::record_published_quarantine(
                        state,
                        meter_org,
                        contract_id,
                        1,
                    )
                    .await;
                }
            }
            crate::stream_fail::FailureRoute::CountedDrop { source, sample } => {
                crate::stream_fail::record_counted_drops(
                    state,
                    meter_org,
                    contract_id,
                    &version_row.version,
                    source,
                    sample,
                    &[crate::stream_fail::parse_failure()],
                )
                .await;
            }
        }
    }

    fn kafka_audit(
        contract_id: Uuid,
        meter_org: Option<Uuid>,
        version: &str,
        result: &crate::validation::ValidationResult,
        transformed: TransformedPayload,
        validation_us: i64,
    ) -> AuditEntryInsert {
        AuditEntryInsert {
            contract_id,
            org_id: meter_org,
            contract_version: version.to_string(),
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
        }
    }

    async fn write_kafka_audit(state: &AppState, audit: AuditEntryInsert) {
        let contract_id = audit.contract_id;
        // Downstream topic publish uses the payload in memory only. This
        // consumer does not insert `forwarded_events`.
        // RFC-086: gate event-body storage for this contract (one indexed read).
        let store_payloads =
            crate::storage::contract_stores_payloads(&state.db, contract_id, state.self_hosted)
                .await
                .unwrap_or(false);
        if let Err(e) = log_audit_entries_batch(&state.db, &[audit], store_payloads).await {
            tracing::error!(contract_id = %contract_id, "audit log write: {e}");
        }
    }

    async fn publish_kafka(
        producer: &FutureProducer,
        topic: &str,
        payload: &[u8],
        version: &str,
        contract_id: Uuid,
        violation_json: Option<&[u8]>,
    ) {
        let mut headers = OwnedHeaders::new()
            .insert(Header {
                key: "cg-contract-id",
                value: Some(contract_id.to_string().as_bytes()),
            })
            .insert(Header {
                key: "cg-contract-version",
                value: Some(version.as_bytes()),
            });
        if let Some(violation_json) = violation_json {
            headers = headers.insert(Header {
                key: "cg-violation-reason",
                value: Some(violation_json),
            });
        }
        let _ = producer
            .send(
                FutureRecord::to(topic)
                    .payload(payload)
                    .key("")
                    .headers(headers),
                Duration::from_secs(5),
            )
            .await;
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

#[cfg(test)]
mod tests {
    use crate::contract::{OnFailMode, OnFailSource};
    use crate::drop_mode::PlanCapCounts;
    use crate::stream_fail::{
        failure_route, free_cap_counts, route_resolved, stream_input, FailureRoute,
    };

    #[test]
    fn kafka_consumer_at_org_cap_counts_the_drop_and_does_not_publish() {
        let route = route_resolved(
            &stream_input(OnFailMode::Quarantine, false),
            free_cap_counts(50_000, 1),
        );
        assert_eq!(
            route,
            FailureRoute::CountedDrop {
                source: OnFailSource::PlanCap,
                sample: false,
            }
        );
        assert!(!route.publishes_quarantine());
    }

    #[test]
    fn kafka_consumer_under_the_cap_still_publishes() {
        let route = route_resolved(
            &stream_input(OnFailMode::Quarantine, false),
            free_cap_counts(49_999, 9_999),
        );
        assert_eq!(route, FailureRoute::PublishQuarantine);
        assert!(route.publishes_quarantine());
    }

    #[test]
    fn kafka_consumer_paid_plan_still_publishes() {
        let uncapped = PlanCapCounts {
            org_pending: 80_000,
            org_limit: None,
            contract_pending: 20_000,
            contract_limit: None,
            near_percent: 80,
        };
        let route = route_resolved(&stream_input(OnFailMode::Quarantine, false), uncapped);
        assert_eq!(route, FailureRoute::PublishQuarantine);
    }

    #[test]
    fn kafka_consumer_drop_mode_counts_and_does_not_publish() {
        let mut input = stream_input(OnFailMode::Drop, true);
        input.contract_guard.config.sample_drops = true;
        let route = route_resolved(&input, free_cap_counts(1, 1));
        assert_eq!(
            route,
            FailureRoute::CountedDrop {
                source: OnFailSource::Contract,
                sample: true,
            }
        );
        assert!(!route.publishes_quarantine());
    }

    #[test]
    fn kafka_consumer_safety_cap_beats_a_full_plan_cap() {
        let mut input = stream_input(OnFailMode::Quarantine, false);
        input.contract_guard.config.enforce = true;
        input.contract_guard.config.max_per_minute = Some(1);
        input.contract_guard.volume.minute = 5;
        let route = route_resolved(&input, free_cap_counts(50_000, 10_000));
        assert_eq!(
            route,
            FailureRoute::CountedDrop {
                source: OnFailSource::AutoTrip,
                sample: false,
            }
        );
    }

    #[test]
    fn kafka_consumer_plan_cap_does_not_keep_drop_samples() {
        let mut input = stream_input(OnFailMode::Quarantine, false);
        input.contract_guard.config.sample_drops = true;
        let route = route_resolved(&input, free_cap_counts(50_000, 0));
        assert_eq!(
            failure_route(&crate::drop_mode::apply_plan_cap(
                crate::drop_mode::resolve_on_fail(&input),
                free_cap_counts(50_000, 0),
            )),
            route
        );
        assert_eq!(
            route,
            FailureRoute::CountedDrop {
                source: OnFailSource::PlanCap,
                sample: false,
            }
        );
    }
}
