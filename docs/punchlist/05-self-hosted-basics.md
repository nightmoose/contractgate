# Chunk 5 — Self-Hosted Basics

**Theme:** Make on-prem install boring. Helm, Compose, RBAC, air-gap.
**Why now:** Enterprise table stakes. Unblocks the standalone enterprise instance referenced in RFC-001.

## Items

- [ ] Docker Compose reference stack `[S]` — ContractGate + Postgres + Kafka + Prometheus, one `docker compose up`.
- [ ] Official Helm chart `[M]` — values.yaml covers replicas, ingress, secrets, Postgres conn.
- [ ] RBAC role editor UI `[M]` — extends existing partial RBAC. Fine-grained perms (read/write/admin per contract or namespace).
- [ ] Air-gapped install bundle + docs `[M]` — pinned image tarballs, offline Helm chart, license-key flow.

## Deferred

- Kubernetes Operator with CRDs `[XL]` — needs Helm to mature first.
- Terraform provider `[L]` — pairs with Operator, defer together.

## Surface to reuse

- Existing partial RBAC (already shipped per punchlist).
- Prometheus `/metrics` from Chunk 4 (Helm chart should expose it).

## Open questions for the conversation

1. Compose stack — bake in demo seed data, or empty by default?
2. Helm: single chart or umbrella (api + dashboard + worker as subcharts)?
3. RBAC scope: per-contract perms, or namespace-level only? Tenancy model from RFC-001 constrains this — read it first.
4. Air-gap: which registries do we mirror (ghcr.io? quay.io?), and how is the license key validated offline?
5. RFC required for RBAC perm model; Helm + Compose can be skip-RFC.

## Suggested first step

Compose stack first (a day's work, big external-facing win). Helm chart next. Re-read `project_tenancy_model.md` before scoping the RBAC editor.
