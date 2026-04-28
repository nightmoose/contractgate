//! ContractGate core library.
//!
//! Exposes the pure, dependency-light pieces of the validation engine so they
//! can be re-used by auxiliary binaries (demos, benchmarks, one-off tools)
//! without pulling in the Axum / sqlx server stack.
//!
//! The server binary (`src/main.rs`) re-exports these modules at the crate
//! root so existing `crate::contract::...` / `crate::validation::...` paths
//! inside submodules (`error.rs`, `ingest.rs`, `storage.rs`) continue to
//! resolve unchanged.

pub mod cli;
pub mod contract;
pub mod demo_seed;
pub mod transform;
pub mod validation;
