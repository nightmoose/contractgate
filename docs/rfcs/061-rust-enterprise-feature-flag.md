# RFC-061 — Rust `enterprise` Cargo Feature Flag

**Status:** Deferred
**Date:** 2026-05-27
**Branch:** n/a — design only
**Addresses:** [RFC-059](059-open-core-split.md), [RFC-060](060-license-manager-protocol.md)
**Depends on:** RFC-060 (SaaS endpoint + protocol)

> Adds the `enterprise` cargo feature, the `src/enterprise/` module
> (gated by `#[cfg(feature = "enterprise")]`), the Rust
> `LicenseManager`, and the build/CI plumbing to produce two server
> binaries. No user-facing features yet — those land in RFC-062.

---

## ⚠️ Deferred — pure plumbing, no improvement until 062 fires

This RFC is plumbing for [RFC-062](062-rust-enterprise-sso-saml-audit-export.md).
Implementing it standalone adds `#[cfg]` complexity to `main.rs`, a CI
symbol-leak check to maintain, and a second binary to release — all
with nothing to gate.

**Implement when** RFC-062 is being implemented (same nightly).
Both should land together so the feature flag has something to flag.

See [RFC-059 build trigger](059-open-core-split.md#️-build-trigger--do-not-implement-yet)
for the upstream condition.

---

## Decision

- Single feature flag: `enterprise`. Coarse, not per-feature. RFC-059
  invariant.
- Default Cargo build = community. `cargo build` produces
  `contractgate-server` with zero enterprise code linked in.
- `cargo build --features enterprise` produces
  `contractgate-server-enterprise` (renamed via `[[bin]]` cfg).
- All enterprise code lives under `src/enterprise/`. Nothing else in
  `src/` imports from it except via `#[cfg(feature = "enterprise")]`
  call sites in `src/main.rs` and the auth/router modules.
- CLI binary (`src/bin/contractgate.rs`) is built without the
  `enterprise` feature even when the workspace is built with it. Two
  ways to enforce this — see "CLI isolation" below.

---

## Cargo.toml changes

```toml
[package]
name = "contractgate"
version = "0.1.0"
edition = "2021"

[features]
default = []
enterprise = ["dep:saml2", "dep:aws-sdk-s3"]    # plus any new deps
demo = ["dep:rdkafka", ...]                     # existing

[dependencies]
# existing deps unchanged

# Enterprise-only deps marked optional so they're not pulled in by default.
saml2 = { version = "0.5", optional = true }
aws-sdk-s3 = { version = "1.0", optional = true, default-features = false, features = ["rt-tokio"] }
ed25519-dalek = { version = "2.1", optional = true }   # for license offline-token verify
reqwest = { version = "0.12", optional = true, default-features = false, features = ["json", "rustls-tls"] }

[[bin]]
name = "contractgate-server"
path = "src/main.rs"
required-features = []    # always available

[[bin]]
name = "contractgate-server-enterprise"
path = "src/main.rs"      # same entrypoint, different name
required-features = ["enterprise"]

[[bin]]
name = "contractgate"
path = "src/bin/contractgate.rs"
required-features = []    # CLI never depends on enterprise
```

The second `[[bin]]` block reusing `src/main.rs` works because
`required-features` is the only difference. Cargo will refuse to build
`contractgate-server-enterprise` without `--features enterprise`, which
is exactly what we want.

(Same as today, just adding the second name and feature gates.)

---

## Module layout

```
src/
├── main.rs                          server entrypoint; #[cfg] enterprise wiring
├── lib.rs                           unchanged
├── jwt_auth.rs                      community auth (Supabase JWT) — unchanged
├── api_key.rs                       community auth (API key) — unchanged
├── enterprise/                      NEW; entire dir gated by feature
│   ├── mod.rs                       pub use re-exports; module-level cfg gate
│   ├── license_manager.rs           RFC-060 protocol client
│   ├── license_cache.rs             on-disk offline-token persistence
│   ├── saml.rs                      stub for RFC-062
│   ├── audit_export.rs              stub for RFC-062
│   └── README.md                    "this dir is BSL-licensed" marker
```

`src/enterprise/mod.rs`:

```rust
// SPDX-License-Identifier: BUSL-1.1
// Change Date: 2029-05-27
// Change License: Apache-2.0

//! Enterprise-only modules. Compiled only when `--features enterprise`.
//! See docs/rfcs/059-open-core-split.md and 060-license-manager-protocol.md.

pub mod license_cache;
pub mod license_manager;
pub mod saml;
pub mod audit_export;

pub use license_manager::{LicenseManager, LicenseState, LicenseError};
```

The whole module is referenced from `src/lib.rs` as:

```rust
#[cfg(feature = "enterprise")]
pub mod enterprise;
```

That single line is the only mention of `enterprise` in `src/lib.rs`.
Everywhere else that needs enterprise code uses
`#[cfg(feature = "enterprise")]` at the call site, never importing
`enterprise::` from non-gated code.

---

## CLI isolation

The CLI binary at `src/bin/contractgate.rs` must never link in
enterprise code, even when someone runs `cargo build --features
enterprise`. Two layers:

1. **`required-features = []` on the CLI `[[bin]]` block.** This alone
   isn't enough — `--features enterprise` will still enable the feature
   for the package, which means `src/enterprise/` *compiles* and gets
   pulled into the CLI's dependency graph if the CLI references the
   library crate (which it does — `use contractgate::...`).

2. **Conditional compilation at the library boundary.** The `pub mod
   enterprise;` declaration is gated. Any code that calls into
   `enterprise::` is gated. The CLI imports things like
   `contractgate::contract::*` which never transitively reach
   `enterprise::`. Result: even when the feature is enabled, the CLI
   binary's symbol table has no enterprise functions.

3. **CI verification (new):** add a job that runs:
   ```
   cargo build --features enterprise --bin contractgate
   nm target/debug/contractgate | grep -i license_manager && exit 1 || exit 0
   nm target/debug/contractgate | grep -i saml          && exit 1 || exit 0
   ```
   Fails the build if enterprise symbols leak into the CLI. RFC-059
   invariant enforcement.

---

## main.rs wiring (illustrative)

```rust
// src/main.rs
use axum::Router;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // ... existing community setup unchanged ...

    let app = Router::new()
        .merge(community_routes())
        .with_state(state);

    #[cfg(feature = "enterprise")]
    let app = {
        let lic = contractgate::enterprise::LicenseManager::from_env().await?;
        app.merge(contractgate::enterprise::saml::routes(&lic))
           .merge(contractgate::enterprise::audit_export::routes(&lic))
           .layer(contractgate::enterprise::license_middleware(lic))
    };

    axum::serve(listener, app).await?;
    Ok(())
}
```

Three `#[cfg]` call sites total in `main.rs`. That's the entire
enterprise surface area at the wiring layer.

---

## LicenseManager: minimum viable impl (this RFC)

`src/enterprise/license_manager.rs` implements:

```rust
pub struct LicenseManager {
    state: tokio::sync::RwLock<LicenseState>,
    cache_path: PathBuf,
    validation_url: Url,
    license_key: String,
    install_id: Uuid,
}

pub enum LicenseState {
    Valid { license_id: String, expires_at: DateTime<Utc>, features: HashSet<String> },
    Grace { features: HashSet<String>, grace_until: DateTime<Utc> },
    Invalid { reason: String },
    Unconfigured,
}

pub enum LicenseError {
    NoKey,
    InvalidKeyFormat,
    OfflineTokenExpired,
    NetworkFailure(reqwest::Error),
    Crypto(ed25519_dalek::ed25519::Error),
}

impl LicenseManager {
    /// Read env/config, load cached offline token if any, schedule
    /// background phone-home. Returns immediately — does not block on
    /// network.
    pub async fn from_env() -> Result<Arc<Self>, LicenseError> { ... }

    /// Returns the current state. Cheap, lock-read only.
    pub fn state(&self) -> LicenseState { ... }

    /// Returns true if the named feature is currently licensed.
    pub fn has(&self, feature: &str) -> bool { ... }

    /// Force a phone-home now. Used by admin REST endpoint and tests.
    pub async fn refresh(&self) -> Result<(), LicenseError> { ... }
}

/// Axum middleware: rejects requests to enterprise routes when the
/// license is invalid. Community routes are unaffected (this middleware
/// is only applied to the enterprise sub-router).
pub fn license_middleware(lic: Arc<LicenseManager>) -> impl Layer<...> { ... }
```

24h refresh loop runs in a tokio task spawned by `from_env`. Cancellation
is on Drop; in practice the manager lives for the process lifetime.

Offline-token verification uses `ed25519-dalek` against a public key
embedded via `include_bytes!("../../crypto/license-signing-key-2026.pub")`.

---

## Admin REST surface (enterprise builds only)

```
GET  /admin/license              → JSON view of current LicenseState
POST /admin/license/refresh      → force phone-home
```

Both endpoints require existing admin-tier auth (Supabase JWT with an
admin claim). Routes registered only under `#[cfg(feature =
"enterprise")]`. Useful for ops debugging and the support handoff
script.

Not exposed in community builds — the routes don't exist.

---

## Tests

- Unit: offline token verify happy path + expired + wrong signature +
  wrong kid.
- Unit: cache file round-trip (persist + reload + corrupt-file handling).
- Integration: `LicenseManager::from_env` against a mocked HTTP server
  returning each documented response shape (valid / grace / revoked /
  500 / timeout).
- Integration: middleware rejects enterprise requests when state is
  Invalid; accepts when Valid.
- CI: the `nm`-based symbol-leak check from "CLI isolation" above.

Tests live under `src/enterprise/` modules with `#[cfg(test)]`, so they
only build when `--features enterprise --test` is set.

---

## Build artifacts after this RFC

| Command | Output |
|---|---|
| `cargo build` | `contractgate`, `contractgate-server` (community only, no license code) |
| `cargo build --features enterprise` | adds `contractgate-server-enterprise` (community routes + LicenseManager + empty SAML/audit stubs from RFC-062) |
| `cargo build --features demo` | `demo` binary, unchanged |
| `cargo build --features enterprise,demo` | all of the above |

The `contractgate-server-enterprise` binary will boot and serve community
traffic immediately. If no license is configured, it logs a WARN and
behaves like the community server. This makes it safe to release as the
"single binary" for self-hosted enterprise customers who haven't gotten
their key yet.

---

## Out of scope (covered in RFC-062)

- The SAML SP implementation (`src/enterprise/saml.rs` is a stub).
- The audit-export pipeline (`src/enterprise/audit_export.rs` is a stub).
- New DB tables for IdP configs.
- Dashboard UI for SSO setup.

---

## Acceptance Criteria

1. `cargo build` produces `contractgate-server` binary; `nm` shows no
   license/SAML/audit symbols.
2. `cargo build --features enterprise` produces both
   `contractgate-server` and `contractgate-server-enterprise`. Both work
   against community traffic without a license.
3. With `CONTRACTGATE_LICENSE_KEY` set to a valid staging key, the
   enterprise binary logs `License validated for ...` within 5s of
   startup.
4. With an invalid/missing key, the enterprise binary logs the documented
   ERROR line and disables nothing (because no enterprise features exist
   yet — they land in RFC-062).
5. `cargo test --features enterprise` passes including the new
   integration tests.
6. CI job for the CLI symbol-leak check is wired and green.
7. `cargo check` and `cargo test` (no features) remain unaffected.

**Cannot test locally:** Alex needs to run `cargo build`, `cargo build
--features enterprise`, `cargo test`, and `cargo test --features
enterprise`. Per project rules, I won't run cargo.
