# ContractGate — Speed Demo

A self-contained, Kafka-backed demo that showcases ContractGate's validation
throughput at volume, side-by-side with a zero-validation baseline.

## What it does

The demo spins up three concurrent pipelines sharing a single Kafka cluster
(Redpanda running in Docker):

```
                                        ┌─► demo.events.valid
                                        │
    [event generator]                   │
         │                              │
         ▼          ┌──[ContractGate]───┤
   demo.events.in ──┤                   └─► demo.events.quarantine
                    │
                    └──[straight-copy baseline]──► demo.events.copy
```

Both the ContractGate validator and the straight-copy baseline consume the
**exact same** stream off `demo.events.in`, in their own consumer groups.
Any throughput delta between them is the real cost of semantic validation —
not network, not serialization, not broker overhead.

The embedded web dashboard (`http://localhost:8088`) shows live:

- **ContractGate throughput** — events/sec the validator is processing
- **Overhead vs baseline** — `100 − (validator_rate / copy_rate) × 100`. 0%
  means ContractGate is keeping perfect pace with a zero-work passthrough.
  Colour-coded green (≤5%), amber (≤20%), red beyond. This is the headline
  pitch number.
- **Validation cost (p99)** — `validator_p99 − copy_p99` in µs. What
  ContractGate actually *adds* on top of what Kafka + your laptop pay for a
  straight copy of the same bytes.
- Producer rate (events/sec being pushed onto Kafka) and total sent
- Per-lane throughput, latency percentiles (p50 / p95 / p99 / max), and total
  events forwarded downstream
- Pass/fail split on the ContractGate lane
- Rolling throughput chart covering the last ~24s

## Prerequisites

- **Docker** (to run Redpanda via `docker compose`)
- **Rust toolchain** (`rustup` — stable)
- **CMake + a C compiler** — required by `rdkafka`'s bundled `librdkafka`
  build. On macOS: `xcode-select --install` + `brew install cmake`. On
  Debian/Ubuntu: `sudo apt install build-essential cmake`.

## Run it

```bash
# 1. Start Redpanda (Kafka-compatible, single container, no ZooKeeper)
docker compose up -d

# 2. Build + run the demo (release mode matters — validation is that fast)
cargo run --release --bin demo

# 3. Open the dashboard
open http://localhost:8088
```

Pick a scenario, set a fail ratio, optionally cap the producer rate, and hit
**Start**. Stop any time with **Stop** or let the duration expire.

### Optional: Kafka inspection UI

`docker-compose.yml` also starts **Redpanda Console** at
`http://localhost:8080`. It lets you browse the `demo.events.*` topics live
while the demo runs — handy for verifying that pass events really are
landing on `demo.events.valid` and fail events on `demo.events.quarantine`.

## What to look at

**Throughput chart.** The two main series (blue = ContractGate, amber =
baseline) should track each other closely. That visual parity is the pitch:
ContractGate's overhead is effectively constant per event, so at volume
there's nothing left to catch up on.

**Latency percentiles.** Both lanes measure end-to-end latency from the
producer's wall clock until the consumer is done handling the record. The
baseline tells you how much of that is Kafka + your laptop; the difference
is ContractGate.

**Pass/fail bar.** Scroll the fail ratio up to 50% and watch the bar split.
Failed events are published to `demo.events.quarantine` with the full
violation list attached — exactly the quarantine contract that'll run in
production.

## Scenarios

Both contracts live at `demo/scenarios/*.yaml` and are embedded into the
binary at compile time (`include_str!`). Edit them and rebuild to try your
own.

- **simple** (`user_events_simple`) — flat, five fields, a regex, an enum,
  a numeric range. Fast-path baseline.
- **nested** (`order_events_nested`) — e-commerce order with a customer
  object, nested address, and an array of line items. Each item is itself an
  object with pattern/range constraints. The realistic stress case.

## CLI flags

```bash
cargo run --release --bin demo -- \
    --brokers localhost:9092 \
    --port 8088 \
    --snapshot-interval-ms 200
```

Also honours `KAFKA_BROKERS` via environment.

## Tearing down

```bash
docker compose down        # keeps the named volume — subsequent runs reuse state
docker compose down -v     # wipes everything
```

## Architecture notes

- **Single process, three tasks.** The binary runs the producer loop, both
  consumer lanes, and the HTTP server inside one Tokio runtime. No external
  orchestration needed.
- **Shared `FutureProducer`.** All three Kafka-side callers share one
  `rdkafka` producer with aggressive batching (`queue.buffering.max.ms=5`,
  `compression.type=lz4`, `acks=1`).
- **Consumer groups.** `contractgate-validator` and `contractgate-copy-baseline`
  are two distinct groups on the same input topic, so each lane gets every
  record independently.
- **Sampled latency histogram.** To keep the hot path cheap at very high
  rates, each lane records into an `hdrhistogram` on a 1-in-8 sample — plenty
  for stable p50 / p95 / p99 estimates without a per-event lock.
- **Stats fan-out.** Every `--snapshot-interval-ms` (default 200 ms) a
  snapshotter serialises the counters and pushes them onto a `tokio::sync::broadcast`
  channel.  The `/api/stream` SSE endpoint subscribes each dashboard tab to
  that channel.
