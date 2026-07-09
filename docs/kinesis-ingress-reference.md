# Kinesis Ingress — API Reference

**RFC:** 026  
**Status:** Accepted  
**Added:** nightly-maintenance-2026-05-15 (impl); reference doc added 2026-05-24  
**Feature flag:** `--features kinesis-ingress` (compile-time opt-in)

---

## Overview

Kinesis Ingress provisions a dedicated set of AWS Kinesis streams and a scoped
IAM user for each contract. Events published to the raw stream are consumed by
ContractGate, validated against the contract, and routed to either the clean or
quarantine stream. The IAM access key is stored encrypted at rest (AES-256-GCM)
and is returned in plaintext only on first enable or after a credential rotation.

The feature requires `--features kinesis-ingress` at compile time. Without that
flag the routes compile but every handler returns `501 Not Implemented`.

---

## Stream topology

For each contract three Kinesis streams are created:

| Stream name pattern | Purpose |
|---|---|
| `cg-{contract_id}-raw` | Inbound events from the data producer. Producer writes here. |
| `cg-{contract_id}-clean` | Events that passed validation. Downstream consumers read from here. |
| `cg-{contract_id}-quarantine` | Events that failed validation. Held for inspection or replay. |

---

## Environment variables

| Variable | Required | Default | Description |
|---|---|---|---|
| `KINESIS_AWS_REGION` | No | `us-east-1` | AWS region for provisioned streams. Stored per-row so changing it only affects new contracts. |
| `ENCRYPTION_KEY` | Yes (with feature) | — | 32-byte hex string (64 hex chars) used as the AES-256-GCM key for encrypting IAM secrets at rest. |
| AWS credential variables | Yes | instance role | Standard chain: `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, `AWS_SESSION_TOKEN`, or instance/task role. |

---

## Endpoints

All endpoints require a valid `x-api-key` header.

### `GET /contracts/{contract_id}/kinesis-ingress`

Return the current Kinesis ingress configuration for a contract.

**Path parameter:** `contract_id` — UUID of the contract.

**Response `200 OK`:**

```json
{
  "id": "e1a2b3c4-...",
  "contract_id": "d5e6f7a8-...",
  "enabled": true,
  "aws_region": "us-east-1",
  "stream_raw": "cg-d5e6f7a8-...-raw",
  "stream_clean": "cg-d5e6f7a8-...-clean",
  "stream_quarantine": "cg-d5e6f7a8-...-quarantine",
  "raw_stream_arn": "arn:aws:kinesis:us-east-1:123456789:stream/cg-d5e6f7a8-...-raw",
  "clean_stream_arn": "arn:aws:kinesis:us-east-1:123456789:stream/cg-d5e6f7a8-...-clean",
  "quarantine_stream_arn": "arn:aws:kinesis:us-east-1:123456789:stream/cg-d5e6f7a8-...-quarantine",
  "iam_access_key_id": "AKIAIOSFODNN7EXAMPLE",
  "shard_count": 1,
  "created_at": "2026-05-01T10:00:00Z"
}
```

`iam_secret_access_key` is **never** returned by this endpoint. It is only
present in the `POST` (enable) and credential-rotation responses.

**Response `404 Not Found`:** kinesis ingress not enabled for this contract.

---

### `POST /contracts/{contract_id}/kinesis-ingress`

Enable Kinesis ingress for a contract. Provisions the three streams and a scoped
IAM user. Idempotent — if ingress is already enabled, returns `200 OK` with the
existing config (secret not re-exposed).

**Path parameter:** `contract_id` — UUID of the contract.

**Request body:** empty (no body required).

**Response `201 Created`** (first enable):

```json
{
  "id": "e1a2b3c4-...",
  "contract_id": "d5e6f7a8-...",
  "enabled": true,
  "aws_region": "us-east-1",
  "stream_raw": "cg-d5e6f7a8-...-raw",
  "stream_clean": "cg-d5e6f7a8-...-clean",
  "stream_quarantine": "cg-d5e6f7a8-...-quarantine",
  "raw_stream_arn": "arn:aws:kinesis:us-east-1:...",
  "clean_stream_arn": "arn:aws:kinesis:us-east-1:...",
  "quarantine_stream_arn": "arn:aws:kinesis:us-east-1:...",
  "iam_access_key_id": "AKIAIOSFODNN7EXAMPLE",
  "iam_secret_access_key": "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
  "shard_count": 1,
  "created_at": "2026-05-01T10:00:00Z"
}
```

`iam_secret_access_key` is included **only** in this response. Save it
immediately — it cannot be retrieved again without rotating credentials.

**Response `200 OK`** (already enabled): same shape, `iam_secret_access_key` absent.

**Response `401 Unauthorized`:** no org context (org ID cannot be determined from the request).

---

### `DELETE /contracts/{contract_id}/kinesis-ingress`

Disable Kinesis ingress. Immediately revokes the IAM access key, deletes the
IAM user and inline policy, and soft-deletes the streams (Kinesis deletion is
asynchronous). The `kinesis_ingress` row is soft-deleted (`disabled_at` set).

**Response `204 No Content`:** success.

**Response `404 Not Found`:** ingress not enabled.

---

### `POST /contracts/{contract_id}/kinesis-ingress/rotate-credentials`

Rotate the IAM access key. Creates a new key, updates the database, then
deletes the old key. The new plaintext secret is returned once.

**Response `200 OK`:**

```json
{
  "id": "e1a2b3c4-...",
  "contract_id": "d5e6f7a8-...",
  "enabled": true,
  "aws_region": "us-east-1",
  "stream_raw": "cg-d5e6f7a8-...-raw",
  "stream_clean": "cg-d5e6f7a8-...-clean",
  "stream_quarantine": "cg-d5e6f7a8-...-quarantine",
  "iam_access_key_id": "AKIAI4EXAMPLE2NEWKEY",
  "iam_secret_access_key": "newSecretValueReturnedOnce...",
  "shard_count": 1,
  "created_at": "2026-05-01T10:00:00Z"
}
```

**Response `404 Not Found`:** ingress not enabled.

---

## IAM policy

The provisioned IAM user receives an inline policy scoped to the three streams:

| Action | Stream |
|---|---|
| `kinesis:PutRecord`, `kinesis:PutRecords` | raw (`-raw`) only |
| `kinesis:GetRecords`, `kinesis:GetShardIterator`, `kinesis:DescribeStream`, `kinesis:ListShards` | clean and quarantine only |

The raw stream is write-only to the data producer; the clean and quarantine
streams are read-only (for downstream consumers). The IAM user cannot read the
raw stream or write to clean/quarantine.

---

## Credential encryption

The `ENCRYPTION_KEY` env var must be a 64-character hex string encoding 32
random bytes. The secret access key is encrypted with AES-256-GCM before
storage. The nonce (12 bytes) is prepended to the ciphertext and the whole
blob is base64-encoded.

```bash
# Generate a suitable key
openssl rand -hex 32
```

Without `ENCRYPTION_KEY` set (and without the `kinesis-ingress` feature), the
fallback stores a base64-encoded plaintext as a stub — **not safe for
production**.

---

## Database objects

Migration `015_kinesis_ingress.sql` adds the `kinesis_ingress` table:

| Column | Type | Notes |
|---|---|---|
| `id` | UUID | Primary key |
| `contract_id` | UUID | FK → `contracts.id` |
| `org_id` | UUID | FK → `organizations.id` |
| `enabled` | boolean | False after disable |
| `aws_region` | text | e.g. `us-east-1` |
| `raw_stream_arn` | text | ARN of the raw stream |
| `clean_stream_arn` | text | ARN of the clean stream |
| `quarantine_stream_arn` | text | ARN of the quarantine stream |
| `iam_user_arn` | text | ARN of the provisioned IAM user |
| `iam_access_key_id` | text | Current key ID (plaintext) |
| `iam_secret_enc` | text | AES-256-GCM encrypted secret (base64) |
| `shard_count` | integer | Fixed at 1 in v1 |
| `drain_window_hours` | integer | Default 24 h; streams drain before deletion |
| `disabled_at` | timestamptz | Set on disable; row is hidden from GET |

---

## Edge cases

- **Stream provisioning is async.** The enable handler polls up to 60 seconds
  for all three streams to reach `ACTIVE` status before proceeding. If the
  timeout is exceeded, provisioning fails and no DB row is inserted.
- **Idempotent enable.** A second `POST` returns `200 OK` with the existing
  row; no new AWS resources are created.
- **Key rotation atomicity.** The new key is created before the old key is
  deleted. If deletion of the old key fails it is logged but does not fail the
  rotation response.
- **Feature not compiled.** Without `--features kinesis-ingress`, every handler
  returns `501 Not Implemented` with the message
  `"kinesis-ingress feature is not enabled at compile time"`.
- **Plan gating.** Kafka and Kinesis integration tabs require the Growth plan
  (RFC-045). See [plan-gating-reference.md](plan-gating-reference.md).
