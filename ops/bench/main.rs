//! In-process benchmark of the validation hot path.
//!
//! Times [`contractgate::validation::validate`] on the public Stripe charges
//! catalog contract and its committed sample events. Compile happens once,
//! before the timer. One thread. No HTTP, auth, or database.
//!
//! Release numbers are the ones to cite. A debug build still runs, and says so.
//!
//! ```text
//! cargo run --release --bin contractgate-bench
//! ```

use anyhow::{bail, Context, Result};
use contractgate::contract::Contract;
use contractgate::validation::{validate, CompiledContract};
use serde_json::Value;
use std::hint::black_box;
use std::time::{Duration, Instant};

const CONTRACT_YAML: &str = include_str!("../../catalog/seeds/stripe-charges/contract.yaml");
const SAMPLES_JSON: &str = include_str!("../../catalog/seeds/stripe-charges/samples.json");

/// Warm the instruction cache and branch predictor before measuring.
const WARMUP: usize = 50_000;
/// Per-call samples for the percentile lines, and iterations for the tight loop.
const ITERS: usize = 100_000;

fn main() {
    if let Err(err) = run() {
        eprintln!("contractgate-bench: {err:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let (compiled, samples) = load()?;
    for (index, sample) in samples.iter().enumerate() {
        let result = validate(&compiled, sample);
        if !result.passed {
            bail!(
                "catalog sample {index} failed validation: {:?}",
                result.violations
            );
        }
    }
    let failing = failing_charge(&samples[0])?;
    let failed = validate(&compiled, &failing);
    if failed.passed {
        bail!("negative amount was accepted; the failing workload is not a failure");
    }

    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    if profile == "debug" {
        eprintln!(
            "contractgate-bench: this is a debug build. Do not cite these numbers. \
             Re-run: cargo run --release --bin contractgate-bench"
        );
    }

    warmup(&compiled, &samples);
    let pass = measure(&compiled, &samples);
    warmup(&compiled, std::slice::from_ref(&failing));
    let fail = measure(&compiled, std::slice::from_ref(&failing));

    println!("command: cargo run --release --bin contractgate-bench");
    println!("profile: {profile}");
    println!("threads: 1");
    println!("cpu: {}", cpu_line());
    println!("contract: catalog/seeds/stripe-charges/contract.yaml");
    println!(
        "pass_events: catalog/seeds/stripe-charges/samples.json ({} samples, round-robin)",
        samples.len()
    );
    println!("fail_event: first catalog sample with amount set to -1 (one violation)");
    println!("warmup_iters: {WARMUP}");
    println!("measured_iters: {ITERS}");
    println!("pass_p50_us: {}", pass.p50_us);
    println!("pass_p95_us: {}", pass.p95_us);
    println!("pass_p99_us: {}", pass.p99_us);
    println!("pass_mean_us: {}", pass.mean_us);
    println!("pass_max_us: {}", pass.max_us);
    println!("pass_events_per_sec: {}", pass.events_per_sec);
    println!("fail_p50_us: {}", fail.p50_us);
    println!("fail_p95_us: {}", fail.p95_us);
    println!("fail_p99_us: {}", fail.p99_us);
    println!("fail_mean_us: {}", fail.mean_us);
    println!("fail_max_us: {}", fail.max_us);
    println!("fail_events_per_sec: {}", fail.events_per_sec);
    Ok(())
}

struct Report {
    p50_us: String,
    p95_us: String,
    p99_us: String,
    mean_us: String,
    max_us: String,
    events_per_sec: String,
}

fn load() -> Result<(CompiledContract, Vec<Value>)> {
    let contract: Contract =
        serde_yaml::from_str(CONTRACT_YAML).context("parse stripe-charges contract")?;
    let compiled = CompiledContract::compile(contract).context("compile stripe-charges")?;
    let samples: Vec<Value> =
        serde_json::from_str(SAMPLES_JSON).context("parse stripe-charges samples")?;
    if samples.is_empty() {
        bail!("stripe-charges samples.json is empty");
    }
    Ok((compiled, samples))
}

fn failing_charge(sample: &Value) -> Result<Value> {
    let mut event = sample.clone();
    let Some(obj) = event.as_object_mut() else {
        bail!("catalog sample is not a JSON object");
    };
    obj.insert("amount".to_string(), serde_json::json!(-1));
    Ok(event)
}

fn warmup(compiled: &CompiledContract, events: &[Value]) {
    for i in 0..WARMUP {
        let event = &events[i % events.len()];
        black_box(validate(black_box(compiled), black_box(event)));
    }
}

fn measure(compiled: &CompiledContract, events: &[Value]) -> Report {
    let mut samples_ns = Vec::with_capacity(ITERS);
    for i in 0..ITERS {
        let event = &events[i % events.len()];
        let started = Instant::now();
        black_box(validate(black_box(compiled), black_box(event)));
        samples_ns.push(ns_u64(started.elapsed()));
    }
    samples_ns.sort_unstable();

    let started = Instant::now();
    for i in 0..ITERS {
        let event = &events[i % events.len()];
        black_box(validate(black_box(compiled), black_box(event)));
    }
    let elapsed = started.elapsed();

    Report {
        p50_us: format_us_ceil(percentile_ns(&samples_ns, 50, 100)),
        p95_us: format_us_ceil(percentile_ns(&samples_ns, 95, 100)),
        p99_us: format_us_ceil(percentile_ns(&samples_ns, 99, 100)),
        mean_us: format_us_ceil(mean_ns_ceil(&samples_ns)),
        max_us: format_us_ceil(*samples_ns.last().unwrap_or(&0)),
        events_per_sec: format_rate_floor(ITERS, elapsed),
    }
}

fn ns_u64(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}

/// Nearest-rank percentile. `sorted` must be ascending and non-empty.
/// Rank is `ceil(numerator / denominator * n)`, then the index is rank − 1.
/// Integer arithmetic so 0.99 × 100_000 cannot land on the wrong side of an integer.
fn percentile_ns(sorted: &[u64], numerator: u64, denominator: u64) -> u64 {
    let n = sorted.len() as u64;
    let rank = (numerator * n).div_ceil(denominator);
    let index = rank.saturating_sub(1).min(n.saturating_sub(1)) as usize;
    sorted[index]
}

fn mean_ns_ceil(samples: &[u64]) -> u64 {
    let n = samples.len() as u128;
    let sum: u128 = samples.iter().map(|ns| u128::from(*ns)).sum();
    u64::try_from(sum.div_ceil(n)).unwrap_or(u64::MAX)
}

/// Microseconds rounded up to 0.01 so a printed latency is never faster than measured.
fn format_us_ceil(ns: u64) -> String {
    let hundredths = ns.div_ceil(10);
    format!("{:.2}", hundredths as f64 / 100.0)
}

/// Events/sec rounded down so a printed rate is never faster than measured.
fn format_rate_floor(events: usize, elapsed: Duration) -> String {
    let nanos = elapsed.as_nanos();
    if nanos == 0 {
        return "0".to_string();
    }
    // events/sec = events * 1e9 / nanos, rounded down.
    let rate = (events as u128) * 1_000_000_000 / nanos;
    format!("{rate}")
}

fn cpu_line() -> String {
    let Ok(text) = std::fs::read_to_string("/proc/cpuinfo") else {
        return "unavailable (bench uses 1 thread)".to_string();
    };
    let mut model = None;
    let mut family = None;
    let mut model_num = None;
    let mut stepping = None;
    let mut mhz = None;
    let mut cpus = 0u32;
    for line in text.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        match key {
            "processor" => cpus += 1,
            "model name" if model.is_none() => model = Some(value.to_string()),
            "cpu family" if family.is_none() => family = Some(value.to_string()),
            "model" if model_num.is_none() => model_num = Some(value.to_string()),
            "stepping" if stepping.is_none() => stepping = Some(value.to_string()),
            "cpu MHz" if mhz.is_none() => mhz = Some(value.to_string()),
            _ => {}
        }
    }
    let model = model.unwrap_or_else(|| "unknown model".to_string());
    let ident = match (family, model_num, stepping) {
        (Some(f), Some(m), Some(s)) => format!(", family {f} model {m} stepping {s}"),
        _ => String::new(),
    };
    let mhz = mhz.map(|h| format!(" @ {h} MHz")).unwrap_or_default();
    format!("{model}{ident}{mhz}, {cpus} logical CPUs (bench uses 1 thread)")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearest_rank_matches_the_documented_index() {
        let sorted: Vec<u64> = (1..=100).collect();
        // ceil(50/100 * 100) - 1 = 49 → 50
        assert_eq!(percentile_ns(&sorted, 50, 100), 50);
        // ceil(99/100 * 100) - 1 = 98 → 99
        assert_eq!(percentile_ns(&sorted, 99, 100), 99);
        // ceil(95/100 * 100) - 1 = 94 → 95
        assert_eq!(percentile_ns(&sorted, 95, 100), 95);
        // 99/100 of 100_000 is exact, so the index is 98_999 (value 99_000).
        let big: Vec<u64> = (1..=100_000).collect();
        assert_eq!(percentile_ns(&big, 99, 100), 99_000);
    }

    #[test]
    fn latency_print_rounds_up_and_rate_rounds_down() {
        assert_eq!(format_us_ceil(1000), "1.00");
        assert_eq!(format_us_ceil(1001), "1.01");
        assert_eq!(format_us_ceil(1234), "1.24");
        let rate = format_rate_floor(1_000, Duration::from_micros(10_500));
        // 1000 / 0.0105 s = 95238.095... → 95238
        assert_eq!(rate, "95238");
    }

    #[test]
    fn stripe_charges_samples_pass_and_negative_amount_fails() {
        let (compiled, samples) = load().unwrap();
        assert_eq!(samples.len(), 3);
        for sample in &samples {
            assert!(validate(&compiled, sample).passed);
        }
        let failing = failing_charge(&samples[0]).unwrap();
        assert!(!validate(&compiled, &failing).passed);
    }
}
