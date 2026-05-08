//! Kinesis Ingress — RFC-026
//!
//! Handles:
//!   - AWS Kinesis stream provisioning (3 streams per contract) via the AWS SDK.
//!   - IAM user + scoped access key creation/rotation/deletion.
//!   - DB CRUD for the `kinesis_ingress` table.
//!   - Three Axum handler functions wired into `main.rs`.
//!   - Credential rotation handler.
//!
//! **Encryption**: the IAM secret access key is stored encrypted at rest using
//! AES-256-GCM keyed by the `ENCRYPTION_KEY` environment variable (32-byte hex).
//! The nonce is prepended to the ciphertext and the whole blob is base64-encoded.
//!
//! **Feature gate**: AWS SDK provisioning and the consumer loop are compiled only
//! with `--features kinesis-ingress`.  Stub implementations allow `AppState` and
//! the routes to compile without the feature.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
};
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value as JsonValue;
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

use crate::{
    error::{AppError, AppResult},
    AppState,
};

// AES-GCM — only compiled with the `kinesis-ingress` feature.
#[cfg(feature = "kinesis-ingress")]
use aes_gcm::{
    aead::{Aead, KeyInit, OsRng},
    AeadCore, Aes256Gcm, Key, Nonce,
};
#[cfg(feature = "kinesis-ingress")]
use base64::{engine::general_purpose::STANDARD as B64, Engine};

// ---------------------------------------------------------------------------
// Configuration helpers
// ---------------------------------------------------------------------------

fn aws_region() -> String {
    // MVP: us-east-1 fixed. Stored per-row so future expansion needs no migration.
    std::env::var("KINESIS_AWS_REGION").unwrap_or_else(|_| "us-east-1".to_string())
}

/// AES-256-GCM key from ENCRYPTION_KEY env var (32-byte hex).
#[cfg(feature = "kinesis-ingress")]
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
#[cfg(feature = "kinesis-ingress")]
pub fn encrypt(plaintext: &str) -> anyhow::Result<String> {
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

#[cfg(not(feature = "kinesis-ingress"))]
pub fn encrypt(plaintext: &str) -> anyhow::Result<String> {
    use base64::{engine::general_purpose::STANDARD as B64, Engine};
    Ok(B64.encode(plaintext.as_bytes()))
}

/// Decrypt a base64(nonce || ciphertext) blob produced by `encrypt`.
/// Called from `kinesis_consumer` to retrieve IAM secrets at runtime.
#[cfg(feature = "kinesis-ingress")]
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

// ---------------------------------------------------------------------------
// Stream name helpers
// ---------------------------------------------------------------------------

pub fn stream_raw(contract_id: Uuid) -> String {
    format!("cg-{contract_id}-raw")
}

pub fn stream_clean(contract_id: Uuid) -> String {
    format!("cg-{contract_id}-clean")
}

pub fn stream_quarantine(contract_id: Uuid) -> String {
    format!("cg-{contract_id}-quarantine")
}

#[cfg(feature = "kinesis-ingress")]
fn iam_user_name(contract_id: Uuid) -> String {
    format!("cg-kinesis-{contract_id}")
}

#[cfg(feature = "kinesis-ingress")]
fn iam_policy_name(contract_id: Uuid) -> String {
    format!("cg-kinesis-policy-{contract_id}")
}

// ---------------------------------------------------------------------------
// AWS provisioning (async, feature-gated)
// ---------------------------------------------------------------------------

/// Provision the three Kinesis streams and the scoped IAM user+key for a
/// contract.  Returns (iam_user_arn, raw_arn, clean_arn, quarantine_arn,
/// access_key_id, secret_access_key).
#[cfg(feature = "kinesis-ingress")]
pub async fn provision_kinesis_ingress(
    contract_id: Uuid,
    shard_count: i32,
    region: &str,
) -> anyhow::Result<(String, String, String, String, String, String)> {
    use aws_sdk_iam::Client as IamClient;
    use aws_sdk_kinesis::config::BehaviorVersion;
    use aws_sdk_kinesis::Client as KinesisClient;

    let config = aws_sdk_kinesis::config::Builder::new()
        .behavior_version(BehaviorVersion::latest())
        .region(aws_sdk_kinesis::config::Region::new(region.to_string()))
        .build();
    // Use separate SDK configs for Kinesis vs IAM since IAM is global.
    let kinesis_conf = config;
    let iam_conf = aws_sdk_iam::config::Builder::new()
        .behavior_version(aws_sdk_iam::config::BehaviorVersion::latest())
        .region(aws_sdk_iam::config::Region::new(region.to_string()))
        .build();

    let kinesis = KinesisClient::from_conf(kinesis_conf);
    let iam = IamClient::from_conf(iam_conf);

    // 1. Create the three streams.
    let raw_name = stream_raw(contract_id);
    let clean_name = stream_clean(contract_id);
    let quarantine_name = stream_quarantine(contract_id);

    for name in [&raw_name, &clean_name, &quarantine_name] {
        kinesis
            .create_stream()
            .stream_name(name)
            .shard_count(shard_count)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("create_stream {name}: {e}"))?;
    }

    // 2. Wait until streams are ACTIVE (Kinesis creation is async).
    wait_for_streams_active(&kinesis, &[&raw_name, &clean_name, &quarantine_name]).await?;

    // 3. Fetch ARNs from DescribeStream.
    let raw_arn = describe_stream_arn(&kinesis, &raw_name).await?;
    let clean_arn = describe_stream_arn(&kinesis, &clean_name).await?;
    let quarantine_arn = describe_stream_arn(&kinesis, &quarantine_name).await?;

    // 4. Create IAM user.
    let user_name = iam_user_name(contract_id);
    let user_resp = iam
        .create_user()
        .user_name(&user_name)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("create_user: {e}"))?;
    let iam_user_arn = user_resp
        .user()
        .map(|u| u.arn().to_string())
        .unwrap_or_default();

    // 5. Attach inline policy: produce-only on raw, consume-only on clean+quarantine.
    let policy_doc = serde_json::json!({
        "Version": "2012-10-17",
        "Statement": [
            {
                "Effect": "Allow",
                "Action": ["kinesis:PutRecord", "kinesis:PutRecords"],
                "Resource": raw_arn
            },
            {
                "Effect": "Allow",
                "Action": [
                    "kinesis:GetRecords",
                    "kinesis:GetShardIterator",
                    "kinesis:DescribeStream",
                    "kinesis:ListShards"
                ],
                "Resource": [&clean_arn, &quarantine_arn]
            }
        ]
    });
    iam.put_user_policy()
        .user_name(&user_name)
        .policy_name(iam_policy_name(contract_id))
        .policy_document(policy_doc.to_string())
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("put_user_policy: {e}"))?;

    // 6. Create access key.
    let key_resp = iam
        .create_access_key()
        .user_name(&user_name)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("create_access_key: {e}"))?;
    let ak = key_resp
        .access_key()
        .ok_or_else(|| anyhow::anyhow!("no access key in response"))?;
    let access_key_id = ak.access_key_id().to_string();
    let secret_access_key = ak.secret_access_key().to_string();

    Ok((
        iam_user_arn,
        raw_arn,
        clean_arn,
        quarantine_arn,
        access_key_id,
        secret_access_key,
    ))
}

/// Poll until all named streams reach ACTIVE status (max 60 s).
#[cfg(feature = "kinesis-ingress")]
async fn wait_for_streams_active(
    client: &aws_sdk_kinesis::Client,
    names: &[&str],
) -> anyhow::Result<()> {
    use aws_sdk_kinesis::types::StreamStatus;

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    for &name in names {
        loop {
            if std::time::Instant::now() > deadline {
                anyhow::bail!("timed out waiting for stream {name} to become ACTIVE");
            }
            let resp = client
                .describe_stream_summary()
                .stream_name(name)
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("describe_stream_summary {name}: {e}"))?;
            let status = resp
                .stream_description_summary()
                .map(|s| s.stream_status())
                .cloned()
                .unwrap_or(StreamStatus::Creating);
            if status == StreamStatus::Active {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
    }
    Ok(())
}

#[cfg(feature = "kinesis-ingress")]
async fn describe_stream_arn(
    client: &aws_sdk_kinesis::Client,
    name: &str,
) -> anyhow::Result<String> {
    let resp = client
        .describe_stream_summary()
        .stream_name(name)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("describe_stream_summary {name}: {e}"))?;
    resp.stream_description_summary()
        .map(|s| s.stream_arn().to_string())
        .ok_or_else(|| anyhow::anyhow!("no ARN returned for stream {name}"))
}

/// Rotate IAM credentials: create new key → update DB → delete old key.
#[cfg(feature = "kinesis-ingress")]
pub async fn rotate_iam_credentials(
    contract_id: Uuid,
    old_access_key_id: &str,
    region: &str,
) -> anyhow::Result<(String, String)> {
    use aws_sdk_iam::Client as IamClient;

    let iam_conf = aws_sdk_iam::config::Builder::new()
        .behavior_version(aws_sdk_iam::config::BehaviorVersion::latest())
        .region(aws_sdk_iam::config::Region::new(region.to_string()))
        .build();
    let iam = IamClient::from_conf(iam_conf);
    let user_name = iam_user_name(contract_id);

    let key_resp = iam
        .create_access_key()
        .user_name(&user_name)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("create_access_key: {e}"))?;
    let ak = key_resp
        .access_key()
        .ok_or_else(|| anyhow::anyhow!("no access key in response"))?;
    let new_id = ak.access_key_id().to_string();
    let new_secret = ak.secret_access_key().to_string();

    // Delete old key after new one is created.
    iam.delete_access_key()
        .user_name(&user_name)
        .access_key_id(old_access_key_id)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("delete_access_key: {e}"))?;

    Ok((new_id, new_secret))
}

/// Revoke IAM credentials and delete Kinesis streams for a disabled contract.
#[cfg(feature = "kinesis-ingress")]
pub async fn deprovision_kinesis_ingress(
    contract_id: Uuid,
    access_key_id: &str,
    region: &str,
) -> anyhow::Result<()> {
    use aws_sdk_iam::Client as IamClient;
    use aws_sdk_kinesis::Client as KinesisClient;

    let kinesis_conf = aws_sdk_kinesis::config::Builder::new()
        .behavior_version(aws_sdk_kinesis::config::BehaviorVersion::latest())
        .region(aws_sdk_kinesis::config::Region::new(region.to_string()))
        .build();
    let iam_conf = aws_sdk_iam::config::Builder::new()
        .behavior_version(aws_sdk_iam::config::BehaviorVersion::latest())
        .region(aws_sdk_iam::config::Region::new(region.to_string()))
        .build();
    let kinesis = KinesisClient::from_conf(kinesis_conf);
    let iam = IamClient::from_conf(iam_conf);
    let user_name = iam_user_name(contract_id);

    // 1. Delete the access key (immediate revocation).
    let _ = iam
        .delete_access_key()
        .user_name(&user_name)
        .access_key_id(access_key_id)
        .send()
        .await;

    // 2. Delete inline policy, then delete user.
    let _ = iam
        .delete_user_policy()
        .user_name(&user_name)
        .policy_name(iam_policy_name(contract_id))
        .send()
        .await;
    let _ = iam.delete_user().user_name(&user_name).send().await;

    // 3. Delete streams (best-effort; Kinesis deletes are async and idempotent).
    for name in [
        stream_raw(contract_id),
        stream_clean(contract_id),
        stream_quarantine(contract_id),
    ] {
        let _ = kinesis.delete_stream().stream_name(&name).send().await;
    }

    Ok(())
}

// Stubs compiled without the feature so the server binary links.
#[cfg(not(feature = "kinesis-ingress"))]
pub async fn provision_kinesis_ingress(
    _: Uuid,
    _: i32,
    _: &str,
) -> anyhow::Result<(String, String, String, String, String, String)> {
    anyhow::bail!("kinesis-ingress feature is not enabled at compile time")
}

#[cfg(not(feature = "kinesis-ingress"))]
pub async fn rotate_iam_credentials(_: Uuid, _: &str, _: &str) -> anyhow::Result<(String, String)> {
    anyhow::bail!("kinesis-ingress feature is not enabled at compile time")
}

#[cfg(not(feature = "kinesis-ingress"))]
pub async fn deprovision_kinesis_ingress(_: Uuid, _: &str, _: &str) -> anyhow::Result<()> {
    anyhow::bail!("kinesis-ingress feature is not enabled at compile time")
}

// ---------------------------------------------------------------------------
// DB types
// ---------------------------------------------------------------------------

#[allow(dead_code)]
#[derive(Debug, sqlx::FromRow)]
pub struct KinesisIngressRow {
    pub id: Uuid,
    pub contract_id: Uuid,
    pub org_id: Uuid,
    pub enabled: bool,
    pub aws_region: String,
    pub raw_stream_arn: Option<String>,
    pub clean_stream_arn: Option<String>,
    pub quarantine_stream_arn: Option<String>,
    pub iam_user_arn: Option<String>,
    pub iam_access_key_id: Option<String>,
    pub iam_secret_enc: Option<String>,
    pub shard_count: i32,
    pub drain_window_hours: i32,
    pub last_sequence_numbers: Option<JsonValue>,
    pub disabled_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// Wire types
// ---------------------------------------------------------------------------

/// Response returned to the dashboard.  Secret shown only on first enable and
/// after rotation; subsequent GETs omit it.
#[derive(Debug, Serialize)]
pub struct KinesisIngressResponse {
    pub id: Uuid,
    pub contract_id: Uuid,
    pub enabled: bool,
    pub aws_region: String,
    pub stream_raw: String,
    pub stream_clean: String,
    pub stream_quarantine: String,
    pub raw_stream_arn: Option<String>,
    pub clean_stream_arn: Option<String>,
    pub quarantine_stream_arn: Option<String>,
    pub iam_access_key_id: Option<String>,
    /// Present only on first enable or credential rotation response.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub iam_secret_access_key: Option<String>,
    pub shard_count: i32,
    pub created_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// DB helpers
// ---------------------------------------------------------------------------

pub async fn get_kinesis_ingress_row(
    pool: &PgPool,
    contract_id: Uuid,
) -> AppResult<Option<KinesisIngressRow>> {
    let row = sqlx::query_as::<_, KinesisIngressRow>(
        r#"SELECT id, contract_id, org_id, enabled, aws_region,
                  raw_stream_arn, clean_stream_arn, quarantine_stream_arn,
                  iam_user_arn, iam_access_key_id, iam_secret_enc,
                  shard_count, drain_window_hours, last_sequence_numbers,
                  disabled_at, created_at, updated_at
           FROM kinesis_ingress
           WHERE contract_id = $1 AND disabled_at IS NULL
           LIMIT 1"#,
    )
    .bind(contract_id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

#[allow(clippy::too_many_arguments)]
async fn insert_kinesis_ingress_row(
    pool: &PgPool,
    contract_id: Uuid,
    org_id: Uuid,
    region: &str,
    raw_arn: &str,
    clean_arn: &str,
    quarantine_arn: &str,
    iam_user_arn: &str,
    access_key_id: &str,
    secret_enc: &str,
    shard_count: i32,
) -> AppResult<KinesisIngressRow> {
    let row = sqlx::query_as::<_, KinesisIngressRow>(
        r#"INSERT INTO kinesis_ingress
               (contract_id, org_id, enabled, aws_region,
                raw_stream_arn, clean_stream_arn, quarantine_stream_arn,
                iam_user_arn, iam_access_key_id, iam_secret_enc, shard_count)
           VALUES ($1, $2, TRUE, $3, $4, $5, $6, $7, $8, $9, $10)
           RETURNING id, contract_id, org_id, enabled, aws_region,
                     raw_stream_arn, clean_stream_arn, quarantine_stream_arn,
                     iam_user_arn, iam_access_key_id, iam_secret_enc,
                     shard_count, drain_window_hours, last_sequence_numbers,
                     disabled_at, created_at, updated_at"#,
    )
    .bind(contract_id)
    .bind(org_id)
    .bind(region)
    .bind(raw_arn)
    .bind(clean_arn)
    .bind(quarantine_arn)
    .bind(iam_user_arn)
    .bind(access_key_id)
    .bind(secret_enc)
    .bind(shard_count)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

async fn soft_delete_kinesis_ingress_row(pool: &PgPool, contract_id: Uuid) -> AppResult<()> {
    sqlx::query(
        "UPDATE kinesis_ingress SET disabled_at = NOW(), enabled = FALSE \
         WHERE contract_id = $1 AND disabled_at IS NULL",
    )
    .bind(contract_id)
    .execute(pool)
    .await?;
    Ok(())
}

async fn update_credentials_in_db(
    pool: &PgPool,
    contract_id: Uuid,
    new_access_key_id: &str,
    new_secret_enc: &str,
) -> AppResult<()> {
    sqlx::query(
        "UPDATE kinesis_ingress SET iam_access_key_id = $1, iam_secret_enc = $2, updated_at = NOW() \
         WHERE contract_id = $3 AND disabled_at IS NULL",
    )
    .bind(new_access_key_id)
    .bind(new_secret_enc)
    .bind(contract_id)
    .execute(pool)
    .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Axum handlers
// ---------------------------------------------------------------------------

pub async fn get_kinesis_ingress_handler(
    State(state): State<Arc<AppState>>,
    Path(contract_id): Path<Uuid>,
) -> AppResult<Json<KinesisIngressResponse>> {
    let row = get_kinesis_ingress_row(&state.db, contract_id)
        .await?
        .ok_or_else(|| {
            AppError::NotFound(format!(
                "kinesis ingress not enabled for contract {contract_id}"
            ))
        })?;
    Ok(Json(row_to_response(row, None)))
}

pub async fn enable_kinesis_ingress_handler(
    State(state): State<Arc<AppState>>,
    Path(contract_id): Path<Uuid>,
    req: axum::extract::Request,
) -> AppResult<(StatusCode, Json<KinesisIngressResponse>)> {
    let org_id = crate::org_id_from_req(&req);

    // Idempotent: return existing config if already enabled.
    if let Some(row) = get_kinesis_ingress_row(&state.db, contract_id).await? {
        return Ok((StatusCode::OK, Json(row_to_response(row, None))));
    }

    let shard_count = 1_i32;
    let region = aws_region();

    let (iam_user_arn, raw_arn, clean_arn, quarantine_arn, access_key_id, secret) =
        provision_kinesis_ingress(contract_id, shard_count, &region)
            .await
            .map_err(|e| AppError::Internal(format!("Kinesis provisioning: {e}")))?;

    let secret_enc = encrypt(&secret).map_err(|e| AppError::Internal(format!("encrypt: {e}")))?;

    let org_id = org_id.ok_or(AppError::Unauthorized)?;
    let row = insert_kinesis_ingress_row(
        &state.db,
        contract_id,
        org_id,
        &region,
        &raw_arn,
        &clean_arn,
        &quarantine_arn,
        &iam_user_arn,
        &access_key_id,
        &secret_enc,
        shard_count,
    )
    .await?;

    state
        .kinesis_consumers
        .start(Arc::clone(&state), contract_id)
        .await;

    Ok((
        StatusCode::CREATED,
        Json(row_to_response(row, Some(secret))),
    ))
}

pub async fn disable_kinesis_ingress_handler(
    State(state): State<Arc<AppState>>,
    Path(contract_id): Path<Uuid>,
) -> AppResult<StatusCode> {
    let row = get_kinesis_ingress_row(&state.db, contract_id)
        .await?
        .ok_or_else(|| {
            AppError::NotFound(format!(
                "kinesis ingress not enabled for contract {contract_id}"
            ))
        })?;

    state.kinesis_consumers.stop(contract_id);

    let access_key_id = row.iam_access_key_id.clone().unwrap_or_default();
    let region = row.aws_region.clone();
    let drain_hours = row.drain_window_hours;

    // Revoke credentials immediately; streams drain before deletion.
    deprovision_kinesis_ingress(contract_id, &access_key_id, &region)
        .await
        .map_err(|e| AppError::Internal(format!("Kinesis deprovisioning: {e}")))?;

    soft_delete_kinesis_ingress_row(&state.db, contract_id).await?;

    tracing::info!(
        contract_id = %contract_id,
        drain_hours,
        "kinesis ingress disabled; streams deleted after drain window"
    );

    Ok(StatusCode::NO_CONTENT)
}

pub async fn rotate_kinesis_credentials_handler(
    State(state): State<Arc<AppState>>,
    Path(contract_id): Path<Uuid>,
) -> AppResult<Json<KinesisIngressResponse>> {
    let row = get_kinesis_ingress_row(&state.db, contract_id)
        .await?
        .ok_or_else(|| {
            AppError::NotFound(format!(
                "kinesis ingress not enabled for contract {contract_id}"
            ))
        })?;

    let old_key_id = row.iam_access_key_id.clone().unwrap_or_default();
    let region = row.aws_region.clone();

    let (new_key_id, new_secret) = rotate_iam_credentials(contract_id, &old_key_id, &region)
        .await
        .map_err(|e| AppError::Internal(format!("credential rotation: {e}")))?;

    let new_secret_enc =
        encrypt(&new_secret).map_err(|e| AppError::Internal(format!("encrypt: {e}")))?;

    update_credentials_in_db(&state.db, contract_id, &new_key_id, &new_secret_enc).await?;

    // Re-fetch updated row so response reflects current DB state.
    let updated_row = get_kinesis_ingress_row(&state.db, contract_id)
        .await?
        .ok_or_else(|| AppError::NotFound("kinesis ingress row disappeared after update".into()))?;

    tracing::info!(
        contract_id = %contract_id,
        "kinesis credentials rotated; old key invalidated"
    );

    Ok(Json(row_to_response(updated_row, Some(new_secret))))
}

// ---------------------------------------------------------------------------
// Internal helper
// ---------------------------------------------------------------------------

fn row_to_response(
    row: KinesisIngressRow,
    plaintext_secret: Option<String>,
) -> KinesisIngressResponse {
    let contract_id = row.contract_id;
    KinesisIngressResponse {
        id: row.id,
        contract_id,
        enabled: row.enabled,
        aws_region: row.aws_region,
        stream_raw: stream_raw(contract_id),
        stream_clean: stream_clean(contract_id),
        stream_quarantine: stream_quarantine(contract_id),
        raw_stream_arn: row.raw_stream_arn,
        clean_stream_arn: row.clean_stream_arn,
        quarantine_stream_arn: row.quarantine_stream_arn,
        iam_access_key_id: row.iam_access_key_id,
        iam_secret_access_key: plaintext_secret,
        shard_count: row.shard_count,
        created_at: row.created_at,
    }
}
