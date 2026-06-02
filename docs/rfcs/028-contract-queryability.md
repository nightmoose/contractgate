# RFC-028: Contract Queryability — Supabase-Backed Contract Store

**Status:** Accepted  
**Date:** 2026-05-14  
**Author:** Alex Suarez  

---

## Problem

Contracts currently live as YAML files in git. This is correct for authoring and version control, but it creates a gap: the audit log (in Supabase) records *which contract version* validated each event, yet the contracts themselves are not co-located in the database. As a result:

- Joining "what rules were active during incident window X" to the audit log requires manual git archaeology.
- There is no SQL-accessible record of which contracts contain specific field configurations (e.g., "show me all contracts where `monthly_rent` has `min: 0` — those are the ones that could pass bad Yardi data").
- Per-contract quarantine rates, violation breakdowns, and field-level error frequencies cannot be queried without parsing YAML out-of-band.
- External stakeholders (property managers, auditors) cannot be given read-only visibility into active contract definitions without DB access.

This gap surfaced during a Findigs evaluation conversation: the claim that contracts are "queryable" was aspirational, not accurate. The audit log is queryable; the contracts are not.

---

## Proposed Solution

Mirror every contract into a `contracts` Supabase table on deploy, making contracts first-class queryable objects alongside the audit log.

### Schema (proposed)

```sql
CREATE TABLE contracts (
  id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  name         text NOT NULL,
  version      text NOT NULL,
  source       text NOT NULL,             -- PMS vendor or logical feed name
  yaml_content text NOT NULL,             -- raw YAML, authoritative copy in git
  parsed_json  jsonb,                     -- parsed ontology for SQL querying
  deployed_at  timestamptz NOT NULL DEFAULT now(),
  deployed_by  text,                      -- CI job or user
  active       boolean NOT NULL DEFAULT true,
  UNIQUE (name, version)
);
```

### What this unlocks

**Incident investigation:**
```sql
-- What contract version was active when this event was quarantined?
SELECT c.yaml_content
FROM audit_log a
JOIN contracts c ON c.name = a.contract_name AND c.version = a.contract_version
WHERE a.event_id = 'evt_abc123';
```

**Field-level risk queries:**
```sql
-- Which active contracts allow monthly_rent = 0?
SELECT name, version
FROM contracts
WHERE active = true
  AND (parsed_json -> 'ontology' -> 'entities' @> '[{"name":"monthly_rent","min":0}]');
```

**Per-contract violation rates:**
```sql
SELECT contract_name, contract_version, outcome, count(*)
FROM audit_log
GROUP BY 1, 2, 3
ORDER BY 4 DESC;
```

**Auditor read-only view (no full DB access required):**
```sql
CREATE VIEW active_contracts_public AS
SELECT name, version, source, deployed_at, parsed_json
FROM contracts
WHERE active = true;
-- Grant SELECT on this view only
```

### Integration points

- **Deploy pipeline:** `cargo run -- deploy-contract <file>` upserts into `contracts` table and sets previous version `active = false`.
- **Audit log:** `contract_version` column already exists — no migration needed on `audit_log`.
- **Git remains authoritative:** the `yaml_content` column is the stored copy; git is the source of truth for authoring. A CI check can assert they are in sync.

---

## Out of Scope (this RFC)

- User-facing contract browser UI (dashboard feature, separate RFC)
- Contract diff / changelog UI
- Tenant/org-scoped contract visibility (deferred per RFC-001 tenancy model)

---

## Open Questions — Resolved 2026-05-14

1. **`parsed_json` generation:** At deploy time. The `deploy_contract_version` storage function serializes the parsed `Contract` struct to JSONB and writes it alongside the YAML. No lazy logic.
2. **Who can deactivate:** Admin-only. Deprecation of prior stable versions is a side-effect of `POST /contracts/deploy`, which requires a service-role API key. No direct deprecation-by-name endpoint is exposed to regular authenticated users.
3. **Quarantine guard:** Yes — block. If the contract has any `status = 'pending'` quarantine events, the deploy call is rejected with a 400. Operators must resolve or purge pending events before rolling a new stable version.

---

## Acceptance Criteria

- [ ] Migration adds `contracts` table with schema above
- [ ] `deploy-contract` CLI command upserts contract row and marks prior version inactive
- [ ] `audit_log.contract_version` JOIN to `contracts` returns correct YAML for all existing test fixtures
- [ ] `active_contracts_public` view exists and is grantable without exposing full DB
- [ ] `cargo test` passes; no existing audit behavior changed
