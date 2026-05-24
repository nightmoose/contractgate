# Schema Notes — Deprecated Columns

This document records schema columns that are present in the database but
superseded by the current data model. They must **not** be written to by new
code. They are retained to avoid a potentially risky `ALTER TABLE … DROP COLUMN`
until a dedicated cleanup migration is reviewed and scheduled.

---

## `contracts` table — legacy columns from migration 001

Three columns on the `contracts` table were part of the original v1 schema
(migration `001_initial_schema.sql`) before the versioning model was introduced
in migration `003_contract_versioning.sql`. All three have been superseded by
the `contract_versions` table and must not be read or written by application
code.

| Column | Original type | Status | Superseded by |
|---|---|---|---|
| `contracts.version` | `TEXT NOT NULL DEFAULT '1.0'` | **Deprecated** | `contract_versions.version` |
| `contracts.active` | `BOOLEAN NOT NULL DEFAULT TRUE` | **Deprecated** | `contract_versions.state` (draft/stable/deprecated) |
| `contracts.yaml_content` | `TEXT NOT NULL` | **Deprecated** | `contract_versions.yaml_content` |

### Verification

As of 2026-05-24, confirmed by grep across `src/` and `dashboard/`:

- No `INSERT INTO contracts` or `UPDATE contracts` query in `src/` references
  `version`, `active`, or `yaml_content` on the `contracts` table directly.
- `yaml_content` appears extensively in code but always in the context of
  `contract_versions.yaml_content`, not `contracts.yaml_content`.
- The dashboard does not read or write these columns.

### Why not dropped yet

Dropping columns requires careful coordination (zero-downtime migration, replica
lag, ORM sync). A follow-up migration (`028_deprecate_legacy_contract_columns`)
that adds `COMMENT ON COLUMN` markers and a subsequent `029_drop_legacy_contract_columns`
should be planned once the versioning model has had sufficient production
bake-time. That work is deferred from RFC-057.

---

*Added 2026-05-24 as part of RFC-057 documentation completeness pass.*
