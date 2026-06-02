//! Demo seeder support modules (RFC-017).
//!
//! Used exclusively by `src/bin/demo-seeder.rs`.  Not imported by the main
//! server binary or the library crate's public API.
//!
//! Modules:
//!   - `outcome`  — pass / fail / quarantine dice roll
//!   - `synth`    — per-contract synthetic payload generators
//!   - `client`   — thin blocking HTTP client for the gateway

pub mod client;
pub mod outcome;
pub mod synth;
