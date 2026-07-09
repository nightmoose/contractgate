# Contributing to ContractGate

Thanks for your interest in ContractGate. This guide covers how we plan, build,
and ship changes. The project is licensed MIT with a patent NOTICE — see
[`LICENSE`](LICENSE) and [`NOTICE`](NOTICE) before contributing.

## Ground rules

- **Performance and correctness are non-negotiable.** The validation engine must
  stay fast (sub-millisecond p99) and never regress existing behavior.
- **One issue or refactor per PR** unless agreed otherwise.
- Prefer simple, idiomatic Rust. Comment only where behavior isn't obvious.

## RFC-first for non-trivial work

Anything beyond a small bug fix starts with an RFC.

1. Add `docs/rfcs/NNN-short-slug.md` (next number after the highest in
   [`docs/rfcs/`](docs/rfcs/); see [`docs/STATUS.md`](docs/STATUS.md) for the
   current index).
2. Use the existing RFCs as a template: `Status`, `Date`, `Branch`, `Problem`,
   `Fix`, `Testing`, `Rollout`.
3. Land the code, then update `docs/STATUS.md` and `MAINTENANCE_LOG.md`.

Trivial fixes (typos, comment fixes, obvious one-liners) can skip the RFC.

## Branching

Branch from an up-to-date `origin/main` (local `main` can be stale — always
`git fetch` first). Name branches:

```
nightly-maintenance-YYYY-MM-DD-rfcNNN-short-slug
```

e.g. `nightly-maintenance-2026-05-28-rfc065-ingest-egress-scope`.

## Local development

**Backend (Rust):**

```bash
cargo check      # fast feedback
cargo build
cargo test       # see "Tests" below
cargo run        # needs DATABASE_URL (Supabase) — see .env.example
```

**Frontend (Next.js):**

```bash
cd dashboard
npm install
npm run build
```

**Full demo stack:**

```bash
make demo        # docker-compose: API + dashboard + seeded demo data
make demo-down   # tear down
make demo-reset  # rebuild from scratch
```

## Tests

Unit tests run without a database. Tests that need a live Postgres are tagged
`#[ignore]` and require `DATABASE_URL`:

```bash
cargo test                 # unit tests only
cargo test -- --ignored    # DB-backed tests (DATABASE_URL must be set)
```

New behavior needs test coverage — especially auth, storage, inference, and
quarantine paths. Don't mark a test `#[ignore]` to make CI pass; gate it on
`DATABASE_URL` and document why.

## Before you open a PR

- [ ] `cargo check` and `cargo test` pass.
- [ ] `cargo fmt` clean (see [`rustfmt.toml`](rustfmt.toml)).
- [ ] `cargo clippy` clean for changed code.
- [ ] Existing behavior preserved.
- [ ] User-facing changes (CLI flag, API endpoint, contract field, config key):
      update the relevant `docs/` page, or add `docs/<feature>-reference.md`.
- [ ] RFC added/updated and `docs/STATUS.md` + `MAINTENANCE_LOG.md` reflect the change.

## Security

Do not file public issues for security-sensitive findings — see
[`SECURITY.md`](SECURITY.md) for private disclosure.
