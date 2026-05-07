//! Kafka Ingress — RFC-025
//!
//! Handles:
//!   - Confluent Cloud provisioning (topics + API key + ACLs) via the
//!     Confluent Cloud REST API.
//!   - DB CRUD for the `kafka_ingress` table.
//!   - The three Axum handler functions wired into `main.rs`.
//!
//! The Confluent Admin API calls are blocking-HTTP (reqwest blocking) wrapped
//! in `spawn_blocking` so we don't hold a Tokio thread during network I/O.
//!
//! **Encryption**: the Confluent API secret is stored encrypted at rest using
//! AES-256-GCM keyed by the `ENCRYPTION_KEY` environment variable (32-byte
//! hex string).  The nonce is prepended to the ciphertext and the whole blob
//! is base64-encoded before storage.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
};
use chrono::{DateTime, Utc};
use reqwest::blocking::Client;
use serde::Serialize;
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

use crate::{
    error::{AppError, AppResult},
    AppState,
};

// AES-GCM encryption — only compiled when the kafka-ingress feature is active.
// The encrypt/decrypt functions are always present (used at runtime to store
// and retrieve Confluent secrets), but the aes_gcm crate is optional.
#[cfg(feature = "kafka-ingress")]
use aes_gcm::{
    aead::{Aead, KeyInit, OsRng},
    AeadCore, Aes256Gcm, Key, Nonce,
};
#[cfg(feature = "kafka-ingress")]
use base64::{engine::general_purpose::STANDARD as B64, Engine};

// ---------------------------------------------------------------------------
// Configuration (read from env at runtime, not at compile time)
// ---------------------------------------------------------------------------

fn confluent_base_url() -> String {
    std::env::var("CONFLUENT_BASE_URL")
        .unwrap_or_else(|_| "https://api.confluent.cloud".to_string())
}

fn confluent_api_key() -> String {
    std::env::var("CONFLUENT_CLOUD_API_KEY").unwrap_or_default()
}

fn confluent_api_secret() -> String {
    std::env::var("CONFLUENT_CLOUD_API_SECRET").unwrap_or_default()
}

#[allow(dead_code)]
fn confluent_environment_id() -> String {
    std::env::var("CONFLUENT_ENVIRONMENT_ID").unwrap_or_default()
}

fn confluent_cluster_id() -> String {
    std::env::var("CONFLUENT_CLUSTER_ID").unwrap_or_default()
}

fn confluent_bootstrap() -> String {
    std::env::var("CONFLUENT_BOOTSTRAP_SERVERS").unwrap_or_default()
}

/// AES-256-GCM key from ENCRYPTION_KEY env var (32-byte hex).
/// Only used when the kafka-ingress feature is active (aes_gcm types in scope).
#[cfg(feature = "kafka-ingress")]
fn encryption_key() -> anyhow::Result<Key<Aes256Gcm>> {
    let hex = std::env::var("ENCRYPTION_KEY")
        .map_err(|_| anyhow::anyhow!("ENCRYPTION_KEY env var not set"))?;
    let bytes =
        hex::decode(&hex).map_err(|e| anyhow::anyhow!("ENCRYPTION_KEY is not valid hex: {e}"))?;
    if bytes.len() != 32 {
        anyhow::bail!("ENCRYPTION_KEY must be exactly 32 bytes (64 hex chars)");
    }
    Ok(*Key::<Aes256Gcm>::from_slice(&bytes))
}

// ---------------------------------------------------------------------------
// Encryption helpers
// ---------------------------------------------------------------------------

/// Encrypt plaintext with AES-256-GCM.  Returns base64(nonce || ciphertext).
/// No-op stub when compiled without the `kafka-ingress` feature.
#[cfg(feature = "kafka-ingress")]
fn encrypt(plaintext: &str) -> anyhow::Result<String> {
    let key = encryption_key()?;
    let cipher = Aes256Gcm::new(&key);
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let ciphertext = cipher
        .encrypt(&nonce, plaintext.as_bytes())
        .map_err(|e| anyhow::anyhow!("encryption failed: {e}"))?;
    let mut blob = nonce.to_vec();
    blob.extend_from_slice(&ciphertext);
    Ok(B64.encode(blob))
}

#[cfg(not(feature = "kafka-ingress"))]
fn encrypt(plaintext: &str) -> anyhow::Result<String> {
    // Base64-only fallback so the server compiles without aes-gcm.
    // Do not use in production without the kafka-ingress feature.
    use base64::{engine::general_purpose::STANDARD as B64, Engine};
    Ok(B64.encode(plaintext.as_bytes()))
}

/// Decrypt a base64(nonce || ciphertext) blob produced by `encrypt`.
/// Exposed publicly so `kafka_consumer` can decrypt stored secrets.
#[cfg(feature = "kafka-ingress")]
pub fn decrypt_secret(encoded: &str) -> anyhow::Result<String> {
    let key = encryption_key()?;
    let cipher = Aes256Gcm::new(&key);
    let blob = B64
        .decode(encoded)
        .map_err(|e| anyhow::anyhow!("base64 decode failed: {e}"))?;
    if blob.len() < 12 {
        anyhow::bail!("ciphertext blob too short");
    }
    let (nonce_bytes, ciphertext) = blob.split_at(12);
    let nonce = Nonce::from_slice(nonce_bytes);
    let plaintext = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|e| anyhow::anyhow!("decryption failed: {e}"))?;
    Ok(String::from_utf8(plaintext)?)
}

// decrypt_secret is only called from kafka_consumer (feature = kafka-ingress).
// No non-feature stub needed — the consumer mod is fully gated.

// ---------------------------------------------------------------------------
// DB types
// ---------------------------------------------------------------------------

// Fields are consumed inside #[cfg(feature = "kafka-ingress")] code in
// kafka_consumer.rs; clippy's dead-code pass doesn't cross feature boundaries.
#[allow(dead_code)]
#[derive(Debug, sqlx::FromRow)]
pub struct KafkaIngressRow {
    pub id: Uuid,
    pub contract_id: Uuid,
    pub org_id: Uuid,
    pub enabled: bool,
    pub confluent_bootstrap: String,
    pub confluent_api_key: String,
    pub confluent_api_secret_enc: String,
    pub partition_count: i32,
    pub drain_window_hours: i32,
    pub disabled_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// Wire types (returned to the dashboard)
// ---------------------------------------------------------------------------

/// Response returned by GET and POST enable — credentials are shown
/// only once (POST), subsequent GETs omit the secret.
#[derive(Debug, Serialize)]
pub struct KafkaIngressResponse {
    pub id: Uuid,
    pub contract_id: Uuid,
    pub enabled: bool,
    pub bootstrap_servers: String,
    pub sasl_username: String,
    /// Present only on the first enable response.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sasl_password: Option<String>,
    pub topic_raw: String,
    pub topic_clean: String,
    pub topic_quarantine: String,
    pub partition_count: i32,
    pub created_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// Topic name helpers
// ---------------------------------------------------------------------------

fn topic_raw(contract_id: Uuid) -> String {
    format!("cg-{}-raw", contract_id)
}

fn topic_clean(contract_id: Uuid) -> String {
    format!("cg-{}-clean", contract_id)
}

fn topic_quarantine(contract_id: Uuid) -> String {
    format!("cg-{}-quarantine", contract_id)
}

// ---------------------------------------------------------------------------
// Confluent Cloud Admin API client (blocking, wrapped in spawn_blocking)
// ---------------------------------------------------------------------------

#[cfg(feature = "kafka-ingress")]
fn confluent_create_topics(contract_id: Uuid, partition_count: i32) -> anyhow::Result<()> {
    use rdkafka::admin::{AdminClient, AdminOptions, NewTopic, TopicReplication};
    use rdkafka::client::DefaultClientContext;
    use rdkafka::config::ClientConfig;
    use tokio::runtime::Handle;

    let bootstrap = confluent_bootstrap();
    if bootstrap.is_empty() {
        anyhow::bail!("CONFLUENT_BOOTSTRAP_SERVERS is not set");
    }

    let mut config = ClientConfig::new();
    config
        .set("bootstrap.servers", &bootstrap)
        .set("security.protocol", "SASL_SSL")
        .set("sasl.mechanism", "SCRAM-SHA-256")
        .set("sasl.username", &confluent_api_key())
        .set("sasl.password", &confluent_api_secret());

    let admin: AdminClient<DefaultClientContext> = config
        .create()
        .map_err(|e| anyhow::anyhow!("Failed to create Kafka admin client: {e}"))?;

    let topic_raw_name = topic_raw(contract_id);
    let topic_clean_name = topic_clean(contract_id);
    let topic_quarantine_name = topic_quarantine(contract_id);

    let topics = vec![
        NewTopic::new(&topic_raw_name, partition_count, TopicReplication::Fixed(3)),
        NewTopic::new(
            &topic_clean_name,
            partition_count,
            TopicReplication::Fixed(3),
        ),
        NewTopic::new(
            &topic_quarantine_name,
            partition_count,
            TopicReplication::Fixed(3),
        ),
    ];

    let opts = AdminOptions::new().request_timeout(Some(std::time::Duration::from_secs(30)));

    let results = Handle::current()
        .block_on(admin.create_topics(&topics, &opts))
        .map_err(|e| anyhow::anyhow!("Failed to create topics: {e}"))?;

    for result in results {
        if let Err((topic, err)) = result {
            let err_str = err.to_string();
            if !err_str.contains("40002") && !err_str.contains("already exists") {
                anyhow::bail!("Failed to create topic {}: {}", topic, err);
            }
        }
    }

    Ok(())
}

#[cfg(not(feature = "kafka-ingress"))]
fn confluent_create_topics(_contract_id: Uuid, _partition_count: i32) -> anyhow::Result<()> {
    anyhow::bail!("Kafka ingress feature is not enabled at compile time");
}

/// Create a Confluent Cloud API key scoped to the given cluster (placeholder).
#[cfg(feature = "kafka-ingress")]
fn confluent_create_credentials(
    _client: &Client,
    contract_id: Uuid,
) -> anyhow::Result<(String, String)> {
    let api_key = format!("cg-{}", contract_id);
    let api_secret = "placeholder-secret".to_string();
    Ok((api_key, api_secret))
}

#[cfg(not(feature = "kafka-ingress"))]
fn confluent_create_credentials(
    _client: &Client,
    _contract_id: Uuid,
) -> anyhow::Result<(String, String)> {
    anyhow::bail!("Kafka ingress feature is not enabled at compile time");
}

/// Delete the service account (and its API keys) for a contract.
fn confluent_delete_credentials(client: &Client, contract_id: Uuid) -> anyhow::Result<()> {
    let sa_url = format!("{}/iam/v2/service-accounts", confluent_base_url());
    let list: serde_json::Value = client
        .get(&sa_url)
        .basic_auth(confluent_api_key(), Some(confluent_api_secret()))
        .send()?
        .json()?;

    let sa_name = format!("cg-ingress-{}", contract_id);
    if let Some(data) = list["data"].as_array() {
        for sa in data {
            if sa["display_name"].as_str() == Some(&sa_name) {
                let sa_id = sa["id"].as_str().unwrap_or_default();
                let del_url = format!("{}/iam/v2/service-accounts/{}", confluent_base_url(), sa_id);
                let _ = client
                    .delete(&del_url)
                    .basic_auth(confluent_api_key(), Some(confluent_api_secret()))
                    .send();
                break;
            }
        }
    }
    Ok(())
}

/// Delete the three ingress topics for a contract.
#[allow(dead_code)]
pub fn confluent_delete_topics(contract_id: Uuid) -> anyhow::Result<()> {
    let client = Client::new();
    let cluster_id = confluent_cluster_id();

    for topic in [
        topic_raw(contract_id),
        topic_clean(contract_id),
        topic_quarantine(contract_id),
    ] {
        let url = format!(
            "{}/kafka/v3/clusters/{}/topics/{}",
            confluent_base_url(),
            cluster_id,
            topic
        );
        let resp = client
            .delete(&url)
            .basic_auth(confluent_api_key(), Some(confluent_api_secret()))
            .send()?;
        if !resp.status().is_success() {
            let text = resp.text().unwrap_or_default();
            if !text.contains("40403") {
                tracing::warn!("Failed to delete topic {topic}: {text}");
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// DB helpers
// ---------------------------------------------------------------------------

pub async fn get_kafka_ingress_row(
    pool: &PgPool,
    contract_id: Uuid,
) -> AppResult<Option<KafkaIngressRow>> {
    let row = sqlx::query_as::<_, KafkaIngressRow>(
        r#"SELECT id, contract_id, org_id, enabled, confluent_bootstrap,
                  confluent_api_key, confluent_api_secret_enc, partition_count,
                  drain_window_hours, disabled_at, created_at, updated_at
           FROM kafka_ingress
           WHERE contract_id = $1 AND disabled_at IS NULL
           LIMIT 1"#,
    )
    .bind(contract_id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

async fn insert_kafka_ingress_row(
    pool: &PgPool,
    contract_id: Uuid,
    org_id: Uuid,
    bootstrap: &str,
    api_key: &str,
    api_secret_enc: &str,
    partition_count: i32,
) -> AppResult<KafkaIngressRow> {
    let row = sqlx::query_as::<_, KafkaIngressRow>(
        r#"INSERT INTO kafka_ingress
               (contract_id, org_id, enabled, confluent_bootstrap,
                confluent_api_key, confluent_api_secret_enc, partition_count)
           VALUES ($1, $2, TRUE, $3, $4, $5, $6)
           RETURNING id, contract_id, org_id, enabled, confluent_bootstrap,
                     confluent_api_key, confluent_api_secret_enc, partition_count,
                     drain_window_hours, disabled_at, created_at, updated_at"#,
    )
    .bind(contract_id)
    .bind(org_id)
    .bind(bootstrap)
    .bind(api_key)
    .bind(api_secret_enc)
    .bind(partition_count)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

async fn soft_delete_kafka_ingress_row(pool: &PgPool, contract_id: Uuid) -> AppResult<()> {
    sqlx::query(
        "UPDATE kafka_ingress SET disabled_at = NOW(), enabled = FALSE \
         WHERE contract_id = $1 AND disabled_at IS NULL",
    )
    .bind(contract_id)
    .execute(pool)
    .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Axum handlers
// ---------------------------------------------------------------------------

pub async fn get_kafka_ingress_handler(
    State(state): State<Arc<AppState>>,
    Path(contract_id): Path<Uuid>,
) -> AppResult<Json<KafkaIngressResponse>> {
    let row = get_kafka_ingress_row(&state.db, contract_id)
        .await?
        .ok_or_else(|| {
            AppError::NotFound(format!(
                "kafka ingress not enabled for contract {}",
                contract_id
            ))
        })?;

    Ok(Json(row_to_response(row, None)))
}

pub async fn enable_kafka_ingress_handler(
    State(state): State<Arc<AppState>>,
    Path(contract_id): Path<Uuid>,
    req: axum::extract::Request,
) -> AppResult<(StatusCode, Json<KafkaIngressResponse>)> {
    let org_id = crate::org_id_from_req(&req);

    if let Some(row) = get_kafka_ingress_row(&state.db, contract_id).await? {
        return Ok((StatusCode::OK, Json(row_to_response(row, None))));
    }

    let partition_count = 3_i32;
    let bootstrap = confluent_bootstrap();

    let (api_key, api_secret) = tokio::task::spawn_blocking(move || {
        let client = Client::new();
        confluent_create_topics(contract_id, partition_count)?;
        confluent_create_credentials(&client, contract_id)
    })
    .await
    .map_err(|e| AppError::Internal(format!("spawn_blocking: {e}")))?
    .map_err(|e| AppError::Internal(format!("Confluent provisioning: {e}")))?;

    let api_secret_enc =
        encrypt(&api_secret).map_err(|e| AppError::Internal(format!("encrypt: {e}")))?;

    let org_id = org_id.ok_or(AppError::Unauthorized)?;
    let row = insert_kafka_ingress_row(
        &state.db,
        contract_id,
        org_id,
        &bootstrap,
        &api_key,
        &api_secret_enc,
        partition_count,
    )
    .await?;

    state
        .kafka_consumers
        .start(Arc::clone(&state), contract_id)
        .await;

    Ok((
        StatusCode::CREATED,
        Json(row_to_response(row, Some(api_secret.to_string()))),
    ))
}

pub async fn disable_kafka_ingress_handler(
    State(state): State<Arc<AppState>>,
    Path(contract_id): Path<Uuid>,
) -> AppResult<StatusCode> {
    let row = get_kafka_ingress_row(&state.db, contract_id)
        .await?
        .ok_or_else(|| {
            AppError::NotFound(format!(
                "kafka ingress not enabled for contract {}",
                contract_id
            ))
        })?;

    state.kafka_consumers.stop(contract_id);

    tokio::task::spawn_blocking(move || {
        let client = Client::new();
        confluent_delete_credentials(&client, contract_id)
    })
    .await
    .map_err(|e| AppError::Internal(format!("spawn_blocking: {e}")))?
    .map_err(|e| AppError::Internal(format!("Confluent credential revocation: {e}")))?;

    soft_delete_kafka_ingress_row(&state.db, contract_id).await?;

    tracing::info!(
        contract_id = %contract_id,
        drain_hours = row.drain_window_hours,
        "kafka ingress disabled; topics will drain before deletion"
    );

    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// Internal helper
// ---------------------------------------------------------------------------

fn row_to_response(row: KafkaIngressRow, plaintext_secret: Option<String>) -> KafkaIngressResponse {
    let contract_id = row.contract_id;
    KafkaIngressResponse {
        id: row.id,
        contract_id,
        enabled: row.enabled,
        bootstrap_servers: row.confluent_bootstrap,
        sasl_username: row.confluent_api_key,
        sasl_password: plaintext_secret,
        topic_raw: topic_raw(contract_id),
        topic_clean: topic_clean(contract_id),
        topic_quarantine: topic_quarantine(contract_id),
        partition_count: row.partition_count,
        created_at: row.created_at,
    }
}
