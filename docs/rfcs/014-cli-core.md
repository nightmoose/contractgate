# RFC-014: CLI Core + Reference GitHub Actions Workflow

| Status        | Accepted (2026-04-27) — sign-off on Q1..Q6 (recommendations)          |
|---------------|------------------------------------------------------------------------|
| Author        | ContractGate team                                                      |
| Created       | 2026-04-27                                                             |
| Target branch | `nightly-maintenance-YYYY-MM-DD` (set when work starts)                |
| Chunk         | Punchlist v2 #1                                                        |
| Supersedes    | RFC-007 (broader scope; trimmed to pre-customer essentials)            |

## Summary

Ship the `contractgate` CLI binary with three subcommands: `push`, `pull`,
`validate`. Add a `.contractgate.yml` config file. Commit one reference
GitHub Actions workflow that consumes the binary directly. That's it.

No GitLab/Bitbucket templates, no pre-commit framework, no GitOps loop,
no Marketplace action sub-repo. Those return when there's pull from
real users.

## Goals

1. Single static binary; cross-compiled on release for Linux/macOS x86_64
   and arm64.
2. Three subcommands in v0.1: `push`, `pull`, `validate`. Auth via env var.
3. `.contractgate.yml` discovered via walk-up from cwd.
4. Both human-readable and `--json` output modes.
5. Reference workflow `.github/workflows/contractgate.yml` checked into
   *this* repo as both example and dogfood.

## Non-goals

- `diff` subcommand → RFC-015.
- `template` subcommand → not needed for pre-customer.
- Pre-commit hook installer.
- Homebrew tap, Scoop, Chocolatey distribution. `cargo install` + GH
  Releases tarballs only.
- OS keychain auth. Env var only.
- Workspace split into `contractgate-core`. Deferred until a second
  consumer (the SDK in RFC-018) makes it pay off.

## Decisions

| #  | Question | Recommendation |
|----|----------|----------------|
| Q1 | Crate location | **`src/bin/contractgate.rs`** in the gateway crate. No workspace split yet. |
| Q2 | Auth | **`CONTRACTGATE_API_KEY` env var** + `--api-key` flag override. |
| Q3 | Output | **Human default; `--json` flag** flips every command. |
| Q4 | Config discovery | **Walk up from cwd** for `.contractgate.yml`, stop at git root. `--config` overrides. |
| Q5 | Distribution | **`cargo install --git ...` + GH Releases tarballs.** Homebrew deferred. |
| Q6 | GH Actions hosting | **Workflow YAML committed in this repo.** No separate marketplace action. Workflow downloads the release tarball at run time. |

## Design

### Binary layout

```
src/bin/contractgate.rs           # clap entry, dispatches to commands
src/cli/                           # NEW module
├── mod.rs
├── commands/{push.rs, pull.rs, validate.rs}
├── config.rs                      # .contractgate.yml parse + walk-up
├── client.rs                      # reqwest blocking wrapper
└── output.rs                      # human / json render switch
```

The CLI links the gateway's existing `crate::contract::Contract` and
`crate::validation` modules directly — no shared-crate split.

### `.contractgate.yml`

```yaml
version: "1.0"
gateway:
  url: "https://gw.example.com"
contracts:
  dir: "./contracts"
  pattern: "*.yaml"
defaults:
  format: human                  # or json
```

JSON Schema lives at `cli/schema/contractgate-config.schema.json`.
Emitted via hidden subcommand `contractgate config schema`.

### Subcommands

```
contractgate push [--dir <path>] [--dry-run] [--json]
  Walk contracts dir, parse YAML, POST /contracts (or
  POST /contracts/:id/versions if name already exists). Per-file result.
  Exit 0 on all success, 1 on any failure.

contractgate pull [--name <contract>] [--out <dir>] [--json]
  GET /contracts (list) or GET /contracts/:id (one). Write each as
  <name>.yaml under out dir. Idempotent.

contractgate validate [--dir <path>] [--json]
  Parse + compile each contract YAML locally. No network. Per-file
  pass/fail with line/column on parse errors.
```

Exit codes: 0 success, 1 user error (validation failed), 10–19 client
errors (network, auth, server).

### Reference GH Actions workflow

`.github/workflows/contractgate.yml`:

```yaml
name: contractgate
on:
  pull_request:
    paths: ['contracts/**']
  push:
    branches: [main]
    paths: ['contracts/**']
jobs:
  validate:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - name: Install contractgate
        run: |
          curl -sSL https://github.com/contractgate/contractgate/releases/latest/download/contractgate-linux-x86_64.tar.gz \
            | tar xz -C /usr/local/bin
      - run: contractgate validate
  publish:
    if: github.event_name == 'push' && github.ref == 'refs/heads/main'
    needs: validate
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - run: |
          curl -sSL https://github.com/contractgate/contractgate/releases/latest/download/contractgate-linux-x86_64.tar.gz \
            | tar xz -C /usr/local/bin
      - run: contractgate push
        env:
          CONTRACTGATE_API_KEY: ${{ secrets.CONTRACTGATE_API_KEY }}
```

This is what users copy into their own repos.

## Test plan

- `tests/cli_push_pull.rs` — spin gateway via `tokio::test` + `axum::serve`,
  push 3 contracts, pull them back, assert byte equivalence.
- `tests/cli_validate.rs` — fixture corpus of valid + invalid YAML, assert
  exit codes and `--json` output shapes.
- Snapshot tests on `--json` outputs (insta crate).
- Cross-compile smoke for the four release targets in CI before tagging.

## Rollout

1. ✅ Sign-off this RFC.
2. ✅ `src/bin/contractgate.rs` + `src/cli/` scaffold.
3. ✅ `push` (forces auth + error + output plumbing into existence).
4. ✅ `pull` (round-trip with `push`).
5. ✅ `validate` (local-only).
6. ✅ `.contractgate.yml` parser + walk-up.
7. ✅ `tests/cli_*.rs`.
8. ✅ Cross-compile job in `.github/workflows/ci.yml` (`cli-cross-compile` matrix job).
9. ✅ Reference workflow `.github/workflows/contractgate.yml`.
10. ⏳ `cargo check && cargo test` — cargo not available in sandbox; CI will verify.
11. ✅ Update `MAINTENANCE_LOG.md`.

## Deferred

- `diff` subcommand → RFC-015.
- Pre-commit hook script + installer.
- GitLab CI / Bitbucket Pipelines templates.
- GitOps reconciliation loop.
- Workspace split into `contractgate-core`.
- Homebrew / Scoop / Chocolatey distribution.
- Keychain / OAuth auth.
