//! Streaming field profiler for the brownfield scaffolder (RFC-024 §C).
//!
//! Single-pass over sampled JSON values; no materialisation of all records
//! simultaneously.  Memory budget is enforced per-profiler instance.
//!
//! Developer tooling — not part of the patent-core validation engine.

use std::collections::HashMap;

// ---------------------------------------------------------------------------
// HyperLogLog++ (precision 12 — ~4096 registers, ~1.6% error)
// ---------------------------------------------------------------------------

const HLL_P: u32 = 12;
const HLL_M: usize = 1 << HLL_P; // 4096
/// Bias-correction constant α_m for m = 4096.
const HLL_ALPHA: f64 = 0.7213 / (1.0 + 1.079 / HLL_M as f64);

/// Lightweight HyperLogLog cardinality estimator.
/// Uses `std::collections::hash_map::DefaultHasher` (non-cryptographic, good
/// distribution for our purpose).
#[derive(Clone)]
pub struct HyperLogLog {
    registers: Vec<u8>, // 4096 bytes
}

impl HyperLogLog {
    pub fn new() -> Self {
        Self {
            registers: vec![0u8; HLL_M],
        }
    }

    /// Add a string value to the sketch.
    pub fn insert(&mut self, value: &str) {
        // Apply splitmix64 finalizer: FNV-1a has poor avalanche in the top bits
        // for short sequential strings, causing register clustering.
        let hash = cheap_hash(fnv64(value.as_bytes()));
        // Top P bits → register index.
        let idx = (hash >> (64 - HLL_P)) as usize;
        // Remaining 64-P bits; count position of leftmost 1-bit (+1).
        let remaining = (hash << HLL_P) | (1 << (HLL_P - 1)); // guard against all-zero
        let rho = remaining.leading_zeros() + 1;
        let rho = rho.min(64) as u8;
        if rho > self.registers[idx] {
            self.registers[idx] = rho;
        }
    }

    /// Estimate the number of distinct values inserted.
    pub fn estimate(&self) -> u64 {
        let sum: f64 = self
            .registers
            .iter()
            .map(|&r| 2.0_f64.powi(-(r as i32)))
            .sum();
        let raw = HLL_ALPHA * (HLL_M as f64).powi(2) / sum;

        // Small-range correction: linear counting when many registers are 0.
        let zeros = self.registers.iter().filter(|&&r| r == 0).count() as f64;
        if raw <= 2.5 * HLL_M as f64 && zeros > 0.0 {
            return (HLL_M as f64 * (HLL_M as f64 / zeros).ln()) as u64;
        }

        raw as u64
    }
}

impl Default for HyperLogLog {
    fn default() -> Self {
        Self::new()
    }
}

/// FNV-1a 64-bit hash — fast, good distribution, no external dep.
fn fnv64(data: &[u8]) -> u64 {
    const FNV_OFFSET: u64 = 14695981039346656037;
    const FNV_PRIME: u64 = 1099511628211;
    let mut h = FNV_OFFSET;
    for &b in data {
        h ^= b as u64;
        h = h.wrapping_mul(FNV_PRIME);
    }
    h
}

// ---------------------------------------------------------------------------
// Per-field statistics
// ---------------------------------------------------------------------------

/// Summarised statistics for a single field across all sampled events.
#[derive(Debug, Clone)]
pub struct FieldStats {
    pub name: String,
    pub total_count: u64,
    pub null_count: u64,
    /// Approximate distinct-value count (HyperLogLog++ estimate).
    pub distinct_estimate: u64,
    /// Numeric min (for integer/float fields).
    pub numeric_min: Option<f64>,
    /// Numeric max (for integer/float fields).
    pub numeric_max: Option<f64>,
    /// Approximate p5 string-length percentile (reservoir sampled, cap 1000).
    pub length_p5: Option<usize>,
    /// Approximate p50 string-length percentile.
    pub length_p50: Option<usize>,
    /// Approximate p95 string-length percentile.
    pub length_p95: Option<usize>,
    /// Top-k values by frequency (exact, up to saturation limit).
    /// Empty when `top_k_saturated` is true.
    pub top_k: Vec<(String, u64)>,
    /// True when distinct count exceeded TOP_K_CAP — exact counts are not kept.
    pub top_k_saturated: bool,
}

impl FieldStats {
    /// Null rate in [0.0, 1.0].
    pub fn null_rate(&self) -> f64 {
        if self.total_count == 0 {
            0.0
        } else {
            self.null_count as f64 / self.total_count as f64
        }
    }
}

// ---------------------------------------------------------------------------
// Mutable accumulator (internal)
// ---------------------------------------------------------------------------

const LENGTH_RESERVOIR_CAP: usize = 1_000;
const TOP_K_CAP: usize = 500; // switch to saturated above this many distinct
const TOP_K_EMIT: usize = 20; // emit at most this many in the final top_k list

struct FieldAccum {
    name: String,
    total: u64,
    nulls: u64,
    hll: HyperLogLog,
    numeric_min: Option<f64>,
    numeric_max: Option<f64>,
    /// Reservoir sample of string lengths (bounded).
    length_reservoir: Vec<usize>,
    length_reservoir_seen: u64,
    /// Exact value → count (up to TOP_K_CAP distinct values).
    value_counts: HashMap<String, u64>,
    top_k_saturated: bool,
}

impl FieldAccum {
    fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            total: 0,
            nulls: 0,
            hll: HyperLogLog::new(),
            numeric_min: None,
            numeric_max: None,
            length_reservoir: Vec::new(),
            length_reservoir_seen: 0,
            value_counts: HashMap::new(),
            top_k_saturated: false,
        }
    }

    fn record(&mut self, value: &serde_json::Value, rng_seed: u64) {
        self.total += 1;

        use serde_json::Value::*;
        match value {
            Null => {
                self.nulls += 1;
            }
            Number(n) => {
                if let Some(f) = n.as_f64() {
                    self.hll.insert(&f.to_bits().to_string());
                    self.numeric_min = Some(match self.numeric_min {
                        Some(m) => m.min(f),
                        None => f,
                    });
                    self.numeric_max = Some(match self.numeric_max {
                        Some(m) => m.max(f),
                        None => f,
                    });
                    self.record_top_k(&f.to_string());
                }
            }
            Bool(b) => {
                let s = b.to_string();
                self.hll.insert(&s);
                self.record_top_k(&s);
            }
            String(s) => {
                self.hll.insert(s);
                // Length reservoir (Vitter's Algorithm R).
                let len = s.len();
                self.length_reservoir_seen += 1;
                if self.length_reservoir.len() < LENGTH_RESERVOIR_CAP {
                    self.length_reservoir.push(len);
                } else {
                    // Simple deterministic pseudo-random slot selection.
                    let slot = cheap_hash(rng_seed ^ self.length_reservoir_seen)
                        % LENGTH_RESERVOIR_CAP as u64;
                    self.length_reservoir[slot as usize] = len;
                }
                // Use only first 256 chars as top-k key to avoid huge HashMap keys.
                let key = if s.len() > 256 { &s[..256] } else { s.as_str() };
                self.record_top_k(key);
            }
            Array(_) | Object(_) => {
                // Nested: count presence but don't drill in here.
                self.hll.insert("__complex__");
                self.record_top_k("__complex__");
            }
        }
    }

    fn record_top_k(&mut self, key: &str) {
        if self.top_k_saturated {
            return;
        }
        let entry = self.value_counts.entry(key.to_string()).or_insert(0);
        *entry += 1;
        if self.value_counts.len() > TOP_K_CAP {
            self.top_k_saturated = true;
            self.value_counts.clear(); // free memory
        }
    }

    fn finalise(mut self) -> FieldStats {
        // Sort reservoir and compute percentiles.
        self.length_reservoir.sort_unstable();
        let reservoir = &self.length_reservoir;
        let pct = |p: f64| -> Option<usize> {
            if reservoir.is_empty() {
                None
            } else {
                let idx = ((reservoir.len() as f64 * p).ceil() as usize).saturating_sub(1);
                Some(reservoir[idx.min(reservoir.len() - 1)])
            }
        };

        // Top-k: sort by count desc, take TOP_K_EMIT.
        let mut pairs: Vec<(String, u64)> = self.value_counts.into_iter().collect();
        pairs.sort_unstable_by_key(|b| std::cmp::Reverse(b.1));
        pairs.truncate(TOP_K_EMIT);

        FieldStats {
            name: self.name,
            total_count: self.total,
            null_count: self.nulls,
            distinct_estimate: self.hll.estimate(),
            numeric_min: self.numeric_min,
            numeric_max: self.numeric_max,
            length_p5: pct(0.05),
            length_p50: pct(0.50),
            length_p95: pct(0.95),
            top_k: pairs,
            top_k_saturated: self.top_k_saturated,
        }
    }
}

fn cheap_hash(x: u64) -> u64 {
    // Splitmix64 step — good avalanche, no deps.
    let x = x.wrapping_add(0x9e3779b97f4a7c15);
    let x = (x ^ (x >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    let x = (x ^ (x >> 27)).wrapping_mul(0x94d049bb133111eb);
    x ^ (x >> 31)
}

// ---------------------------------------------------------------------------
// Profiler — the public API
// ---------------------------------------------------------------------------

/// Streaming field profiler.
///
/// Call `record_event` for each sampled JSON object, then `finalise` once
/// to obtain per-field statistics.
pub struct Profiler {
    fields: HashMap<String, FieldAccum>,
    event_count: u64,
    /// Approximate memory budget in bytes.  Checked at each event.
    memory_budget_bytes: usize,
    over_budget: bool,
}

impl Profiler {
    /// Create a new profiler with the given memory budget (bytes).
    /// Default from RFC-024: 64 MiB.
    pub fn new(memory_budget_bytes: usize) -> Self {
        Self {
            fields: HashMap::new(),
            event_count: 0,
            memory_budget_bytes,
            over_budget: false,
        }
    }

    pub fn with_default_budget() -> Self {
        Self::new(64 * 1024 * 1024)
    }

    /// Process one JSON event object.  Non-objects are silently skipped.
    pub fn record_event(&mut self, event: &serde_json::Value) {
        let obj = match event.as_object() {
            Some(o) => o,
            None => return,
        };

        // Rough memory estimate: 4096 bytes HLL + ~32 bytes overhead per field.
        let estimated_bytes = self.fields.len() * (HLL_M + 256);
        if estimated_bytes > self.memory_budget_bytes {
            self.over_budget = true;
        }

        let seed = self.event_count;
        self.event_count += 1;

        for (key, val) in obj {
            let accum = self
                .fields
                .entry(key.clone())
                .or_insert_with(|| FieldAccum::new(key));
            if !self.over_budget {
                accum.record(val, seed);
            } else {
                // Over budget: only track null/total counts.
                accum.total += 1;
                if val.is_null() {
                    accum.nulls += 1;
                }
            }
        }
    }

    /// Consume the profiler and return finalised stats for all observed fields.
    pub fn finalise(self) -> Vec<FieldStats> {
        let mut stats: Vec<FieldStats> = self.fields.into_values().map(|a| a.finalise()).collect();
        // Stable field order for deterministic output.
        stats.sort_by(|a, b| a.name.cmp(&b.name));
        stats
    }

    pub fn event_count(&self) -> u64 {
        self.event_count
    }

    pub fn over_budget(&self) -> bool {
        self.over_budget
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn null_rate_zero_for_all_present() {
        let mut p = Profiler::with_default_budget();
        for _ in 0..100 {
            p.record_event(&json!({"x": 42}));
        }
        let stats = p.finalise();
        assert_eq!(stats[0].name, "x");
        assert_eq!(stats[0].null_rate(), 0.0);
    }

    #[test]
    fn null_rate_one_for_all_null() {
        let mut p = Profiler::with_default_budget();
        for _ in 0..50 {
            p.record_event(&json!({"x": null}));
        }
        let stats = p.finalise();
        assert_eq!(stats[0].null_rate(), 1.0);
    }

    #[test]
    fn distinct_count_bounded_by_sample_count() {
        let mut p = Profiler::with_default_budget();
        for i in 0..200u64 {
            p.record_event(&json!({"v": i.to_string()}));
        }
        let stats = p.finalise();
        // Estimate must not wildly exceed total observations.
        assert!(stats[0].distinct_estimate <= 500);
    }

    #[test]
    fn numeric_min_max_tracked() {
        let mut p = Profiler::with_default_budget();
        for v in [1.0, 5.0, 3.0, -2.0, 10.0] {
            p.record_event(&json!({"n": v}));
        }
        let stats = p.finalise();
        assert_eq!(stats[0].numeric_min, Some(-2.0));
        assert_eq!(stats[0].numeric_max, Some(10.0));
    }

    #[test]
    fn hll_low_error_on_large_set() {
        let mut hll = HyperLogLog::new();
        let n = 10_000u64;
        for i in 0..n {
            hll.insert(&i.to_string());
        }
        let est = hll.estimate();
        // Expect < 5% error (HLL++ is typically < 2%).
        let error = (est as f64 - n as f64).abs() / n as f64;
        assert!(error < 0.05, "error={error:.3} est={est} n={n}");
    }
}
