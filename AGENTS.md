# Contributor guide (humans and AI coding assistants)

DataContractGate is a semantic data-contract enforcement gateway. AISchemaGate
shares the same engine. See [`README.md`](README.md) for the overview and
[`CONTRIBUTING.md`](CONTRIBUTING.md) for the full process.

## Stack
- Rust (Axum) gateway in `src/`. The validation engine must stay fast (p99 well under 15 ms) and correct.
- Next.js + TypeScript + Tailwind dashboard in `dashboard/`.
- Supabase Postgres (contracts, audit log, quarantined events); migrations in `supabase/migrations/`.

## Working rules
- One issue or refactor per PR. Don't break existing behavior.
- Prefer simple, idiomatic Rust; comment only where behavior isn't obvious.
- Non-trivial work starts with an RFC in `docs/rfcs/`; update [`docs/STATUS.md`](docs/STATUS.md) when it lands.
- User-facing changes (CLI flag, API endpoint, contract field, config key): update the matching `docs/` page or add `docs/<feature>-reference.md`.
- Open PRs against `staging`. Use roles, not names, in commits, PRs and docs.
- Public prose says DataContractGate or AISchemaGate; identifiers such as `contractgate` crate names and `CONTRACTGATE_*` settings stay as they are.

## Commands
```
cargo check
cargo test
cargo clippy --all-targets

cd dashboard && npm ci && npm run build
```

## Contract format
Contracts are YAML with `version`, `name`, `description`, `ontology.entities`
(fields with `type`, `required`, `pattern`, `enum`, `min`/`max`), `glossary` and
`metrics`. See `docs/` and `examples/` for full references.
