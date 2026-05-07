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

#[cfg(not(feature = "kafka-ingress"))]
pub fn decrypt_secret(encoded: &str) -> anyhow::Result<String> {
    use base64::{engine::general_purpose::STANDARD as B64, Engine};
    let bytes = B64.decode(encoded)?;
    Ok(String::from_utf8(bytes)?)
}

// ---------------------------------------------------------------------------
// DB types
// ---------------------------------------------------------------------------

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

/// Create the three ingress topics on Confluent Cloud.
fn confluent_create_topics(
    client: &Client,
    contract_id: Uuid,
    partition_count: i32,
) -> anyhow::Result<()> {
    let cluster_id = confluent_cluster_id();
    let url = format!(
        "{}/kafka/v3/clusters/{}/topics",
        confluent_base_url(),
        cluster_id
    );

    for topic in [
        topic_raw(contract_id),
        topic_clean(contract_id),
        topic_quarantine(contract_id),
    ] {
        let body = serde_json::json!({
            "topic_name": topic,
            "partitions_count": partition_count,
            "replication_factor": 3,
            "configs": [
                { "name": "retention.ms", "value": "604800000" }  // 7 days
            ]
        });
        let resp = client
            .post(&url)
            .basic_auth(confluent_api_key(), Some(confluent_api_secret()))
            .json(&body)
            .send()?;

        // 201 = created, 400 with error_code 40002 = topic already exists (idempotent).
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().unwrap_or_default();
            // Tolerate "topic already exists" so re-enable is idempotent.
            if !text.contains("40002") {
                anyhow::bail!("Confluent create topic {topic} failed ({status}): {text}");
            }
        }
    }
    Ok(())
}

/// Create a Confluent Cloud API key scoped to the given cluster, then apply
/// ACLs: produce on raw, consume on clean + quarantine.
///
/// Returns (api_key, api_secret).
fn confluent_create_credentials(
    client: &Client,
    contract_id: Uuid,
) -> anyhow::Result<(String, String)> {
    let env_id = confluent_environment_id();
    let cluster_id = confluent_cluster_id();

    // 1. Create service account (idempotent by name).
    let sa_name = format!("cg-ingress-{}", contract_id);
    let sa_url = format!("{}/iam/v2/service-accounts", confluent_base_url());
    let sa_body = serde_json::json!({
        "display_name": sa_name,
        "description": format!("ContractGate kafka ingress for contract {}", contract_id)
    });
    let sa_resp: serde_json::Value = client
        .post(&sa_url)
        .basic_auth(confluent_api_key(), Some(confluent_api_secret()))
        .json(&sa_body)
        .send()?
        .json()?;
    let sa_id = sa_resp["id"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("unexpected SA response: {sa_resp}"))?
        .to_string();

    // 2. Create API key for that service account, scoped to the cluster.
    let key_url = format!("{}/iam/v2/api-keys", confluent_base_url());
    let key_body = serde_json::json!({
        "spec": {
            "display_name": format!("cg-{}", contract_id),
            "description": "ContractGate kafka ingress key",
            "owner": { "id": sa_id },
            "resource": {
                "id": cluster_id,
                "environment": { "id": env_id }
            }
        }
    });
    let key_resp: serde_json::Value = client
        .post(&key_url)
        .basic_auth(confluent_api_key(), Some(confluent_api_secret()))
        .json(&key_body)
        .send()?
        .json()?;

    let api_key = key_resp["id"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("unexpected key response: {key_resp}"))?
        .to_string();
    let api_secret = key_resp["spec"]["secret"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("missing secret in key response"))?
        .to_string();

    // 3. Grant ACLs: produce on raw, consume on clean + quarantine.
    let acl_url = format!(
        "{}/kafka/v3/clusters/{}/acls",
        confluent_base_url(),
        cluster_id
    );

    let acls = vec![
        // Produce on raw
        (topic_raw(contract_id), "WRITE"),
        // Consume on clean + quarantine
        (topic_clean(contract_id), "READ"),
        (topic_quarantine(contract_id), "READ"),
    ];

    for (topic, operation) in acls {
        let acl_body = serde_json::json!({
            "resource_type": "TOPIC",
            "resource_name": topic,
            "pattern_type": "LITERAL",
            "principal": format!("User:{}", sa_id),
            "host": "*",
            "operation": operation,
            "permission": "ALLOW"
        });
        let resp = client
            .post(&acl_url)
            .basic_auth(confluent_api_key(), Some(confluent_api_secret()))
            .json(&acl_body)
            .send()?;
        if !resp.status().is_success() {
            let text = resp.text().unwrap_or_default();
            tracing::warn!("ACL grant {operation} on {topic} failed: {text}");
        }
    }

    Ok((api_key, api_secret))
}

/// Delete the service account (and its API keys) for a contract.
fn confluent_delete_credentials(client: &Client, contract_id: Uuid) -> anyhow::Result<()> {
    // Look up SA by display name and delete it.  If not found, treat as already gone.
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

/// Delete the three ingress topics for a contract.  Called after the drain
/// window elapses; not called on immediate disable.
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
            // 40403 = topic not found — already gone, treat as success.
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

/// `GET /contracts/:id/kafka-ingress` — return current config (no secret).
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

/// `POST /contracts/:id/kafka-ingress/enable` — provision Confluent resources
/// and store encrypted credentials.
pub async fn enable_kafka_ingress_handler(
    State(state): State<Arc<AppState>>,
    Path(contract_id): Path<Uuid>,
    req: axum::extract::Request,
) -> AppResult<(StatusCode, Json<KafkaIngressResponse>)> {
    let org_id = crate::org_id_from_req(&req);

    // Idempotent: if already enabled, return existing config (no secret).
    if let Some(row) = get_kafka_ingress_row(&state.db, contract_id).await? {
        return Ok((StatusCode::OK, Json(row_to_response(row, None))));
    }

    let partition_count = 3_i32;
    let bootstrap = confluent_bootstrap();

    // Spawn Confluent provisioning off the async thread (blocking HTTP).
    let (api_key, api_secret) = tokio::task::spawn_blocking(move || {
        let client = Client::new();
        confluent_create_topics(&client, contract_id, partition_count)?;
        confluent_create_credentials(&client, contract_id)
    })
    .await
    .map_err(|e| AppError::Internal(format!("spawn_blocking: {e}")))?
    .map_err(|e| AppError::Internal(format!("Confluent provisioning: {e}")))?;

    // Encrypt secret before storing.
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

    // Start consumer for this contract.
    state
        .kafka_consumers
        .start(Arc::clone(&state), contract_id)
        .await;
    // Note: `state` is Arc<AppState> here (from axum State extractor).

    // Return plaintext secret once — not stored again.
    Ok((
        StatusCode::CREATED,
        Json(row_to_response(row, Some(api_secret))),
    ))
}

/// `DELETE /contracts/:id/kafka-ingress/disable` — revoke credentials
/// immediately; topics soft-deleted (drained after window).
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

    // Stop consumer first so no more messages are processed.
    state.kafka_consumers.stop(contract_id);

    // Revoke Confluent credentials immediately.
    tokio::task::spawn_blocking(move || {
        let client = Client::new();
        confluent_delete_credentials(&client, contract_id)
    })
    .await
    .map_err(|e| AppError::Internal(format!("spawn_blocking: {e}")))?
    .map_err(|e| AppError::Internal(format!("Confluent credential revocation: {e}")))?;

    // Soft-delete the row; topics survive until drain window elapses.
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
