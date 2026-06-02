# RFC-019: CI + Release Pipeline

| Status        | Accepted (2026-04-27) — Q1–Q11 signed off; all rollout steps complete  |
|---------------|------------------------------------------------------------------------|
| Author        | ContractGate team                                                      |
| Created       | 2026-04-27                                                             |
| Target branch | `nightly-maintenance-YYYY-MM-DD` (set when work starts)                |
| Chunk         | Punchlist v2 — infrastructure (not in v2 build list; unblocks all chunks) |
| Supersedes    | —                                                                      |
| Depends on    | RFC-014 (CLI binary + `cli-cross-compile` job, both landed)            |
|               | RFC-005 (Python SDK + `sdks/python/pyproject.toml`, landed)            |

---

## Summary

Three workstreams that make ContractGate releasable end-to-end:

**A. Release workflow** — `.github/workflows/release.yml` builds the
`contractgate` CLI binary (RFC-014) for four targets, tarballs each,
attaches everything to a GitHub Release with auto-generated release
notes, and verifies the tag matches `Cargo.toml`.

**B. PyPI publish** — same release trigger publishes the Python SDK
(RFC-005) to TestPyPI (always) then to PyPI (tags only) via OIDC
Trusted Publisher. No API tokens.

**C. Migrations enforced in CI** — new `migrations-check` job in
`.github/workflows/ci.yml` spins a Postgres 16 service container,
applies every file in `supabase/migrations/` in order, then runs
`cargo sqlx prepare --check` to assert the committed `.sqlx/` query
metadata still matches the live schema.

None of these workstreams touch `rust-check`, `dashboard`, or `docker`
jobs.

---

## Goals

1. `git push --tags v0.1.0` (after a merged version-bump PR) produces a
   GitHub Release with four CLI tarballs, SHA256 sums, and release notes
   — no manual steps.
2. The same tag push publishes `contractgate==0.1.0` to PyPI via OIDC;
   no API token stored in repo secrets.
3. Every PR that touches `src/storage.rs` or `supabase/migrations/` must
   pass the `migrations-check` job or it cannot merge.
4. A freshly cloned repo applying migrations from scratch must succeed
   with no human intervention.

---

## Non-goals

- Windows CLI targets in v0.1. (macOS + Linux only, four targets total.)
- Code-signing / notarization of CLI binaries.
- Rollback testing for migrations (apply-clean is the contract).
- Homebrew tap, Scoop, Chocolatey. (`cargo install --git` + tarball.)
- PyPI Trusted Publisher for TestPyPI (OIDC there uses a different
  environment; TestPyPI publish uses a stored API token as a pragmatic
  exception — see Q5).
- Running Python SDK tests against the service-container DB. The SDK's
  test suite uses `httpx.MockTransport`; it does not need a live DB.
- Migrating the Supabase CLI into CI (`supabase db push`). Raw `psql`
  apply is simpler and eliminates a heavyweight toolchain dep.

---

## Decisions

| #   | Question | Recommendation |
|-----|----------|----------------|
| Q1  | Cross-compile tooling: `cross` vs `cargo zigbuild` vs native matrix | **`cross` for `aarch64-unknown-linux-gnu`, native `cargo build` for the other three.** Mirrors the existing `cli-cross-compile` smoke job in `ci.yml` exactly — same runners, same `cross` install step. No new tooling. `cargo zigbuild` is promising but adds a zig install step and is untested on this codebase; defer until we have a reason. |
| Q2  | Version-sync direction: `Cargo.toml` drives, or tag drives? | **`Cargo.toml` is the source of truth.** Developer bumps `Cargo.toml` + `sdks/python/pyproject.toml` in a PR, merges it, then tags the merge commit. The release job reads `Cargo.toml`'s `[package].version` at the start, strips the leading `v` from the pushed tag, and fails immediately if they differ. Tag does not set the version. |
| Q3  | Release notes generation: `release-drafter`, `git-cliff`, GitHub auto-generated, or hand-rolled | **GitHub's built-in auto-generated release notes** (`generate_release_notes: true` in the `gh release create` step). Zero config, zero maintenance, good enough for pre-customer. `git-cliff` (conventional commits → categorized changelog) deferred until we adopt conventional-commit discipline — imposing it now would be a mis-design. |
| Q4  | Tarball naming convention | **`contractgate-<VERSION>-<TARGET>.tar.gz`** (e.g., `contractgate-0.1.0-x86_64-unknown-linux-gnu.tar.gz`). Version included in the filename so each release's artifacts are unambiguous in the GitHub Release asset list. The reference workflow `contractgate.yml` currently downloads `contractgate-linux-x86_64.tar.gz` — that URL will need updating to the versioned form (see Rollout step 7). |
| Q5  | PyPI Trusted Publisher: environment names + TestPyPI auth | **Production PyPI:** OIDC environment named `pypi`, project name `contractgate` (RFC-005 Q1). **TestPyPI:** publish with a stored `TEST_PYPI_API_TOKEN` repo secret — TestPyPI's Trusted Publisher support is less mature and the token is low-risk (no production packages). Note this is the only stored secret in the pipeline; document it in the repo secrets audit. |
| Q6  | Postgres image tag in CI | **`postgres:16`** (major version pin, no minor). Supabase runs Postgres 16. Minor-version pins would require manual bumps with no meaningful benefit for a migrations-apply job. |
| Q7  | `migrations-check` scope: gateway DB only, or also Python SDK test fixtures? | **Gateway DB only.** The Python SDK's test suite uses `httpx.MockTransport` and has no live-DB dependency. Including it here would couple two unrelated concerns and add ~3 minutes of install time. If the SDK grows live-DB tests, they get their own job. |
| Q8  | SDK version bump policy: lockstep with gateway, or independent? | **Independent SemVer**, per RFC-005 §Versioning. SDK 0.1.0 ships alongside gateway 0.1.0, but subsequent bumps are decoupled. Wire-format breaks (gateway changes a response shape) require a coordinated major bump and a migration guide; that policy is already locked in RFC-005. The release workflow publishes whatever version is in `sdks/python/pyproject.toml` at tag time — no auto-sync. |
| Q9  | `.sqlx/` bootstrap: does it exist? | **It does not exist yet.** `.sqlx/` is generated by `cargo sqlx prepare` against a live database and must be committed before `--check` mode works. Rollout step 1 of workstream C is to generate and commit this directory. Until that commit lands, `SQLX_OFFLINE` in the existing `rust-check` job would need to be set `false` or the `.sqlx/` files would need to be seeded. This RFC's rollout creates them as part of implementation. |
| Q10 | Migration apply tool in CI: `psql` or `sqlx migrate`? | **`psql` in a loop**, applying files in `ls -v` (natural sort) order. The migration files use `NNN_name.sql` naming with no sqlx migration headers; `sqlx migrate` expects a specific format and a `_sqlx_migrations` table. Raw `psql` is simpler, needs no extra tooling, and is already how the files are applied against Supabase manually. A comment in the job documents the naming convention contract (`NNN_` prefix, no gaps). |
| Q11 | SHA256 approach | **Single `SHA256SUMS` file** listing all four tarballs, attached as a fifth release asset. Generated with `sha256sum *.tar.gz > SHA256SUMS`. Consumers can run `sha256sum -c SHA256SUMS` after download. |

---

## Design

### A. `.github/workflows/release.yml`

**Triggers:**

```yaml
on:
  push:
    tags: ['v*.*.*']
  workflow_dispatch:
    inputs:
      tag:
        description: 'Tag to release (e.g. v0.1.0)'
        required: true
```

**Jobs:**

```
version-check
└── build-cli (matrix: 4 targets)
    └── publish-release
        └── publish-pypi (separate job, same trigger)
```

**`version-check` job** (ubuntu-latest, ~10 s):
1. `actions/checkout@v4`
2. Read `Cargo.toml` version with `grep -m1 '^version' Cargo.toml | cut -d'"' -f2`.
3. Strip leading `v` from `${{ github.ref_name }}` (or `inputs.tag`).
4. Fail if they differ: `[[ "$CARGO_VERSION" != "$TAG_VERSION" ]] && exit 1`.

**`build-cli` matrix job** (depends on `version-check`):

Same matrix as `cli-cross-compile` in `ci.yml`:

| target | runner | tool |
|--------|--------|------|
| `x86_64-unknown-linux-gnu` | `ubuntu-latest` | `cargo build --release` |
| `aarch64-unknown-linux-gnu` | `ubuntu-latest` | `cross build --release` |
| `x86_64-apple-darwin` | `macos-latest` | `cargo build --release` |
| `aarch64-apple-darwin` | `macos-latest` | `cargo build --release` |

Each matrix leg:
1. `actions/checkout@v4`
2. `dtolnay/rust-toolchain@stable` with `targets: ${{ matrix.target }}`
3. `actions/cache@v4` (same cache key pattern as `cli-cross-compile`)
4. Install `cross` if `matrix.cross == true`
5. Build: `cross build --release --bin contractgate --target ${{ matrix.target }}` or `cargo build --release --bin contractgate --target ${{ matrix.target }}`
6. Rename binary: `cp target/${{ matrix.target }}/release/contractgate contractgate`
7. Create tarball: `tar czf contractgate-${{ env.TAG_VERSION }}-${{ matrix.target }}.tar.gz contractgate LICENSE README.md`
8. `actions/upload-artifact@v4` with name `tarball-${{ matrix.target }}`

**`publish-release` job** (ubuntu-latest, depends on all `build-cli` legs):
1. `actions/checkout@v4`
2. `actions/download-artifact@v4` (all `tarball-*` artifacts → `./dist/`)
3. `sha256sum ./dist/*.tar.gz > ./dist/SHA256SUMS`
4. `gh release create ${{ github.ref_name }} ./dist/*.tar.gz ./dist/SHA256SUMS --generate-notes --title "ContractGate ${{ github.ref_name }}"` (uses `GITHUB_TOKEN`)

**Note on `contractgate.yml` reference workflow:** The hardcoded `contractgate-linux-x86_64.tar.gz` URL must be updated to the versioned filename (see Rollout step 7). The reference workflow will use `releases/latest/download/contractgate-<VERSION>-x86_64-unknown-linux-gnu.tar.gz` for pinned consumers, or a separate `latest`-tagged symlink approach. For v0.1 simplicity, update the reference workflow to use `releases/latest/download/contractgate-<VERSION>-x86_64-unknown-linux-gnu.tar.gz` and document the version in a comment.

---

### B. PyPI Publish Job

Separate job in `release.yml`, depends on `version-check`, runs in
parallel with `build-cli`.

**PyPI Trusted Publisher setup (one-time, done before rollout):**
- On PyPI: Add a Trusted Publisher for project `contractgate`, repo
  `contractgate/contractgate`, workflow `release.yml`, environment `pypi`.
- On TestPyPI: Store `TEST_PYPI_API_TOKEN` in GitHub repo secrets (not
  environment-scoped; TestPyPI OIDC support is incomplete as of 2026-04).

**`publish-pypi` job:**

```yaml
publish-pypi:
  needs: version-check
  runs-on: ubuntu-latest
  environment: pypi          # gates the OIDC token for production PyPI
  permissions:
    id-token: write          # required for Trusted Publisher
  steps:
    - uses: actions/checkout@v4

    - name: Verify SDK version matches tag
      run: |
        SDK_VERSION=$(grep -m1 '^version' sdks/python/pyproject.toml | cut -d'"' -f2)
        TAG_VERSION="${{ github.ref_name }}"
        TAG_VERSION="${TAG_VERSION#v}"
        [[ "$SDK_VERSION" == "$TAG_VERSION" ]] || \
          { echo "SDK version $SDK_VERSION != tag $TAG_VERSION"; exit 1; }

    - uses: actions/setup-python@v5
      with:
        python-version: '3.12'

    - name: Install build
      run: pip install build --break-system-packages

    - name: Build wheel + sdist
      working-directory: sdks/python
      run: python -m build

    - name: Publish to TestPyPI
      uses: pypa/gh-action-pypi-publish@release/v1
      with:
        repository-url: https://test.pypi.org/legacy/
        packages-dir: sdks/python/dist/
        password: ${{ secrets.TEST_PYPI_API_TOKEN }}

    - name: Publish to PyPI (tags only)
      if: startsWith(github.ref, 'refs/tags/')
      uses: pypa/gh-action-pypi-publish@release/v1
      with:
        packages-dir: sdks/python/dist/
        # No password — OIDC Trusted Publisher via id-token: write permission
```

---

### C. `migrations-check` Job in `ci.yml`

**Placement:** New job in `.github/workflows/ci.yml`, independent of
`rust-check`, `dashboard`, `docker`, `cli-cross-compile`. Runs on every
push/PR to main.

**Postgres service container:**

```yaml
services:
  postgres:
    image: postgres:16
    env:
      POSTGRES_USER: contractgate
      POSTGRES_PASSWORD: contractgate
      POSTGRES_DB: contractgate_test
    ports:
      - 5432:5432
    options: >-
      --health-cmd pg_isready
      --health-interval 10s
      --health-timeout 5s
      --health-retries 5
```

**Job steps:**

1. `actions/checkout@v4`

2. Install `psql` client:
   ```bash
   sudo apt-get install -y postgresql-client
   ```

3. Apply migrations in order:
   ```bash
   for f in $(ls -v supabase/migrations/*.sql); do
     echo "Applying $f"
     psql "$DATABASE_URL" -f "$f"
   done
   ```
   Fails the step immediately if any migration exits non-zero.

4. Install Rust + sqlx-cli:
   ```bash
   # dtolnay/rust-toolchain@stable (reuse from rust-check's cache pattern)
   cargo install sqlx-cli --no-default-features --features postgres,rustls
   ```
   (Pinned: `--version 0.7.4` to match the current sqlx dep; bump in lockstep with the sqlx upgrade RFC.)

5. Run `sqlx prepare --check`:
   ```bash
   cargo sqlx prepare --check --workspace
   ```
   with `DATABASE_URL` pointing at the service container. Fails if
   `.sqlx/` metadata is stale or missing queries.

6. Assert migration file count matches applied count (belt-and-braces):
   ```bash
   EXPECTED=$(ls supabase/migrations/*.sql | wc -l)
   APPLIED=$(psql "$DATABASE_URL" -Atc "SELECT COUNT(*) FROM information_schema.tables WHERE table_name = '_sqlx_migrations'" 2>/dev/null || echo 0)
   # We use psql-count of files as the contract, not a migrations table
   # (raw psql apply doesn't write a tracking table).
   # Verify the schema has the latest migration's sentinel object instead:
   psql "$DATABASE_URL" -c "\d contracts" | grep -q "id" || \
     { echo "Schema check failed — migrations did not apply cleanly"; exit 1; }
   ```
   
   **Simpler alternative (recommended):** Use a sentinel query — assert
   that the column introduced by the latest migration (009) exists:
   ```bash
   psql "$DATABASE_URL" -Atc \
     "SELECT column_name FROM information_schema.columns \
      WHERE table_name='contracts' AND column_name='github_repo_url'" \
     | grep -q "github_repo_url" || \
     { echo "Migration 009 sentinel column missing"; exit 1; }
   ```
   This is concrete, fast, and doesn't depend on tracking tables.
   Update the sentinel column whenever the latest migration changes.

**Environment variable:**
```yaml
env:
  DATABASE_URL: postgres://contractgate:contractgate@localhost:5432/contractgate_test
```

**Why this catches the `storage.rs`-without-migrations failure mode:**
- New `sqlx::query!` call in `storage.rs` → `cargo sqlx prepare --check`
  fails (new query not in `.sqlx/` metadata).
- Schema change in `storage.rs` that requires a migration → apply step
  fails or `--check` fails because the compiled query shape doesn't
  match the schema the query was prepared against.
- New migration file added but `.sqlx/` not regenerated → `--check`
  fails.

---

## Prerequisites (before rollout)

1. **Generate `.sqlx/` metadata.** The directory does not exist in the
   repo today. This RFC's first implementation step runs
   `cargo sqlx prepare --workspace` against a local Postgres with all
   nine migrations applied, then commits the result. Without this,
   `--check` cannot run.

2. **Reserve `contractgate` on PyPI.** RFC-005 noted this as a separate
   ops task. Must be done before `publish-pypi` job can succeed. Reserve
   `contractgate-sdk` and `cg-sdk` as placeholder packages per RFC-005
   Q1.

3. **Configure Trusted Publisher on PyPI.** One-time setup in the PyPI
   project settings: workflow file `release.yml`, environment `pypi`.

4. **Add `TEST_PYPI_API_TOKEN` to repo secrets.** One-time.

---

## Test Plan

### A. Release workflow

1. Trigger `workflow_dispatch` with a matching tag on a PR branch (not
   main). Verify:
   - Version-check step passes.
   - All four cross-compile legs produce tarballs with the correct names.
   - `publish-release` step creates a draft GitHub Release with four
     `.tar.gz` files + `SHA256SUMS`.
   - `sha256sum -c SHA256SUMS` passes locally after downloading artifacts.
2. Force a version mismatch (tag `v0.1.0` when `Cargo.toml` says `0.1.1`).
   Verify `version-check` job fails and no release is created.
3. Verify the downloaded Linux binary runs:
   `contractgate --version` → `contractgate 0.1.0`.

### B. PyPI publish

4. Trigger on a PR branch. Verify:
   - TestPyPI publish succeeds (`pip install -i https://test.pypi.org/simple/ contractgate==<version>` works).
   - PyPI step is skipped (not a tag push from a `refs/tags/` ref).
5. Force SDK version mismatch. Verify the version-check step fails.

### C. Migrations check

6. On a clean PR: verify `migrations-check` passes green.
7. Add a new `sqlx::query!` to `src/storage.rs` without running `cargo
   sqlx prepare`. Verify `migrations-check` fails on `--check`.
8. Add a new `.sql` file to `supabase/migrations/` with a syntax error.
   Verify the apply step fails.
9. Delete an existing migration file. Verify the sentinel column check
   (or a schema-level assertion) fails.

---

## Rollout

1. ✅ Sign off this RFC (Q1–Q11 decisions). Alex sign-off 2026-04-27.
2. ✅ Run `cargo sqlx prepare --workspace` locally (all 9 migrations applied
   against a Postgres 16 instance). Commit `.sqlx/` metadata. Confirmed 2026-04-27.
3. ✅ One-time ops: reserve `contractgate` on PyPI + TestPyPI; configure
   Trusted Publisher environment `pypi` on pypi.org; add
   `TEST_PYPI_API_TOKEN` to repo secrets. Confirmed 2026-04-27.
4. ✅ Implement `migrations-check` job in `.github/workflows/ci.yml`.
5. ✅ Implement `.github/workflows/release.yml` (version-check +
   build-cli matrix + publish-release + publish-pypi jobs).
6. ✅ Update `.github/workflows/contractgate.yml` reference workflow:
   versioned tarball URL with pinned `CONTRACTGATE_VERSION` env var;
   comment explaining consumers must bump the version explicitly.
7. ✅ Trigger `workflow_dispatch` on a branch to smoke-test the full
   release.yml without pushing a real tag. Tarballs, SHA256SUMS, and
   TestPyPI publish verified. Confirmed 2026-04-27.
8. ✅ Append run entry to `MAINTENANCE_LOG.md`.
9. ✅ Update this RFC's Rollout with checkmarks on completed steps.

---

## Deferred

- Windows CLI targets (`x86_64-pc-windows-msvc`). No pilot demand.
- Code-signing / notarization for macOS binaries.
- Homebrew tap formula.
- `git-cliff` conventional-commit changelog. Revisit when we adopt
  conventional commits across the team.
- TestPyPI Trusted Publisher (OIDC). Revisit when TestPyPI's support
  matures.
- Rollback testing for migrations (separate RFC if needed).
- `supabase migrate` CLI in CI. Prefer raw psql; revisit if we adopt
  Supabase CLI for local dev.
- Migration tracking table (e.g., adopting sqlx migrate format or
  Flyway). The sentinel-column approach is sufficient for v0.1.
- Python SDK CI job (lint + test) — that is a separate `sdk-python` job
  to be added when the SDK grows past the current 43 tests. Not in scope
  for this RFC.
