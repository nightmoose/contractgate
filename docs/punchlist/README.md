# Punchlist Chunks

Source: `contractgate-upgrade-punchlist.md` (2026-04-26).

JSON inference shipped 2026-04-27 morning (`src/infer.rs`). Strike that line.

Each chunk = one conversation. RFC pre-written for chunks 2–8 (Draft,
awaiting sign-off). Use [KICKOFF_PROMPT.md](KICKOFF_PROMPT.md) to start
a chunk — paste it verbatim, replace placeholders.

| # | Chunk | RFC | Why now |
|---|---|---|---|
| 1 | [Inference family](01-inference-family.md) | [006](../rfcs/006-inference-formats.md) (landed) | Extends today's endpoint. |
| 2 | [CLI + GitOps core](02-cli-gitops-core.md) | [007](../rfcs/007-cli-gitops-core.md) | Unlocks SDKs, CI templates, GitOps. |
| 3 | [Breaking-change story](03-breaking-change-story.md) | [008](../rfcs/008-breaking-change-story.md) | Lean on versioning. |
| 4 | [Observability quick wins](04-observability-quick-wins.md) | [009](../rfcs/009-observability-quick-wins.md) | Five items. Self-host parity. |
| 5 | [Self-hosted basics](05-self-hosted-basics.md) | [010](../rfcs/010-self-hosted-basics.md) | Helm + Compose + RBAC. |
| 6 | [SDK rollout](06-sdk-rollout.md) | [011](../rfcs/011-sdk-rollout.md) | Needs CLI surface stable. |
| 7 | [Templates + marketplace](07-templates-marketplace.md) | [012](../rfcs/012-templates-marketplace.md) | Network effect. |
| 8 | [Multi-tenant SaaS](08-multitenant-saas.md) | [013](../rfcs/013-multitenant-saas.md) | **Gated on RFC-001 sign-off.** |
