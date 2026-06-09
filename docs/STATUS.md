# RFC Status Index

All RFCs in `docs/rfcs/`. Status derived from each RFC's frontmatter and
`MAINTENANCE_LOG.md`. **Shipped** = code merged to main; **Accepted** = design
signed off (may be planning docs or UI-only); **Draft** = under review;
**Superseded** = replaced by a later RFC.

> RFC-001 was used for two separate topics; see rows 001a–001c.

| RFC | Title | Status | Shipped in branch |
|---|---|---|---|
| 001a | [Batch Ingest](rfcs/001-batch-ingest.md) | Accepted | `nightly-maintenance-2026-04-17` |
| 001b | [Org-Scoped Tenancy](rfcs/001-org-scoped-tenancy.md) | Accepted | `nightly-maintenance-2026-05-03-rfc-001-finish` |
| 001c | [Org Isolation Verification Runbook](rfcs/001-org-scoped-tenancy-verification.md) | Accepted | `nightly-maintenance-2026-05-03-rfc-001-finish` |
| 002 | [Contract Versioning](rfcs/002-versioning.md) | Accepted | `nightly-maintenance-2026-04-17` |
| 003 | [Manual Replay Quarantine](rfcs/003-auto-retry.md) | Accepted | `nightly-maintenance-2026-04-18` |
| 004 | [PII Masking at Ingest](rfcs/004-pii-masking.md) | Accepted | `nightly-maintenance-2026-04-19` |
| 005 | [Python SDK](rfcs/005-python-sdk.md) | Accepted | `nightly-maintenance-2026-04-26` |
| 006 | [Multi-Format Contract Inference](rfcs/006-inference-formats.md) | Accepted | `nightly-maintenance-2026-04-27` |
| 007 | [CLI + GitOps Core](rfcs/007-cli-gitops-core.md) | Superseded by RFC-014 | — |
| 008 | [Breaking-Change Story](rfcs/008-breaking-change-story.md) | Superseded by RFC-015 | — |
| 009 | [Observability Quick Wins](rfcs/009-observability-quick-wins.md) | Superseded by RFC-016 | — |
| 010 | [Self-Hosted Basics](rfcs/010-self-hosted-basics.md) | Superseded by RFC-017 | — |
| 011 | [SDK Rollout (TS, Go, Java)](rfcs/011-sdk-rollout.md) | Superseded by RFC-018 | — |
| 012 | [Templates + Marketplace](rfcs/012-templates-marketplace.md) | Superseded by RFC-017 | — |
| 013 | [Multi-Tenant SaaS](rfcs/013-multitenant-saas.md) | Superseded (deferred) | — |
| 014 | [CLI Core + GitHub Actions Workflow](rfcs/014-cli-core.md) | Accepted | `nightly-maintenance-2026-04-27` |
| 015 | [Breaking-Change Demo Arc](rfcs/015-breaking-change-demo.md) | Draft | — |
| 016 | [Observability v1 (Metrics Only)](rfcs/016-observability-v1.md) | Shipped | `nightly-maintenance-2026-05-03-rfc-016` |
| 017 | [Onboarding Stack (Compose + Demo Seeder)](rfcs/017-onboarding-stack.md) | Accepted | `nightly-maintenance-2026-04-28` |
| 018 | [TypeScript SDK](rfcs/018-typescript-sdk.md) | Draft | — |
| 019 | [CI + Release Pipeline](rfcs/019-ci-release-pipeline.md) | Accepted | `nightly-maintenance-2026-04-27` |
| 020 | [Dashboard Polish](rfcs/020-dashboard-polish.md) | Accepted | `nightly-maintenance-2026-04-28` |
| 021 | [REST/HTTP Bulk Ingest](rfcs/021-bulk-ingest-http.md) | Accepted | `nightly-maintenance-2026-05-01` |
| 022 | [Axum 0.7 → 0.8 Upgrade](rfcs/022-axum-0.8-upgrade.md) | Shipped | `nightly-maintenance-2026-05-02-axum-upgrade` |
| 023 | [Demo Mode — Zero-Auth Local Experience](rfcs/023-demo-mode.md) | Shipped | `nightly-maintenance-2026-05-04-demo-mode` |
| 024 | [Brownfield Contract Scaffolder](rfcs/024-brownfield-scaffolder.md) | Shipped | `nightly-maintenance-2026-05-06` |
| 025 | [Kafka Ingress for Hosted ContractGate](rfcs/025-kafka-ingress.md) | Shipped | `nightly-maintenance-2026-05-07` |
| 026 | [AWS Kinesis Ingress](rfcs/026-kinesis-ingress.md) | Shipped | `nightly-maintenance-2026-05-07` |
| 027 | [Disclosed Reddit Bot](rfcs/027-reddit-bot.md) | Accepted | — |
| 028 | [Contract Queryability](rfcs/028-contract-queryability.md) | Accepted | `nightly-maintenance-2026-05-14` |
| 029 | [Egress Validation](rfcs/029-egress-validation.md) | Shipped | `nightly-maintenance-2026-05-14` |
| 030 | [Egress PII & Leakage Guard](rfcs/030-egress-pii-leakage-guard.md) | Shipped | `nightly-maintenance-2026-05-15` |
| 031 | [Provider Data-Quality Scorecard](rfcs/031-provider-data-quality-scorecard.md) | Shipped | `nightly-maintenance-2026-05-15` |
| 032 | [Contract Sharing & Publication](rfcs/032-contract-sharing-publication.md) | Shipped | `nightly-maintenance-2026-05-15` |
| 033 | [Provider-Consumer Collaboration](rfcs/033-provider-consumer-collaboration.md) | Shipped | `nightly-maintenance-2026-05-15` |
| 034 | [Public Data Source Contracts + Forking](rfcs/034-public-data-source-contracts.md) | Shipped (RFC Draft) | `nightly-maintenance-2026-05-15` |
| 035 | [CSV Contract Inference](rfcs/035-csv-contract-inference.md) | Shipped (RFC Draft) | `nightly-maintenance-2026-05-15` |
| 036 | [Source-First New Contract Flow](rfcs/036-source-first-contract-creation.md) | Draft | — |
| 037 | [API Endpoint as Contract Source (URL Inference)](rfcs/037-api-source-contract-creation.md) | Shipped (RFC Draft) | `nightly-maintenance-2026-05-15` |
| 038 | [MRI API Contracts for Findigs Integration](rfcs/038-mri-api-contracts.md) | Draft | — |
| 039 | [Supabase-JWT Dashboard Auth](rfcs/039-supabase-jwt-auth.md) | Accepted | `nightly-maintenance-2026-05-16` |
| 040 | [Fix RLS on contract_versions + quarantine_events](rfcs/040-rls-contract-versions-quarantine.md) | Accepted | `dev/p02-rls-contract-versions` |
| 041 | [API Key Hash Algorithm Docs](rfcs/041-api-key-hash-algorithm-docs.md) | Accepted | `dev/p02-rls-contract-versions` |
| 042 | [P1 Abuse-Prevention Bundle](rfcs/042-p1-abuse-prevention.md) | Accepted | `dev/p1-abuse-prevention` |
| 043 | [RFC-042 Follow-up Fixes + P0-3 Loose Ends](rfcs/043-rfc042-followup-fixes.md) | Accepted | `nightly-maintenance-2026-05-17-rfc043` |
| 044 | [Native `date` Field Type](rfcs/044-date-type.md) | Accepted | `nightly-maintenance-2026-05-17-rfc044-date-type` |
| 045 | [Plan-Based Feature Gating](rfcs/045-plan-gating.md) | Accepted | `nightly-maintenance-2026-05-17-rfc045-plan-gating` |
| 046 | [API Workbench](rfcs/046-api-workbench.md) | Draft | `nightly-maintenance-2026-05-18-rfc046-api-workbench` |
| 047 | [Backend Org Scoping (IDOR fix)](rfcs/047-backend-org-scoping.md) | Accepted | `nightly-maintenance-2026-05-22-rfc047-048` |
| 048 | [Remove Trusted x-org-id Header](rfcs/048-drop-x-org-id-header-trust.md) | Accepted | `nightly-maintenance-2026-05-22-rfc047-048` |
| 049 | [SSRF Redirect Hardening](rfcs/049-ssrf-redirect-hardening.md) | Accepted | `nightly-maintenance-2026-05-22-rfc049-050` |
| 050 | [CORS Origin Allowlist](rfcs/050-cors-origin-allowlist.md) | Accepted | `nightly-maintenance-2026-05-22-rfc049-050` |
| 051 | [API-Key Cache Hardening](rfcs/051-api-key-cache-hardening.md) | Accepted | `nightly-maintenance-2026-05-23-rfc051-054` |
| 052 | [Periodic Supabase JWKS Refresh](rfcs/052-jwks-refresh.md) | Accepted | `nightly-maintenance-2026-05-24-rfc052-053` |
| 053 | [Real `/health` with DB Probe](rfcs/053-health-check-db-probe.md) | Accepted | `nightly-maintenance-2026-05-24-rfc052-053` |
| 054 | [Lock-Poison Recovery](rfcs/054-lock-poison-recovery.md) | Accepted | `nightly-maintenance-2026-05-23-rfc051-054` |
| 055 | [Fix CI sqlx-cli Version Drift](rfcs/055-ci-sqlx-toolchain-fix.md) | Accepted | `nightly-maintenance-2026-05-23-rfc055` |
| 056 | [Server-Side API Key Issuance](rfcs/056-server-side-api-key-issuance.md) | Accepted | `nightly-maintenance-2026-05-24-rfc056` |
| 057 | [Documentation Completeness for Public Launch](rfcs/057-launch-documentation-completeness.md) | Accepted | `nightly-maintenance-2026-05-24-rfc057` |
| 058 | [12-Month Product Roadmap (2026 H2 – 2027 H1)](rfcs/058-twelve-month-roadmap.md) | Draft | — |
| 059 | [Open-Core Split: Architecture & Licensing](rfcs/059-open-core-split.md) | Draft (shelf) | — |
| 060 | [LicenseManager Protocol + SaaS Validation](rfcs/060-license-manager-protocol.md) | Draft (shelf) | — |
| 061 | [Rust `enterprise` Cargo Feature Flag](rfcs/061-rust-enterprise-feature-flag.md) | Deferred | — |
| 062 | [Rust Enterprise: SSO/SAML + Audit Export](rfcs/062-rust-enterprise-sso-saml-audit-export.md) | Deferred | — |
| 063 | [Maven Multi-Module Restructure of `connect/`](rfcs/063-maven-multimodule-connect.md) | Deferred | — |
| 064 | [Kafka Connect SMT: Dynamic Reload + DLQ Routing](rfcs/064-java-enterprise-smt-features.md) | Accepted | `nightly-maintenance-2026-05-27-rfc064-smt-reload-dlq` |
| 065 | [Ingest/Egress Contract-Scope Enforcement](rfcs/065-ingest-egress-contract-scope.md) | Accepted | `nightly-maintenance-2026-05-28-rfc065-ingest-egress-scope` |
| 066 | [Remove Legacy env-var `API_KEY` Master Key](rfcs/066-remove-legacy-api-key.md) | Implemented | `nightly-maintenance-2026-05-28-rfc065-ingest-egress-scope` |
| 067 | [Request-Path Panic Hardening](rfcs/067-request-path-panic-hardening.md) | Implemented | `nightly-maintenance-2026-05-28-rfc067-panic-hardening` |
| 068 | [Run Org-Isolation DB Tests in CI](rfcs/068-org-isolation-tests-in-ci.md) | Implemented | `nightly-maintenance-2026-05-28-rfc068-isolation-ci` |
| 069 | [Unit Coverage for Untested Pure Functions](rfcs/069-pure-fn-unit-coverage.md) | Implemented | `nightly-maintenance-2026-05-28-rfc069-pure-fn-coverage` |
| 070 | [Org-Scope Tests for Version-Mutating Storage Fns](rfcs/070-version-mutation-org-scope-tests.md) | Implemented | `nightly-maintenance-2026-05-28-rfc069-pure-fn-coverage` |
| 071 | [Coverage Ratchet Gate](rfcs/071-coverage-ratchet-gate.md) | Implemented | `nightly-maintenance-2026-05-28-rfc069-pure-fn-coverage` |
| 072 | [Quarantine→Replay Race-Guard Test](rfcs/072-quarantine-replay-race-guard-test.md) | Implemented | `nightly-maintenance-2026-05-28-rfc069-pure-fn-coverage` |
| 073 | [Org-Isolation Test in Compose-Smoke](rfcs/073-org-isolation-test-in-compose-smoke.md) | Implemented | `nightly-maintenance-2026-05-28-rfc069-pure-fn-coverage` |
| 074 | [Org-Ownership Enforcement on Data Plane (latent P0-class)](rfcs/074-org-ownership-enforcement-data-plane.md) | Implemented | `nightly-maintenance-2026-05-28-rfc069-pure-fn-coverage` |
| 075 | [Auth-On Isolation Test Lane](rfcs/075-auth-on-isolation-test-lane.md) | Draft | TBD |
| 076 | [`cg test`: Local Contract Dry-Run](rfcs/076-contract-test-dry-run.md) | Implemented | `nightly-maintenance-2026-06-01-rfc076-contract-test-dry-run` |
| 077 | [RAG-Ingestion Contract Profile](rfcs/077-rag-ingestion-contract-profile.md) | Draft | TBD |
| 078 | [Cross-Surface Pipeline Walkthrough Template](rfcs/078-pipeline-walkthrough-template.md) | Draft | TBD |
| 079 | [Unify Inference on the Rust Engine](rfcs/079-unify-inference-on-rust-engine.md) | Draft | TBD |

---

## Status key

| Status | Meaning |
|---|---|
| **Shipped** | Code merged to main; feature is live. RFC frontmatter says "Implemented". |
| **Accepted** | Design signed off, code shipped or in progress. RFC frontmatter says "Accepted". |
| **Shipped (RFC Draft)** | Code shipped and in production, but the RFC document was never formally accepted (frontmatter still says "Draft"). To be rectified in a follow-up. |
| **Draft** | Under review; no code shipped. |
| **Draft (shelf)** | Design is finished and on the shelf, but implementation is intentionally not scheduled — see the RFC's "Build trigger" section for what fires it. |
| **Deferred** | Design exists but implementation is intentionally postponed (typically waiting on an upstream "shelf" RFC to fire, or on validated customer demand). |
| **Superseded** | Replaced by a later RFC listed in the notes column; kept for design context. |

---

*Last updated: 2026-06-01 — RFC-076 adds `cg test --contract <FILE> --data <FILE>`: a zero-dependency local dry-run CLI over the existing `validate()` engine; accepts NDJSON, JSON array, or single object; exit 0/1/2; `--format json`, `--fail-fast`, `--quiet`; integration tests in `tests/cli_test_dryrun.rs`; reference doc at `docs/cg-test-reference.md`.*

*2026-05-28 — RFC-065 closes the ingest/egress cross-tenant authz gap (per-key `allowed_contract_ids` now enforced on all hot paths); RFC-066 removes the legacy env-var `API_KEY` master key (dev no-auth now gated on explicit `CONTRACTGATE_DEV_NO_AUTH=1`); RFC-067 converts six latent request-path panics (collaboration `expect`, replay `unwrap`) to clean 401 / graceful skip; RFC-068 wires the self-contained org-isolation DB tests into CI (`migrations-check`) so cross-tenant scoping is enforced on every PR; RFC-069 adds unit coverage for the previously-untested pure auth helper `jwks_url_from_database_url` (both Supabase connection-string formats + `None` fallbacks), `PublicationRow::is_revoked`, and `JwtAuthError` Display; RFC-070 extends the org-isolation DB test to the write-side version-mutation BOLA surface (`patch_version_yaml`/`deprecate_version`/`delete_version` wrong-org → `VersionNotFound`); RFC-071 adds a `cargo-llvm-cov` ratchet coverage gate to CI (fails only on a drop beyond tolerance vs a committed baseline; non-blocking for deploy while it beds in); RFC-072 adds a self-seeding DB test for the quarantine→replay race guard (`mark_quarantine_replayed_batch` stamps a source row at most once; the loser links to neither audit id), wired into the `migrations-check` named-test list; RFC-073 wired the cross-tenant isolation integration test (`cross_org_ingest_is_rejected`) into the compose-smoke lane against a fixed-UUID two-org seed — but that wiring proved to be a false signal: the compose stack runs `CONTRACTGATE_DEV_NO_AUTH=1`, so the gateway never validates keys and the test returns 200 with or without org scoping (the smoke step is now disabled pending a fix); RFC-074 fixes a latent P0-class cross-tenant write found by code inspection (the ingest/egress/v1-ingest handlers resolved the caller's `org_id` from the key but passed `None` to the storage lookup, so under production auth any unrestricted key could write to another org's contract) — the fix threads `org_id` through and is verified by code inspection + `cargo test`, not by the smoke lane; RFC-075 (Draft) specifies the auth-on (`DEV_NO_AUTH=0`) test lane needed to actually prove isolation end-to-end, with a no-key→401 sanity gate so the lane can't silently mislead again.*
