#!/usr/bin/env bash
# Produce a passing and a failing event through the demo validator against
# a vanilla Apache Kafka 4.x container. No Confluent image, no license key.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

if ! docker info >/dev/null 2>&1; then
  docker() { sudo docker "$@"; }
fi

NAME="${KAFKA_CONTAINER_NAME:-contractgate-kafka-it}"
PORT="${KAFKA_IT_PORT:-19092}"
IMAGE="${KAFKA_IMAGE:-apache/kafka:4.1.0}"

cleanup() {
  docker rm -f "$NAME" >/dev/null 2>&1 || true
}
trap cleanup EXIT

docker rm -f "$NAME" >/dev/null 2>&1 || true
docker run -d --name "$NAME" -p "${PORT}:9092" \
  -e KAFKA_NODE_ID=1 \
  -e KAFKA_PROCESS_ROLES=broker,controller \
  -e KAFKA_LISTENERS=PLAINTEXT://0.0.0.0:9092,CONTROLLER://0.0.0.0:9093 \
  -e KAFKA_ADVERTISED_LISTENERS=PLAINTEXT://127.0.0.1:${PORT} \
  -e KAFKA_LISTENER_SECURITY_PROTOCOL_MAP=CONTROLLER:PLAINTEXT,PLAINTEXT:PLAINTEXT \
  -e KAFKA_CONTROLLER_LISTENER_NAMES=CONTROLLER \
  -e KAFKA_CONTROLLER_QUORUM_VOTERS=1@127.0.0.1:9093 \
  -e KAFKA_OFFSETS_TOPIC_REPLICATION_FACTOR=1 \
  -e KAFKA_TRANSACTION_STATE_LOG_REPLICATION_FACTOR=1 \
  -e KAFKA_TRANSACTION_STATE_LOG_MIN_ISR=1 \
  -e KAFKA_GROUP_INITIAL_REBALANCE_DELAY_MS=0 \
  -e CLUSTER_ID=MkU3OEVBNTcwNTJENDM2Qk \
  "$IMAGE"

echo "waiting for $IMAGE on 127.0.0.1:${PORT}"
ready=0
for _ in $(seq 1 60); do
  if docker exec "$NAME" /opt/kafka/bin/kafka-broker-api-versions.sh --bootstrap-server 127.0.0.1:9092 >/dev/null 2>&1; then
    ready=1
    break
  fi
  sleep 2
done
if [[ "$ready" != 1 ]]; then
  echo "Kafka did not become ready" >&2
  docker logs "$NAME" >&2 || true
  exit 1
fi

export KAFKA_BROKERS="127.0.0.1:${PORT}"
export CC="${CC:-gcc}"
export CXX="${CXX:-g++}"
cargo test --features demo --bin demo -- --ignored kafka_pipeline_pass_and_quarantine --nocapture
