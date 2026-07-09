# RFC-057 — Documentation completeness for public launch

**Status:** Accepted  
**Date:** 2026-05-22  
**Accepted:** 2026-05-24  
**Branch:** nightly-maintenance-2026-05-24-rfc057  
**Addresses:** REVIEW-2026-05-22-launch-readiness M2  
**Severity:** P2 — medium

---

## Problem

CLAUDE.md requires a `docs/<feature>-reference.md` for every user-facing
surface. Several shipped features have no reference doc, and supporting docs
are stale:

1. **Missing feature docs.** No `docs/*-reference.md` exists for:
   - Kinesis ingress (RFC-026)
   - CSV inference (RFC-035) and URL inference (RFC-037)
   - Public catalog — fork + export (RFC-034)
   - Supabase-JWT dashboard auth (RFC-039)
   - The `date` contract type (RFC-044)
   - Plan gating / tiers (RFC-045)
2. **No RFC status index.** `docs/rfcs/` holds 46 RFCs with no single
   shipped-vs-draft-vs-superseded view. `MAINTENANCE_LOG.md` helps but is
   chronological. A new contributor cannot tell what is live.
3. **Stale CLAUDE.md MCP section.** CLAUDE.md documents a `code-review-graph`
   MCP with tools (`detect_changes`, `query_graph`, `get_impact_radius`) that
   is not connected in this environment. It misleads anyone following the
   instructions.
4. **Dead schema columns undocumented.** `contracts.version`,
   `contracts.active`, `contracts.yaml_content` were superseded by the
   versioning model but remain in the schema with no "deprecated, do not
   write" annotation — they drift out of sync with `contract_versions`.

---

## Fix

1. **Write the six missing reference docs**, each following the structure of
   the existing ones (`scorecard-reference.md`, `egress-validation-reference.md`):
   endpoints, request/response shapes, config keys, examples, edge cases.
2. **Add `docs/STATUS.md`** — a table of every RFC: number, title, status
   (Shipped / Accepted / Draft / Superseded), and the nightly run it shipped
   in. Link it from `README.md` and `CLAUDE.md`.
3. **Reconcile the CLAUDE.md MCP section** — either remove the
   `code-review-graph` section, or mark it optional and gate the "ALWAYS use
   the graph first" instruction on the MCP actually being connected.
4. **Annotate or drop the dead columns.** Add a migration comment (or a
   `COMMENT ON COLUMN`) marking the three legacy `contracts` columns as
   deprecated. Dropping them is a larger change — defer to a follow-up unless
   nothing reads them (verify with a grep across `src/` and `dashboard/`).
5. **README accuracy.** Reconcile the headline `<10 µs p99` latency claim
   with a measured, reproducible benchmark, or soften it to the verified
   number. An unverifiable public performance claim is a credibility and
   (given "Patent Pending" framing) marketing-accuracy risk.

---

## Testing

- Each new doc is linked from `README.md` or the docs index and renders.
- `docs/STATUS.md` row count equals the RFC file count.
- The README latency figure matches a benchmark committed under `ops/` or
  `tests/`.

## Rollout

Docs-only — no code, no migration (except the optional `COMMENT ON COLUMN`).
Can land incrementally; should be complete before the launch announcement,
not necessarily before first pilot.
