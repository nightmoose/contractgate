//! Outcome dice — decide whether a synthetic event should pass, fail, or
//! be quarantined (a sub-type of fail from the gateway's perspective).
//!
//! Both `Fail` and `Quarantine` produce events that fail validation and land
//! in `quarantine_events`.  The distinction is in the *payload shape*:
//!   - `Fail`       — a constraint violation (wrong type, out-of-range, bad enum)
//!   - `Quarantine` — a more severe violation (missing required field entirely)
//!
//! This split is cosmetic for demo purposes; the gateway treats both as failures.

use rand::{rngs::SmallRng, Rng};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Pass,
    Fail,
    Quarantine,
}

/// Roll the dice and return an `Outcome` given pass / fail / quarantine
/// probabilities.  Percentages need not sum to 1.0 — `pass_pct` and
/// `fail_pct` are sampled first; anything remaining maps to `Quarantine`.
pub fn roll(rng: &mut SmallRng, pass_pct: f64, fail_pct: f64) -> Outcome {
    let r: f64 = rng.gen();
    if r < pass_pct {
        Outcome::Pass
    } else if r < pass_pct + fail_pct {
        Outcome::Fail
    } else {
        Outcome::Quarantine
    }
}
