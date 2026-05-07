# RFC-025: Kafka Ingress for Hosted ContractGate

| Field    | Value                  |
|----------|------------------------|
| Status   | **Draft**              |
| Author   | ContractGate team      |
| Created  | 2026-05-07             |
| Branch   | `nightly-maintenance-2026-05-07` |

---

## Problem

Users with Kafka-based pipelines cannot route events through ContractGate
without building their own consumer bridge. The current HTTP ingest path
works, but it forces teams to retool producers or insert a sidecar. Native
Kafka ingress removes that friction.

---

## Proposal

Hosted ContractGate provisions per-contract Kafka topics and runs consumers
on the platform side. Users produce to a well-known input topic; ContractGate
validates each event and routes it to either a clean output topic or a
quarantine topic — exactly the existing HTTP validation semantics, surfaced
over Kafka.

---

## Design

### 1. Topic Naming

| Purpose    | Topic name                             |
|------------|----------------------------------------|
| Raw input  | `cg-{contract_id}-raw`                 |
| Clean out  | `cg-{contract_id}-clean`               |
| Quarantine | `cg-{contract_id}-quarantine`          |

`contract_id` is the existing UUID already stored in Supabase. No new
identifier is introduced.

### 2. Dashboard Toggle

A single **"Enable Kafka Ingress"** toggle appears on the contract detail
page. On enable, the backend:

1. Provisions the three topics (or marks them active in a topic registry table).
2. Creates or reuses a scoped credential set (SASL/SCRAM username + password).
3. Returns bootstrap server address and credentials to the dashboard for display.

On disable, credential revocation happens immediately; topics are soft-deleted
after a configurable drain window (default 24 h) to avoid losing in-flight
events.

### 3. Broker

**Confluent Cloud.** Topic and credential provisioning uses the Confluent
Admin API. Bootstrap servers follow the standard Confluent format
(`pkc-<id>.<region>.confluent.cloud:9092`). SASL mechanism: PLAIN over TLS
(Confluent Cloud standard — no SCRAM negotiation needed).

### 4. Consumer Architecture

A shared `rdkafka` consumer pool runs platform-side. Each enabled contract
gets one consumer group (`cg-consumer-{contract_id}`) subscribed to its raw
topic. Partition count defaults to **3**; the pool assigns one worker thread
per partition. Worker count scales with active contracts — idle contracts
(no messages for 5 min) release their threads back to the pool without
dropping the consumer group assignment, so lag tracking stays continuous.

This avoids per-contract thread proliferation at low traffic while keeping
latency predictable under load. A future quota RFC can expose partition count
as a billing-tier knob without changing this architecture.

```
Producer (user)
    │
    ▼
cg-{contract_id}-raw   (Confluent Cloud)
    │
    ▼
ContractGate Consumer pool (rdkafka / Tokio workers)
    │
    ├─ valid   ──▶  cg-{contract_id}-clean
    └─ invalid ──▶  cg-{contract_id}-quarantine
                        + cg-violation-reason header
                        + original payload preserved
```

Violation metadata mirrors the existing HTTP 422 response body written as a
Kafka message header (`cg-violation-reason`) — quarantined events are never
mutated or dropped.

### 5. Clean Topic — No Skip Mode

Users always receive routed output on the clean topic. Header-only mode
(consuming raw with validation signals) is **not supported** in this RFC.
Rationale: it adds a configuration surface, diverges from the HTTP contract
semantics, and the primary use case is full routing. Users who want to
consume raw events can simply ignore the clean topic — Confluent's retention
will handle cleanup.

### 6. Credentials

- SASL/PLAIN over TLS (Confluent Cloud standard), one API key per contract.
- Confluent ACLs scope the key: produce on `cg-{contract_id}-raw`; consume
  on `cg-{contract_id}-clean` and `cg-{contract_id}-quarantine`.
- Stored encrypted in Supabase; displayed once in the dashboard (copy-on-reveal
  pattern, same UX as existing API keys).
- Rotation: Confluent API creates new key → update `kafka_ingress` row →
  delete old Confluent key. Old key invalid within one poll cycle.

### 7. Supabase Schema Additions

```sql
-- tracks provisioned kafka configs per contract
create table kafka_ingress (
  id                  uuid primary key default gen_random_uuid(),
  contract_id         uuid not null references contracts(id) on delete cascade,
  enabled             boolean not null default false,
  confluent_bootstrap text not null,      -- pkc-<id>.<region>.confluent.cloud:9092
  confluent_api_key   text not null,
  confluent_api_secret_enc text not null, -- encrypted at rest
  partition_count     int not null default 3,
  drain_window_hours  int not null default 24,
  created_at          timestamptz default now(),
  updated_at          timestamptz default now()
);

-- audit log reuses existing audit_logs table; source = 'kafka'
```

RLS: same org-scoped helper (`get_my_org_ids()`) as all other tables —
no inline subqueries.

### 8. Audit Log

No new table. Each event processed via Kafka writes to `audit_logs` with
`source = 'kafka'` and `contract_version` set to the version that actually
matched (no defaulting — existing audit honesty rule applies).

### 9. Validation Engine

Zero changes. The consumer calls the same `validate_event()` function the
HTTP handler calls. p99 budget for validation itself remains <15 ms; Kafka
consumer overhead is outside that budget.

---

## What This RFC Does Not Cover

| Deferred topic                | Rationale                                     |
|-------------------------------|-----------------------------------------------|
| Self-hosted Kafka support     | Users bring their own broker; different auth  |
| Per-topic retention config    | Defer until quota RFC lands                   |
| Schema Registry integration   | Avro/Protobuf deserialization — separate RFC  |
| Billing metering per message  | Depends on quota RFC                          |
| Multi-region topic replication| Infrastructure concern, post-GA               |

---

## Resolved Decisions

| Question | Decision |
|---|---|
| Broker vendor | Confluent Cloud — Admin API for provisioning, SASL/PLAIN over TLS |
| Consumer scaling | Shared pool; 3 partitions/contract default; idle threads released after 5 min |
| Clean topic skip | Not supported — always full routing; deferred indefinitely |
| Drain window | 24 h default; stored per-row so it can be adjusted without migration |

---

## Acceptance Criteria

- [ ] `kafka_ingress` table migrated and covered by RLS.
- [ ] Toggle in dashboard provisions topics and returns credentials.
- [ ] Consumer reads from raw topic, calls existing validator, routes correctly.
- [ ] Quarantine messages contain original payload + `cg-violation-reason` header.
- [ ] Audit logs written with `source = 'kafka'` and correct `contract_version`.
- [ ] Credential rotation invalidates old credentials within one consumer-poll cycle.
- [ ] Disabling ingress revokes credentials immediately; topics drained and removed after drain window.
- [ ] `cargo test` green; no existing tests broken.
