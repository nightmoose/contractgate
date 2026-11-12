# RFC-115 — Native Kafka client

**Status:** Shipped
**Date:** 2026-11-12

## Problem

Kafka support was an optional `rdkafka` dependency. That crate builds
librdkafka with cmake and a C compiler, plus SASL and compression headers.
The `demo`, `scaffold`, and `kafka-ingress` features all paid that cost.

## Decision

Use `confluent-kafka` 1.0.0 (crates.io, Apache-2.0) for every Kafka call:
the ingress consumer and producer, topic admin, the scaffold sampler, and
the speed-demo binary.

The crate is a Rust implementation of the Kafka protocol. It does not
require a Confluent account or a license key. Confluent Cloud remains one
cluster the hosted ingress feature can use (`SASL_SSL` / `PLAIN`). The
integration test runs against the `apache/kafka` image, not a Confluent
image.

Consumer groups use the KIP-848 protocol (`group.protocol=consumer`).
Brokers must be Apache Kafka 4.0 or newer. That includes Confluent Cloud
and Amazon MSK 4.x. Redpanda, as of the 26.2 line, does not implement
KIP-848, so the speed demo broker is Apache Kafka 4.1.

SASL/PLAIN and PEM TLS (including client certificates when the paths are
set) are supported. SASL/SCRAM is not. The scaffolder returns an error for
`--auth sasl-scram-256` and `sasl-scram-512` instead of negotiating a
mechanism the client cannot speak. The documented scaffold auth remains
PLAINTEXT and SASL/PLAIN.

`librdkafka` build packages (`libsasl2-dev`, `libssl-dev`, `libcurl`,
lz4/snappy/zstd system headers) leave the gateway Dockerfile. cmake and a
C compiler stay because `aws-lc-rs` (the TLS provider) and the `zstd` codec
compile bundled C. That is not a system Kafka library.

Offset commit behavior is unchanged: ingress and the speed demo keep
`enable.auto.commit=true`; the scaffolder keeps it false and seeks manually
so it never commits.

## Non-goals

- Changing the hosted ingress product off Confluent Cloud.
- Adding SCRAM or a classic (`group.protocol=classic`) consumer.
- Deploying this change.
