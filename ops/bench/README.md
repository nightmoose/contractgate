# Validation benchmark

Times the in-process hot path: compile the public catalog contract
[`catalog/seeds/stripe-charges/contract.yaml`](../../catalog/seeds/stripe-charges/contract.yaml)
once, then call `validate()` on one thread. The passing workload rotates the
three events in
[`catalog/seeds/stripe-charges/samples.json`](../../catalog/seeds/stripe-charges/samples.json).
A second workload is the first of those events with `amount` set to `-1`, which
returns one violation.

This does not start the HTTP server and does not open a database. It is not
end-to-end ingest latency.

## Prerequisites

- A clone of this repository.
- Rust, via [rustup](https://rustup.rs/). [`rust-toolchain.toml`](../../rust-toolchain.toml) pins the toolchain (1.98.0 at the time of the recorded run). The first `cargo` invocation installs that toolchain if it is missing.
- Network access on the first build, so Cargo can fetch the crates in `Cargo.lock`.

No C compiler, Docker, Postgres, or API key.

Run the command from the repository root.

## Command

```bash
cargo run --release --bin contractgate-bench
```

`make bench` runs that same command.

The package release profile is `opt-level = 3`, `lto = true`, `codegen-units = 1`. The first release build is the slow part. Later runs reuse `target/release/contractgate-bench`.

A debug build (`cargo run --bin contractgate-bench`, without `--release`) prints a warning on stderr and `profile: debug`. Do not cite it.

## How to read the output

The tool prints one `key: value` line per fact.

| Line | Meaning |
|---|---|
| `profile` | `release` is the only profile to cite. |
| `threads` | Always `1`. The process does not pin itself to a CPU, and it does not use the other CPUs. |
| `cpu` | Model, family, stepping, MHz, and logical CPU count from `/proc/cpuinfo` when that file exists. Otherwise `unavailable`. |
| `pass_p50_us`, `pass_p95_us`, `pass_p99_us` | Nearest-rank percentiles of per-call wall time for the passing samples, in microseconds, rounded **up** to 0.01. Rank is `ceil(p × N)` using integer arithmetic, then the index is rank − 1, on 100,000 sorted samples. |
| `pass_mean_us` | Mean of those samples, rounded **up** to 0.01 µs. |
| `pass_max_us` | Slowest single passing call in the sample, rounded **up**. Scheduling noise shows up here. It is not the p99. |
| `pass_events_per_sec` | Passing throughput from a second loop of 100,000 calls with no per-call timer, rounded **down** to an integer. This is the throughput figure. |
| `fail_*` | The same statistics for the negative-amount event. |

`pass_p99_us` is the latency figure. `pass_events_per_sec` is the throughput figure. They are different loops: the latency loop includes a timer around each call, and the throughput loop does not. Docs that cite a single number use the passing workload.

Each run warms up for 50,000 calls before measuring. Inputs and the `validate()` result are passed through `std::hint::black_box`.

## Recorded run

Date: 2026-11-10.

Command, from the repository root, after the release binary was already built:

```bash
cargo run --release --bin contractgate-bench
```

Toolchain: rustc 1.98.0 (88d9e12ae 2026-08-18), from `rust-toolchain.toml`.

Hardware (`lscpu` on the machine that produced the numbers below):

- x86_64, KVM guest
- Model name: Intel(R) Xeon(R) Processor
- CPU family 6, model 207, stepping 2
- 4 logical CPUs, 1 thread per core, 1 socket
- cpu MHz: 2400.000
- The bench uses 1 thread

Three consecutive release runs:

```text
===== RUN 1 =====
command: cargo run --release --bin contractgate-bench
profile: release
threads: 1
cpu: Intel(R) Xeon(R) Processor, family 6 model 207 stepping 2 @ 2400.000 MHz, 4 logical CPUs (bench uses 1 thread)
contract: catalog/seeds/stripe-charges/contract.yaml
pass_events: catalog/seeds/stripe-charges/samples.json (3 samples, round-robin)
fail_event: first catalog sample with amount set to -1 (one violation)
warmup_iters: 50000
measured_iters: 100000
pass_p50_us: 0.67
pass_p95_us: 0.78
pass_p99_us: 0.92
pass_mean_us: 0.67
pass_max_us: 16.79
pass_events_per_sec: 1547368
fail_p50_us: 0.87
fail_p95_us: 1.07
fail_p99_us: 1.13
fail_mean_us: 0.89
fail_max_us: 16.98
fail_events_per_sec: 1175240

===== RUN 2 =====
command: cargo run --release --bin contractgate-bench
profile: release
threads: 1
cpu: Intel(R) Xeon(R) Processor, family 6 model 207 stepping 2 @ 2400.000 MHz, 4 logical CPUs (bench uses 1 thread)
contract: catalog/seeds/stripe-charges/contract.yaml
pass_events: catalog/seeds/stripe-charges/samples.json (3 samples, round-robin)
fail_event: first catalog sample with amount set to -1 (one violation)
warmup_iters: 50000
measured_iters: 100000
pass_p50_us: 0.67
pass_p95_us: 0.78
pass_p99_us: 0.93
pass_mean_us: 0.67
pass_max_us: 23.70
pass_events_per_sec: 1550147
fail_p50_us: 0.87
fail_p95_us: 1.02
fail_p99_us: 1.27
fail_mean_us: 0.89
fail_max_us: 18.38
fail_events_per_sec: 1179724

===== RUN 3 =====
command: cargo run --release --bin contractgate-bench
profile: release
threads: 1
cpu: Intel(R) Xeon(R) Processor, family 6 model 207 stepping 2 @ 2400.000 MHz, 4 logical CPUs (bench uses 1 thread)
contract: catalog/seeds/stripe-charges/contract.yaml
pass_events: catalog/seeds/stripe-charges/samples.json (3 samples, round-robin)
fail_event: first catalog sample with amount set to -1 (one violation)
warmup_iters: 50000
measured_iters: 100000
pass_p50_us: 0.67
pass_p95_us: 0.77
pass_p99_us: 0.92
pass_mean_us: 0.67
pass_max_us: 17.29
pass_events_per_sec: 1560898
fail_p50_us: 0.89
fail_p95_us: 1.09
fail_p99_us: 1.27
fail_mean_us: 0.93
fail_max_us: 14.85
fail_events_per_sec: 1153482
```

Figures cited elsewhere in the repo are the slower end of these three runs, so a single quoted number is not faster than any of the runs:

- Passing p99: **0.93 µs** (`pass_p99_us` on run 2)
- Passing throughput: **1,547,368 events/sec** (`pass_events_per_sec` on run 1)
- Failing p99: **1.27 µs**
- Failing throughput: **1,153,482 events/sec**

Another machine will print different numbers. Use that machine's output for that machine. The committed claim is this host only.
