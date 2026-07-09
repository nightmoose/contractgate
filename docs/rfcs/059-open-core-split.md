# RFC-059 — Open-Core Split: Architecture & Licensing

**Status:** Draft — **on the shelf**
**Date:** 2026-05-27
**Branch:** n/a — planning document
**Addresses:** Founder request 2026-05-27 — keep one public repo, split community vs enterprise to protect revenue without forking.

> Constitutional doc for RFCs 060–064. Defines what is open, what is paid,
> where the license boundary lives, and the invariants every subsequent RFC
> in this series must honor.

---

## ⚠️ Build trigger — do not implement yet

This RFC is a **shelf design**. The open-core direction is sound but
premature: ContractGate has no paying enterprise customers, no inbound
asks for SSO/SAML or audit-log export, and the public launch (RFC-058
Q3 2026) hasn't shipped. Building paid features before the free product
is in the market is inventory risk.

**Implement when** at least one of the following is true:

1. A design partner has signed an MSA naming a specific enterprise
   feature they will pay for, OR
2. ≥3 unsolicited inbound asks for the same paid feature (SSO/SAML,
   audit export, fleet management, etc.), OR
3. ContractGate has ≥10 active self-hosted production deployments and
   we've validated which features they'd actually pay to add.

Until one of those fires: don't implement. Don't write the SaaS license
service. Don't refactor `confluent-connector/`. The design here is
ready to execute on when the moment comes — that's its only job for
now.

---

## Problem

ContractGate has two distribution surfaces:

1. **Rust crate** (repo root) — CLI binary, server binary, dashboard backend
   API, validation engine. All Apache 2.0 today.
2. **Java Kafka Connect SMT** (`confluent-connector/`) — single Maven module,
   Apache 2.0 today. Thin HTTP client that POSTs records to the Rust ingest
   endpoint.

We want to start charging for enterprise features (SSO/SAML, audit-log
export, dynamic contract reload, advanced DLQ routing) while keeping the
community-facing surfaces — CLI, free SMT, validation engine — fully open
to maximize adoption and stay acquirable on developer mindshare.

Forking is rejected: two repos doubles maintenance, splits issues, and
signals to community users that they're a second-class citizen.

---

## Decision

**Single public repo. Open-core split achieved through (a) Rust cargo
feature flags and (b) Maven multi-module structure inside the `connect/`
subtree.** Two LicenseManager implementations (one Rust, one Java) share a
single SaaS validation backend and signed-license format.

The proposal Alex circulated assumed a Java-only codebase; this RFC
corrects that and applies the same open-core intent to the actual mixed
Rust+Java repository.

---

## What is free, forever

These artifacts **must never** import, link to, or runtime-depend on any
license-check code. Anyone — including unauthenticated users — can build
them from `main`.

| Artifact | Language | Distribution |
|---|---|---|
| `contractgate` (CLI binary) | Rust | crates.io, Homebrew, GitHub Releases |
| `contractgate-server` (default build) | Rust | Docker Hub, GitHub Releases, source |
| `contractgate` library crate | Rust | crates.io |
| `kafka-connect-contractgate-community.jar` (SMT) | Java | Confluent Hub, Maven Central, GitHub Releases |

Validation engine, contract format, PII transforms, AI inference, the
patent core — all open. This is the bet: the moat is the patent + the
network effect of the public contract catalog (RFC-034), not the SMT
plumbing.

---

## What is paid

Bundled into two paid artifacts. Both are produced from this same repo
but only distributed to licensed customers (private artifact hosting,
covered in a later release-engineering RFC).

### `contractgate-server-enterprise` (Rust)

Built with `cargo build --features enterprise`. Adds:

- **SSO/SAML** for the dashboard backend API (alternative IdP to Supabase
  JWT — Supabase path stays for community).
- **Audit-log export** to a configurable HTTP webhook or S3 bucket.
- **Dynamic contract reload via control-plane push** (deferred to a future
  RFC; flag the hook here so 062 doesn't need to revisit).
- **LicenseManager** (Rust impl of the RFC-060 protocol).

### `kafka-connect-contractgate-enterprise.jar` (Java)

Built with `mvn package -Penterprise`. Adds:

- **Dynamic contract reload** without Connect task restart (RFC-064).
- **Per-violation DLQ routing rules** — route by severity, contract,
  field, etc. (RFC-064).
- **LicenseManager** (Java impl of the RFC-060 protocol).

Future enterprise features go into one of these two artifacts. No third
paid artifact without an RFC justifying it.

---

## Repo layout after the split

```
contractgate/                            ← Rust workspace root (unchanged)
├── Cargo.toml                           workspace + default-features = []
├── src/                                 server + lib (default build = community)
│   └── enterprise/                      ← NEW; #[cfg(feature = "enterprise")]
│       ├── mod.rs
│       ├── license_manager.rs
│       ├── saml.rs
│       └── audit_export.rs
├── src/bin/contractgate.rs              CLI — never references enterprise/
├── connect/                             ← RENAMED from confluent-connector/
│   ├── pom.xml                          aggregator + profiles
│   ├── connect-client/                  Apache 2.0 — shared HTTP client
│   ├── connect-community/               Apache 2.0 — free SMT
│   └── connect-enterprise/              BSL 1.1 — license-gated SMT features
├── dashboard/                           Next.js (unchanged)
├── docs/                                RFCs + reference docs (unchanged)
├── LICENSE                              Apache 2.0 (repo-wide default)
└── LICENSE-BSL                          ← NEW; covers connect-enterprise/ + src/enterprise/
```

**Why not a top-level Maven multi-module rooted at the repo:** the original
proposal placed `pom.xml` at the repo root with `contractgate-core` /
`contractgate-cli` as Maven modules. Those are Rust crates. Maven at the
root would either collide with `Cargo.toml` or imply a JVM rewrite of the
engine. Neither is on the table — the Rust engine *is* the patent core and
the perf claim (<15ms p99). Mixed-language repos work fine when each
ecosystem owns its own subtree.

**Why rename `confluent-connector/` → `connect/`:** the subtree will host
multiple modules and a parent POM; `connect/` matches what's inside.
Old path stays as a symlink for one minor version, then removed (handled
in RFC-063 migration steps).

---

## License boundary

| Path | License | Why |
|---|---|---|
| Repo root, `src/` (excluding `src/enterprise/`), `src/bin/`, `connect/connect-client/`, `connect/connect-community/`, `dashboard/`, CLI, SDKs | **Apache 2.0** | Maximize adoption, allow embedding, allow commercial use. Unchanged from today. |
| `src/enterprise/`, `connect/connect-enterprise/` | **BSL 1.1** (Business Source License, 3-year change-date to Apache 2.0) | Prevents AWS-style "host the enterprise features for free" hijack while giving customers full source access and an automatic open-source path on a 3-year horizon. |

BSL is the same license MariaDB, Sentry, CockroachDB, and HashiCorp
(pre-IBM) used for this exact pattern. Acquirers understand it. Customers
can read the source, modify it for their own use, and self-host with a
valid license. The only restriction is "don't sell ContractGate as a
managed service" until the change-date.

`LICENSE-BSL` file at repo root contains the BSL 1.1 text. Each
BSL-licensed file gets an SPDX header:

```
// SPDX-License-Identifier: BUSL-1.1
// Change Date: 2029-05-27
// Change License: Apache-2.0
```

The Apache-2.0 paths keep their existing SPDX headers (or get one if
missing — minor cleanup, handled in RFC-061/063).

---

## Invariants (every later RFC must obey)

1. **No license code in community paths.** A `cargo tree -p contractgate
   --no-default-features` build, and a `mvn package -Pcommunity` build,
   must each produce artifacts with zero references to LicenseManager
   classes/structs. CI enforces this with a `cargo deny` rule (forbid
   the `enterprise` module from default builds) and a `mvn dependency:tree`
   assertion (forbid `connect-enterprise` from appearing in community
   artifact graphs).

2. **One protocol, two implementations.** Rust and Java LicenseManagers
   must speak the exact same wire protocol against the SaaS backend
   (defined in RFC-060). Customers who buy both products get one license
   key that validates against either.

3. **The patent core stays open.** Validation engine, contract YAML
   format, ingest pipeline — never gated. The patent is the moat; gating
   the patented code would undermine its disclosure and is commercially
   unnecessary.

4. **The CLI stays free and license-free in perpetuity.** The CLI is the
   primary adoption funnel. Even paid customers' developers use it. No
   exceptions, no "enterprise CLI."

5. **No breaking changes to existing public APIs.** Today's REST surface,
   contract YAML format, SMT config keys, and CLI flags all keep working.
   The split is purely additive at the package/build layer.

6. **One repo, one issue tracker, one CI.** No private mirrors. Customers
   file issues in the public repo regardless of tier. Enterprise-only
   bugs get labeled but stay public.

---

## Build & artifact matrix

| Command | Produces | License surface |
|---|---|---|
| `cargo build` | `contractgate`, `contractgate-server` | Apache 2.0 only |
| `cargo build --features enterprise` | `contractgate-server-enterprise` | Apache 2.0 + BSL |
| `cargo test` | runs community tests; `--features enterprise` adds enterprise tests | — |
| `mvn package` (default profile = community) | `kafka-connect-contractgate-community-X.Y.Z.jar` | Apache 2.0 only |
| `mvn package -Penterprise` | both community + enterprise jars | Apache 2.0 + BSL |
| `make release-community` (new) | CLI + community SMT + community server | Apache 2.0 only |
| `make release-enterprise` (new) | adds enterprise server + enterprise SMT | Apache 2.0 + BSL |

CI runs both profiles on every PR. Community build is the gate that
blocks merge; enterprise build failures are fixed before release but
don't block community merges (avoids enterprise becoming a velocity
chokepoint on community PRs).

---

## RFC sequence

Implementation order (one nightly per RFC, or bundled into 2–3 nightlies
depending on Sonnet's pace):

1. **RFC-060** — LicenseManager protocol + SaaS validation endpoint.
   (Pure design + backend service. Unblocks 061 + 063.)
2. **RFC-061** — Rust `enterprise` feature flag scaffold + Rust
   LicenseManager. (Adds `src/enterprise/`, no user-visible features yet.)
3. **RFC-063** — Maven multi-module restructure of `connect/`. (Adds
   `connect-enterprise` skeleton + Java LicenseManager, no new user
   features yet. Can land in parallel with 061.)
4. **RFC-062** — Rust enterprise SSO/SAML + audit-log export. (First
   real revenue-bearing server features.)
5. **RFC-064** — Java enterprise dynamic reload + DLQ routing. (First
   real revenue-bearing SMT features.)

060 must land first because both 061 and 063 implement it. After that,
the two language tracks are independent and can ship on their own
cadence.

---

## Non-Goals

- **Self-serve enterprise checkout.** Sales-led for now; license keys
  issued manually. Self-serve is a separate billing RFC (not in this
  series).
- **Per-feature license flags beyond a coarse "enterprise" bit.** The
  RFC-060 protocol *supports* a `features: [...]` field for forward
  compatibility, but the initial license is "you have enterprise" or
  "you don't." Per-feature gating can layer on later without a protocol
  change.
- **GraalVM native CLI binaries.** The original proposal mentioned this;
  it's an optimization, not a tiering decision. Park for a separate RFC
  if/when CLI startup time becomes a complaint.
- **Confluent Hub publishing for the enterprise SMT.** Community jar
  only goes to the public Hub; enterprise jar ships from a private
  download page. Release-engineering details deferred to a future RFC.

---

## Risks & mitigations

| Risk | Mitigation |
|---|---|
| Community contributors send PRs to `src/enterprise/` or `connect-enterprise/` and we have to reject them on license grounds. | `CONTRIBUTING.md` clearly labels BSL paths and notes a CLA is required for changes there. Most contributions naturally target Apache 2.0 paths. |
| BSL is unfamiliar to some users; "is this really open source?" FUD. | `README.md` gets a clear "Free vs Paid" table linking to license texts. We're in the same boat as Sentry / MariaDB / Cockroach — well-trodden path. |
| LicenseManager bug locks out paying customers. | Phone-home failure falls back to the 90-day signed file (RFC-060). Bug in *both* paths is the failure mode to fear; integration-tested in CI against a staging SaaS endpoint. |
| Patent licensing interaction with BSL. | Apache 2.0 already grants a patent license for community paths. BSL files include the same patent grant clause; no additional patent surface is closed. Legal review before merging RFC-061. |
| Old `confluent-connector/` path breaks downstream Docker images, scripts, CI. | Symlink `confluent-connector → connect` for one minor version; release notes; deprecation warning in `connect/README.md`. |

---

## Acceptance Criteria for this RFC

This RFC is "accepted" when:

1. Alex signs off on the open-core scope (what's free, what's paid).
2. Alex signs off on the BSL choice (vs SSPL, Elastic License v2, or
   pure commercial). Recommended: **BSL 1.1** for industry familiarity.
3. RFCs 060–064 are reviewed against the invariants here and updated if
   they drift.

No code changes in this RFC. It is a sign-off doc.
