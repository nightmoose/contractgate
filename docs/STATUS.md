# RFC Status Index

All RFCs in `docs/rfcs/`. Status derived from each RFC's frontmatter. **Shipped** = code merged to main; **Accepted** = design
signed off (may be planning docs or UI-only); **Draft** = under review;
**Superseded** = replaced by a later RFC.

Internal tooling (the admin app and its RFC-088), maintenance logs and review
worklists are kept outside this public repository; RFCs that cite them refer to
them by name only.

> RFC-001 was used for two separate topics; see rows 001a–001c.

| RFC | Title | Status |
|---|---|---|
| 001a | [Batch Ingest](rfcs/001-batch-ingest.md) | Accepted |
| 001b | [Org-Scoped Tenancy](rfcs/001-org-scoped-tenancy.md) | Accepted |
| 001c | [Org Isolation Verification Runbook](rfcs/001-org-scoped-tenancy-verification.md) | Accepted |
| 002 | [Contract Versioning](rfcs/002-versioning.md) | Accepted |
| 003 | [Manual Replay Quarantine](rfcs/003-auto-retry.md) | Accepted |
| 004 | [PII Masking at Ingest](rfcs/004-pii-masking.md) | Accepted |
| 005 | [Python SDK](rfcs/005-python-sdk.md) | Accepted |
| 006 | [Multi-Format Contract Inference](rfcs/006-inference-formats.md) | Accepted |
| 007 | [CLI + GitOps Core](rfcs/007-cli-gitops-core.md) | Superseded by RFC-014 |
| 008 | [Breaking-Change Story](rfcs/008-breaking-change-story.md) | Superseded by RFC-015 |
| 009 | [Observability Quick Wins](rfcs/009-observability-quick-wins.md) | Superseded by RFC-016 |
| 010 | [Self-Hosted Basics](rfcs/010-self-hosted-basics.md) | Superseded by RFC-017 |
| 011 | [SDK Rollout (TS, Go, Java)](rfcs/011-sdk-rollout.md) | Superseded by RFC-018 |
| 012 | [Templates + Marketplace](rfcs/012-templates-marketplace.md) | Superseded by RFC-017 |
| 013 | [Multi-Tenant SaaS](rfcs/013-multitenant-saas.md) | Superseded (deferred) |
| 014 | [CLI Core + GitHub Actions Workflow](rfcs/014-cli-core.md) | Accepted |
| 015 | [Breaking-Change Demo Arc](rfcs/015-breaking-change-demo.md) | Draft |
| 016 | [Observability v1 (Metrics Only)](rfcs/016-observability-v1.md) | Shipped |
| 017 | [Onboarding Stack (Compose + Demo Seeder)](rfcs/017-onboarding-stack.md) | Accepted |
| 018 | [TypeScript SDK](rfcs/018-typescript-sdk.md) | Shipped (v0.2.0) |
| 019 | [CI + Release Pipeline](rfcs/019-ci-release-pipeline.md) | Accepted |
| 020 | [Dashboard Polish](rfcs/020-dashboard-polish.md) | Accepted |
| 021 | [REST/HTTP Bulk Ingest](rfcs/021-bulk-ingest-http.md) | Accepted |
| 022 | [Axum 0.7 → 0.8 Upgrade](rfcs/022-axum-0.8-upgrade.md) | Shipped |
| 023 | [Demo Mode — Zero-Auth Local Experience](rfcs/023-demo-mode.md) | Shipped |
| 024 | [Brownfield Contract Scaffolder](rfcs/024-brownfield-scaffolder.md) | Shipped |
| 025 | [Kafka Ingress for Hosted DataContractGate](rfcs/025-kafka-ingress.md) | Shipped |
| 026 | [AWS Kinesis Ingress](rfcs/026-kinesis-ingress.md) | Shipped |
| 027 | [Disclosed Reddit Bot](rfcs/027-reddit-bot.md) | Accepted |
| 028 | [Contract Queryability](rfcs/028-contract-queryability.md) | Accepted |
| 029 | [Egress Validation](rfcs/029-egress-validation.md) | Shipped |
| 030 | [Egress PII & Leakage Guard](rfcs/030-egress-pii-leakage-guard.md) | Shipped |
| 031 | [Provider Data-Quality Scorecard](rfcs/031-provider-data-quality-scorecard.md) | Shipped |
| 032 | [Contract Sharing & Publication](rfcs/032-contract-sharing-publication.md) | Shipped |
| 033 | [Provider-Consumer Collaboration](rfcs/033-provider-consumer-collaboration.md) | Shipped |
| 034 | [Public Data Source Contracts + Forking](rfcs/034-public-data-source-contracts.md) | Shipped |
| 035 | [CSV Contract Inference](rfcs/035-csv-contract-inference.md) | Shipped |
| 036 | [Source-First New Contract Flow](rfcs/036-source-first-contract-creation.md) | Draft |
| 037 | [API Endpoint as Contract Source (URL Inference)](rfcs/037-api-source-contract-creation.md) | Shipped |
| 038 | MRI API Contracts / envelope stanza (document removed) | Draft |
| 039 | [Supabase-JWT Dashboard Auth](rfcs/039-supabase-jwt-auth.md) | Accepted |
| 040 | [Fix RLS on contract_versions + quarantine_events](rfcs/040-rls-contract-versions-quarantine.md) | Accepted |
| 041 | [API Key Hash Algorithm Docs](rfcs/041-api-key-hash-algorithm-docs.md) | Accepted |
| 042 | [P1 Abuse-Prevention Bundle](rfcs/042-p1-abuse-prevention.md) | Accepted |
| 043 | [RFC-042 Follow-up Fixes + P0-3 Loose Ends](rfcs/043-rfc042-followup-fixes.md) | Accepted |
| 044 | [Native `date` Field Type](rfcs/044-date-type.md) | Accepted |
| 045 | [Plan-Based Feature Gating](rfcs/045-plan-gating.md) | Accepted |
| 046 | [API Workbench](rfcs/046-api-workbench.md) | Shipped |
| 047 | [Backend Org Scoping (IDOR fix)](rfcs/047-backend-org-scoping.md) | Accepted |
| 048 | [Remove Trusted x-org-id Header](rfcs/048-drop-x-org-id-header-trust.md) | Accepted |
| 049 | [SSRF Redirect Hardening](rfcs/049-ssrf-redirect-hardening.md) | Accepted |
| 050 | [CORS Origin Allowlist](rfcs/050-cors-origin-allowlist.md) | Accepted |
| 051 | [API-Key Cache Hardening](rfcs/051-api-key-cache-hardening.md) | Accepted |
| 052 | [Periodic Supabase JWKS Refresh](rfcs/052-jwks-refresh.md) | Accepted |
| 053 | [Real `/health` with DB Probe](rfcs/053-health-check-db-probe.md) | Accepted |
| 054 | [Lock-Poison Recovery](rfcs/054-lock-poison-recovery.md) | Accepted |
| 055 | [Fix CI sqlx-cli Version Drift](rfcs/055-ci-sqlx-toolchain-fix.md) | Accepted |
| 056 | [Server-Side API Key Issuance](rfcs/056-server-side-api-key-issuance.md) | Accepted |
| 057 | [Documentation Completeness for Public Launch](rfcs/057-launch-documentation-completeness.md) | Accepted |
| 058 | [12-Month Product Roadmap (2026 H2 – 2027 H1)](rfcs/058-twelve-month-roadmap.md) | Draft |
| 059 | [Open-Core Split: Architecture & Licensing](rfcs/059-open-core-split.md) | Draft (shelf) |
| 060 | [LicenseManager Protocol + SaaS Validation](rfcs/060-license-manager-protocol.md) | Draft (shelf) |
| 061 | [Rust `enterprise` Cargo Feature Flag](rfcs/061-rust-enterprise-feature-flag.md) | Deferred |
| 062 | [Rust Enterprise: SSO/SAML + Audit Export](rfcs/062-rust-enterprise-sso-saml-audit-export.md) | Deferred |
| 063 | [Maven Multi-Module Restructure of `connect/`](rfcs/063-maven-multimodule-connect.md) | Deferred |
| 064 | [Kafka Connect SMT: Dynamic Reload + DLQ Routing](rfcs/064-java-enterprise-smt-features.md) | Accepted |
| 065 | [Ingest/Egress Contract-Scope Enforcement](rfcs/065-ingest-egress-contract-scope.md) | Accepted |
| 066 | [Remove Legacy env-var `API_KEY` Master Key](rfcs/066-remove-legacy-api-key.md) | Implemented |
| 067 | [Request-Path Panic Hardening](rfcs/067-request-path-panic-hardening.md) | Implemented |
| 068 | [Run Org-Isolation DB Tests in CI](rfcs/068-org-isolation-tests-in-ci.md) | Implemented |
| 069 | [Unit Coverage for Untested Pure Functions](rfcs/069-pure-fn-unit-coverage.md) | Implemented |
| 070 | [Org-Scope Tests for Version-Mutating Storage Fns](rfcs/070-version-mutation-org-scope-tests.md) | Implemented |
| 071 | [Coverage Ratchet Gate](rfcs/071-coverage-ratchet-gate.md) | Implemented |
| 072 | [Quarantine→Replay Race-Guard Test](rfcs/072-quarantine-replay-race-guard-test.md) | Implemented |
| 073 | [Org-Isolation Test in Compose-Smoke](rfcs/073-org-isolation-test-in-compose-smoke.md) | Implemented |
| 074 | [Org-Ownership Enforcement on Data Plane (latent P0-class)](rfcs/074-org-ownership-enforcement-data-plane.md) | Implemented |
| 075 | [Auth-On Isolation Test Lane](rfcs/075-auth-on-isolation-test-lane.md) | Shipped |
| 076 | [`cg test`: Local Contract Dry-Run](rfcs/076-contract-test-dry-run.md) | Implemented |
| 077 | [RAG-Ingestion Contract Profile](rfcs/077-rag-ingestion-contract-profile.md) | Accepted (examples shipped, engine-validated; GA promotion deferred) |
| 078 | [Cross-Surface Pipeline Walkthrough Template](rfcs/078-pipeline-walkthrough-template.md) | Shipped |
| 079 | [Unify Inference on the Rust Engine](rfcs/079-unify-inference-on-rust-engine.md) | Shipped |
| 080 | [Visual Builder Nested-Object Support](rfcs/080-visual-builder-object-support.md) | Shipped |
| 081 | [Quarantine List + Replay Reconciliation](rfcs/081-quarantine-list-and-replay-reconciliation.md) | Shipped |
| 082 | [Exportable Pilot Report](rfcs/082-exportable-pilot-report.md) | Shipped |
| 083 | [Event Metering + Usage API](rfcs/083-event-metering-and-usage.md) | Shipped (Phases 1–3) |
| 084 | [Slack Lead-Intake Bot](rfcs/084-slack-lead-bot.md) | Shipped |
| 085 | [Org Admin / Team Management](rfcs/085-org-admin-team-management.md) | Shipped |
| 089 | [LLM-Pasteable Onboarding (`/llms.txt` + agent playbook)](rfcs/089-llm-agent-onboarding.md) | Shipped |
| 090 | [Official MCP Server (stdio)](rfcs/090-mcp-server.md) | Shipped (v0.2.0) |
| 091 | [Help Catalog + “What’s this?” Inspect Mode](rfcs/091-whats-this-help-mode.md) | Accepted |
| 092 | [Ralph-native demo stack (Kafi / Driftless / DataContractGate)](rfcs/092-ralph-native-demo.md) | Accepted — partner-ready |
| 093 | [Egress as a first-class door](rfcs/093-egress-first-class.md) | Shipped |
| 105 | [ODCS v3.2 property enums, dual-brand MCP metadata, AISchemaGate guardrails](rfcs/105-odcs-v32-and-guardrail-recipes.md) | Shipped |
| 106 | [Streaming chunk validation](rfcs/106-streaming-chunk-validation.md) | Draft |
| 107 | [Python framework adapters](rfcs/107-python-framework-adapters.md) | Shipped |
| 108 | [MIT-only dependency policy](rfcs/108-mit-only-dependencies.md) | Accepted |
| 109 | [Drop mode for failing records](rfcs/109-drop-mode.md) | Accepted |
| 110 | [Quarantine by plan](rfcs/110-quarantine-plan-split.md) | Accepted |

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

*2026-10-30 — RFC-110: Free quarantine caps (50,000 per organization and 10,000 per contract, shared by DataContractGate and AISchemaGate) live in `src/plan.rs`. A full cap counts new failures under Drops. Replay and bulk replay return 403 `plan_required`. Migration 046 redacts leftover Free bodies and keeps the rows. A downgrade waits 30 days. See `docs/quarantine-plan-reference.md`.*

*2026-10-30 — RFC-109: failing records can be counted and not stored (`on_fail: drop`). Quarantine stays the default for DataContractGate and AISchemaGate. The safety cap is watch-only until enforcement is enabled. Drop counts are minute buckets kept 30 days. See `docs/drop-mode-reference.md`.*

*2026-10-30 — RFC-108: dependency licenses allow MIT, ISC, BSD-2-Clause, BSD-3-Clause, and 0BSD. The MCP server uses `tmcp` (MIT) with `@tmcp/transport-stdio` and `@tmcp/adapter-zod`. `cargo deny` uses that allow list, and a `MIT OR …` crate still passes because MIT can be selected. npm and Python use the same licenses. Python names them with the pip-licenses classifier strings. Everything else is listed in `docs/LICENSE-EXCEPTIONS.md`. A pull request that changes a manifest or lockfile runs the check, and so does a weekly schedule.*

*2026-10-28 — RFC-107: PyPI `aischemagate` 0.5.0 adds optional LangChain and LlamaIndex output parsers and an Instructor hook. Each calls the existing egress helper (`POST /egress/{contract_id}`, contract names via `GET /contracts`). The Instructor hook runs that structural check before Pydantic field validation. Streaming chunk validation stays RFC-106 (Draft) and is not implemented.*

*2026-10-27 — RFC-106 (Draft): streaming chunk validation for AISchemaGate. A gate session appends each chunk (not the accumulated payload) and/or the same parser ships as WASM/FFI. `AISchemaGate.wrapStream` on `@nightmoose/aischemagate`, and `AISchemaGate.wrap_stream` on PyPI `aischemagate`, wrap an OpenAI `stream=True` iterator or Anthropic `messages.stream()` with `onValidationError` of `drop`, `throw`, or `repair`. Unrepairable chunks stop the provider after a caller-set budget. A speculative snapshot auto-closes brackets so a UI can parse the partial object. No code. Implementation waits on a UI mockup and owner approval.*

*2026-10-23 — RFC-105: ODCS import accepts `apiVersion` v3.1.0 and v3.2.0 and maps a property-level `enum` (scalar list or `{value}` objects, including an empty list and mixed or duplicate entries) onto the field constraint. `x-cg-enum` still wins over the property `enum` and over `invalidValues` / `validValues`. Export stays v3.1.0. The MCP registry listing names DataContractGate and AISchemaGate, and Smithery's default gateway is `https://api.datacontractgate.com`. `@nightmoose/aischemagate` gains `aischemagateGuardrail` (`wrapGenerate`) and `validateStructuredOutput`. Guides cover the Vercel AI SDK, Pydantic / Instructor, and LangGraph. Recipes cover a webhook quarantine firewall, a Dagster / Prefect gate, and quarantine replay.*

*2026-10-07 — v0.2.0 released (2026-10-01) to every channel: GitHub Release (CLI for 4 targets), PyPI `contractgate`, npm `@nightmoose/contractgate-sdk` (RFC-018) and `@nightmoose/contractgate-mcp-server` (RFC-090; the old `@contractgate/mcp-server` is deprecated), and the MCP Registry as `io.github.nightmoose/contractgate`. RFC-093 (egress as a first-class door) is on main. Release pipeline is idempotent and retryable — see the header of `.github/workflows/release.yml`.*

*2026-08-23 — RFC-090: official stdio MCP server at `mcp/` (`npx -y @contractgate/mcp-server`). Tools wrap existing gateway routes (infer, dry-run ingest / playground, deploy, quarantine, list). No engine changes. Reference: `docs/mcp-reference.md`, served raw at `/mcp-reference.md`.

*2026-09-11 — RFC-091: dashboard “What’s this?” inspect mode. Help catalog + `HelpTarget` on sidebar/page headers/dense controls; `?` / Esc; empty-state first-run on Dashboard / Contracts / Audit. No engine changes. Reference: `docs/help-mode-reference.md`.

*2026-08-13 — RFC-089 shipped: `docs/llm-integration.md` (agent-executable integration playbook) and `docs/llms.txt`, copied into `dashboard/public/` by a prebuild step and served raw at `app.datacontractgate.com/llm-integration.md` + `/llms.txt`; `datacontractgate.com` 307s to both, and the marketing site carries a copy-the-prompt block. Drift gate `tests/llm_docs_test.rs` asserts every endpoint the playbook cites exists in the router and that its example contract compiles. Same branch also replaced the CI migration file-count sentinel with a filename-contract check (contiguous NNN_ prefixes, no duplicates) plus `.github/workflows/migration-drift.yml`, which compares the prod ledger to `supabase/migrations/` daily — the check that would have caught 033/034 being applied-but-untracked. Migration 035 is held (listed in `supabase/unapplied-migrations.txt`) pending a re-audit against current signups.*

*Last updated: 2026-07-16 — RFC-078/079/080 implemented on `feature-RFCs_deferred`: RFC-079 unifies contract inference on the Rust engine (Generate-from-Sample now routes through `POST /contracts/infer`, nested objects infer correctly, JS inferrer removed); RFC-080 adds Visual Builder nested-object support; RFC-078 adds the cross-surface walkthrough spine + API/CSV/Kafka/Kinesis walkthroughs with cg-validated runnable examples. RFC-077 RAG profile moved to Accepted — contract, examples, and reference doc are shipped and engine-validated, but GA promotion is deferred until a RAG prospect is active.*

*2026-06-01 — RFC-076 adds `cg test --contract <FILE> --data <FILE>`: a zero-dependency local dry-run CLI over the existing `validate()` engine; accepts NDJSON, JSON array, or single object; exit 0/1/2; `--format json`, `--fail-fast`, `--quiet`; integration tests in `tests/cli_test_dryrun.rs`; reference doc at `docs/cg-test-reference.md`.*

*2026-05-28 — RFC-065 closes the ingest/egress cross-tenant authz gap (per-key `allowed_contract_ids` now enforced on all hot paths); RFC-066 removes the legacy env-var `API_KEY` master key (dev no-auth now gated on explicit `CONTRACTGATE_DEV_NO_AUTH=1`); RFC-067 converts six latent request-path panics (collaboration `expect`, replay `unwrap`) to clean 401 / graceful skip; RFC-068 wires the self-contained org-isolation DB tests into CI (`migrations-check`) so cross-tenant scoping is enforced on every PR; RFC-069 adds unit coverage for the previously-untested pure auth helper `jwks_url_from_database_url` (both Supabase connection-string formats + `None` fallbacks), `PublicationRow::is_revoked`, and `JwtAuthError` Display; RFC-070 extends the org-isolation DB test to the write-side version-mutation BOLA surface (`patch_version_yaml`/`deprecate_version`/`delete_version` wrong-org → `VersionNotFound`); RFC-071 adds a `cargo-llvm-cov` ratchet coverage gate to CI (fails only on a drop beyond tolerance vs a committed baseline; non-blocking for deploy while it beds in); RFC-072 adds a self-seeding DB test for the quarantine→replay race guard (`mark_quarantine_replayed_batch` stamps a source row at most once; the loser links to neither audit id), wired into the `migrations-check` named-test list; RFC-073 wired the cross-tenant isolation integration test (`cross_org_ingest_is_rejected`) into the compose-smoke lane against a fixed-UUID two-org seed — but that wiring proved to be a false signal: the compose stack runs `CONTRACTGATE_DEV_NO_AUTH=1`, so the gateway never validates keys and the test returns 200 with or without org scoping (the smoke step is now disabled pending a fix); RFC-074 fixes a latent P0-class cross-tenant write found by code inspection (the ingest/egress/v1-ingest handlers resolved the caller's `org_id` from the key but passed `None` to the storage lookup, so under production auth any unrestricted key could write to another org's contract) — the fix threads `org_id` through and is verified by code inspection + `cargo test`, not by the smoke lane; RFC-075 (Draft) specifies the auth-on (`DEV_NO_AUTH=0`) test lane needed to actually prove isolation end-to-end, with a no-key→401 sanity gate so the lane can't silently mislead again.*
