# RFC-026: AWS Kinesis Ingress for Hosted ContractGate

| Field    | Value                  |
|----------|------------------------|
| Status   | **Draft**              |
| Author   | ContractGate team      |
| Created  | 2026-05-07             |
| Branch   | `nightly-maintenance-2026-05-07` |

---

## Problem

Teams running AWS-native data pipelines use Kinesis Data Streams as their
primary event bus. ContractGate's HTTP ingest path works, but requires
producers to speak HTTP or a sidecar bridge. Native Kinesis ingress removes
that friction — users produce to a ContractGate-managed stream and receive
validated events on a clean output stream, with no changes to their existing
Kinesis producers.

---

## Proposal

Hosted ContractGate provisions per-contract Kinesis streams in a shared
ContractGate AWS account and runs consumers platform-side. The user produces
to a well-known input stream; ContractGate validates each record and routes
it to a clean output stream or a quarantine stream — same validation
semantics as the HTTP and Kafka paths.

---

## Design

### 1. Stream Naming

| Purpose    | Stream name                            |
|------------|----------------------------------------|
| Raw input  | `cg-{contract_id}-raw`                 |
| Clean out  | `cg-{contract_id}-clean`               |
| Quarantine | `cg-{contract_id}-quarantine`          |

`contract_id` is the existing UUID. No new identifier introduced.

### 2. Dashboard Toggle

A **"Enable Kinesis Ingress"** toggle appears on the contract detail page.
On enable, the backend:

1. Creates the three streams via AWS SDK (`CreateStream`).
2. Creates a scoped IAM user with a policy limited to produce on the raw
   stream and consume from clean + quarantine streams.
3. Generates an access key pair for that IAM user.
4. Returns the stream ARNs + credentials to the dashboard for display
   (copy-on-reveal, same UX as existing API keys).

On disable, credentials are revoked immediately (key pair deleted); streams
are soft-deleted after a configurable drain window (default 24 h).

### 3. AWS Account Model

ContractGate provisions all streams in a **shared ContractGate AWS account**,
one IAM user per contract. Users need no AWS account of their own — they
receive static access key credentials scoped to their streams only.

> **Open question — see §8.** User-account ("bring your own stream") support
> is explicitly deferred.

### 4. Consumer Architecture

A shared async consumer loop runs platform-side using the AWS SDK for Rust
(`aws-sdk-kinesis`). Each enabled contract gets one consumer application
subscribed to its raw stream.

Kinesis does not have consumer groups; instead, each shard is polled
independently via `GetRecords`. Default shard count is **1** (one ordered
lane, 1 MB/s ingest, 2 MB/s egress). Shard count is stored per-row so it can
be increased without migration.

The consumer loop checkpoints sequence numbers in the `kinesis_ingress` table
(no external checkpoint store required). On crash/restart, the consumer
resumes from the last stored sequence number.

Idle contracts (no records for 5 min) pause polling to avoid unnecessary
GetRecords API calls (which are metered). The consumer resumes automatically
on the next incoming record via a lightweight wakeup ping endpoint.

```
Producer (user, any AWS SDK)
    │
    ▼
cg-{contract_id}-raw   (Kinesis, ContractGate AWS account)
    │
    ▼
ContractGate Consumer loop (aws-sdk-kinesis / Tokio)
    │
    ├─ valid   ──▶  cg-{contract_id}-clean
    └─ invalid ──▶  cg-{contract_id}-quarantine
                        + metadata record: violation_reason + original payload
```

Quarantine records are never mutated or dropped. Violation metadata is written
as a JSON envelope wrapping the original payload:

```json
{
  "cg_violation_reason": "...",
  "cg_contract_version": "1.2",
  "cg_original_payload": { ...original record data... }
}
```

(Kinesis has no record-level headers; the envelope pattern matches the
Kinesis ecosystem convention.)

### 5. Credentials

- Static IAM access key + secret, one pair per contract.
- IAM policy: `kinesis:PutRecord`, `kinesis:PutRecords` on raw stream ARN;
  `kinesis:GetRecords`, `kinesis:GetShardIterator`, `kinesis:DescribeStream`
  on clean + quarantine stream ARNs.
- Stored encrypted in Supabase; displayed once (copy-on-reveal).
- Rotation: create new key pair → update `kinesis_ingress` row → delete old
  key pair. Old credentials invalid within one poll cycle.

### 6. Supabase Schema Additions

```sql
create table kinesis_ingress (
  id                    uuid primary key default gen_random_uuid(),
  contract_id           uuid not null references contracts(id) on delete cascade,
  enabled               boolean not null default false,
  aws_region            text not null default 'us-east-1',
  raw_stream_arn        text,
  clean_stream_arn      text,
  quarantine_stream_arn text,
  iam_user_arn          text,
  iam_access_key_id     text,
  iam_secret_enc        text,           -- encrypted at rest
  shard_count           int not null default 1,
  drain_window_hours    int not null default 24,
  last_sequence_number  text,           -- checkpoint; null = TRIM_HORIZON
  created_at            timestamptz default now(),
  updated_at            timestamptz default now()
);
```

RLS: org-scoped via `get_my_org_ids()` helper — no inline subqueries.

### 7. Audit Log

No new table. Each record processed via Kinesis writes to `audit_logs` with
`source = 'kinesis'` and `contract_version` set to the version that actually
matched. No defaulting.

### 8. Validation Engine

Zero changes. The consumer calls the same `validate_event()` the HTTP handler
calls. p99 budget for validation remains <15 ms; Kinesis consumer overhead is
outside that budget.

---

## What This RFC Does Not Cover

| Deferred topic                      | Rationale                                          |
|-------------------------------------|----------------------------------------------------|
| User-account ("bring your own") streams | Requires cross-account IAM assume-role; separate RFC |
| Enhanced fan-out                    | Dedicated throughput at higher cost; post-GA       |
| Per-stream retention config         | Kinesis default is 24 h; configurable knob deferred to quota RFC |
| Multi-region streams                | Infrastructure concern, post-GA                    |
| Billing metering per record         | Depends on quota RFC                               |
| Avro / Protobuf deserialization     | Schema Registry equivalent — separate RFC          |

---

## Resolved Decisions

| Question | Decision | Rationale |
|----------|----------|-----------|
| **AWS region** | `us-east-1` fixed for MVP | Multi-region deferred; single region keeps provisioning simple |
| **Shard count** | 1 shard default | 1 MB/s ingest sufficient for MVP; stored per-row for future increase |
| **Enhanced fan-out** | Standard `GetRecords` | MVP workloads don't justify $0.015/shard-hour extra; add in post-GA if latency demands it |
| **Checkpoint store** | Sequence number in `kinesis_ingress` row | No new infra; Supabase already authoritative for contract state |
| **Idle wakeup** | Slow poll at 1 req/min | Simplest path; GetRecords calls are cheap; wakeup endpoint adds surface for no MVP gain |

---

## Acceptance Criteria

- [ ] `kinesis_ingress` table migrated and covered by RLS.
- [ ] Toggle in dashboard provisions streams + IAM user and returns credentials.
- [ ] Consumer reads from raw stream, calls existing validator, routes correctly.
- [ ] Quarantine records contain original payload + violation reason envelope.
- [ ] Audit logs written with `source = 'kinesis'` and correct `contract_version`.
- [ ] Credential rotation invalidates old key within one consumer-poll cycle.
- [ ] Disabling ingress revokes credentials immediately; streams drained and deleted after drain window.
- [ ] `cargo test` green; no existing tests broken.
