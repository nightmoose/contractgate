# Chunk 8 — Multi-Tenant SaaS

**Theme:** Make the managed plane real. Org isolation, billing, signup, rate limits.
**Why last:** Gated on RFC-001 (tenancy model) sign-off. Highest blast radius. Don't start until upstream chunks stabilize the API surface.

## Items

- [ ] Multi-tenant namespace isolation (per-org data plane) `[L]`.
- [ ] Usage metering service `[M]` — API calls, contracts stored, connector events.
- [ ] Self-serve org signup + onboarding `[M]` — email verification, first-contract wizard.
- [ ] API rate limiting + quota enforcement per plan tier `[M]`.
- [ ] Terraform provider `[L]` — manage contracts, policies, connectors as IaC.
- [ ] Kubernetes Operator with CRDs (`ContractGateInstance`, `ContractPolicy`) `[XL]`.

## Hard prerequisite

- **RFC-001 sign-off.** Per `project_tenancy_model.md`, org-scoped (Option B) is decided but not signed. Do not start implementation until signed.
- Existing per-contract scoping (salts, quotas, audit) must be re-keyed onto org id without breaking history.

## Adjacent decisions to revisit

- sqlx 0.7 → 0.8 upgrade (`project_sqlx_upgrade_deferred.md`). Multi-tenant work likely triggers it. RFC the upgrade when it does.
- Enterprise ACLs deferred to standalone instance — keep them out of this chunk's scope.

## Open questions for the conversation

1. Isolation model: row-level (org_id everywhere) vs schema-per-tenant vs database-per-tenant. RFC-001 likely specifies; confirm.
2. Metering granularity — per-event row, hourly rollup, or both?
3. Rate limit dimension — org, API key, or both? Token bucket or sliding window?
4. Signup flow: email-only first, or SSO from day one?
5. Terraform + Operator — defer until org isolation lands; do not parallelize.

## Suggested first step

Get RFC-001 signed. Then write `docs/rfcs/00X-tenancy-impl-plan.md` mapping every existing table to its org-id migration. Nothing ships before that doc.
