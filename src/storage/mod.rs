//! Supabase (PostgreSQL) storage layer for ContractGate.
//!
//! All database access goes through this module.  Uses `sqlx` with **runtime**
//! (non-macro) query execution so the crate builds without requiring a live
//! `DATABASE_URL` at compile time.  To enable compile-time query verification,
//! run `cargo sqlx prepare` against a real database and commit the `.sqlx/`
//! directory, then switch to `query!` / `query_as!` macros.
//!
//! Split into submodules by domain (2026-07-10, RFC/worklist item 3 — was a
//! single 2,803-line file). This is a pure move: every function/struct kept
//! its exact signature, and `pub use` below re-exports everything at
//! `crate::storage::*` so no call site elsewhere in the crate changed.
//! Grouping deviates from the original worklist proposal where the code
//! didn't support it — there are no api_keys or kafka/kinesis functions in
//! this file (they live in their own top-level modules), so `keys.rs` /
//! `ingress.rs` were dropped. `contracts.rs` (identity+version) and
//! `audit.rs` (writes+reporting) are each a bit over the ~600-line guideline
//! because splitting them further would have meant exposing private row
//! types across more file boundaries for no real benefit — see the
//! module-doc comment in each file for the specific reasoning.

mod audit;
mod collaboration;
mod contracts;
mod publication;
mod replay;

pub use audit::*;
pub use collaboration::*;
pub use contracts::*;
pub use publication::*;
pub use replay::*;
