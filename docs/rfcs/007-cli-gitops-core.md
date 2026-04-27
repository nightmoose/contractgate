# RFC-007: CLI + GitOps Core

| Status        | Draft — awaiting sign-off                                              |
|---------------|------------------------------------------------------------------------|
| Author        | ContractGate team                                                      |
| Created       | 2026-04-27                                                             |
| Target branch | `nightly-maintenance-YYYY-MM-DD` (set when work starts)                |
| Chunk         | Punchlist 02 — CLI + GitOps Core                                       |
| Depends on    | RFC-005 (Python SDK) — pattern reference for HTTP client surface       |

## Summary

Ship the `contractgate` CLI binary plus the GitHub-native workflow around it.
Subcommands `push`, `pull`, `validate` cover the publish/fetch/lint loop.
A `.contractgate.yml` config file captures repo-level settings and points the
CLI at the right gateway and contract directory. A pre-commit hook wraps
`validate` for local enforcement. A reusable GitHub Actions workflow validates
contracts on PR and publishes on merge.

The CLI is the gateway's first non-Python language client and the wire-shape
freeze point for every future SDK (Chunk 6) and CI template (GitLab,
Bitbucket — Chunk 2 follow-ups). Build it once, well.

## Goals

1. Single static binary, cross-platform (Linux x86_64/arm64, macOS x86_64/arm64,
   Windows x86_64). One `cargo install` or `brew install` away.
2. Three subcommands in v0.1: `push`, `pull`, `validate`. Auth via env var.
3. `.contractgate.yml` with a JSON Schema checked into the repo. Discoverable
   via cosmiconfig-style walk-up from cwd.
4. Pre-commit hook script (`scripts/pre-commit`) installable via
   `contractgate hook install`. No external tooling required.
5. Reusable GitHub Actions workflow (`.github/workflows/contractgate.yml`)
   that other repos consume via `uses: contractgate/action@v1`.
6. Both human-readable and `--json` output modes.

## Non-goals

- `diff` subcommand — deferred to RFC-008 (it depends on the breaking-change
  taxonomy, not on basic CLI plumbing).
- GitLab CI / Bitbucket Pipelines templates — pattern follows after GitHub
  Actions proves out.
- GitOps reconciliation loop (Git repo → gateway sync) — separate RFC.
- OAuth, OS keychain integration — env var token only in v0.1.
- Plugin system / custom subcommands.

## Decisions (recommended — flag any to override)

| #  | Question | Recommendation |
|----|----------|----------------|
| Q1 | Implementation language | **Rust.** Reuse `crate::contract` for parse + compile; one static binary; matches existing toolchain. |
| Q2 | Crate location | **`cli/` workspace member.** Keeps gateway and CLI buildable independently; shares `contractgate-core` crate split out of `src/lib.rs`. |
| Q3 | Auth model | **`CONTRACTGATE_API_KEY` env var** + `--api-key` flag override. No keychain in v0.1. |
| Q4 | Output format | **Human by default; `--json` flag** flips every command to a stable JSON shape. CI consumers use `--json`. |
| Q5 | Distribution | **GitHub Releases (cross-compiled binaries) + Homebrew tap + `cargo install contractgate-cli`.** Scoop/Chocolatey deferred. |
| Q6 | Config discovery | **Walk up from cwd looking for `.contractgate.yml`**, stop at git root or filesystem root. Explicit `--config` flag overrides. |
| Q7 | Pre-commit hook style | **Plain shell script that calls `contractgate validate --staged`.** No `pre-commit` framework dep. |
| Q8 | GH Actions hosting | **Marketplace action `contractgate/action`** (separate repo) wrapping the binary. Pinned to a release tag. |

## Current state

- No CLI exists.
- `src/lib.rs` re-exports `Contract`, `compile`, `validate`. CLI can depend on
  the gateway crate directly via path; later split into `contractgate-core`
  if compile-time bloat warrants.
- Python SDK (`sdks/python/`, RFC-005) is the only first-party non-dashboard
  client today and a useful shape reference.
- Existing `x-api-key` auth on protected routes (`/contracts`, `/ingest`,
  `/audit`) — CLI hits these.

## Design

### Workspace layout

```
contractgate/
├── Cargo.toml                  # workspace
├── src/                        # gateway (existing)
├── cli/
│   ├── Cargo.toml              # binary crate `contractgate-cli`, bin name `contractgate`
│   ├── src/
│   │   ├── main.rs             # clap entry
│   │   ├── commands/
│   │   │   ├── mod.rs
│   │   │   ├── push.rs
│   │   │   ├── pull.rs
│   │   │   ├── validate.rs
│   │   │   └── hook.rs         # `hook install`, `hook uninstall`
│   │   ├── config.rs           # `.contractgate.yml` parse + walk-up discovery
│   │   ├── client.rs           # thin reqwest wrapper over the REST API
│   │   ├── output.rs           # human / json render switch
│   │   └── error.rs
│   └── tests/
│       ├── push_pull.rs        # integration: spin gateway, round-trip
│       └── validate.rs
├── scripts/
│   └── pre-commit              # shell hook installed by `hook install`
└── .github/workflows/
    └── contractgate.yml        # reference workflow for consumer repos
```

### `.contractgate.yml` schema

```yaml
# .contractgate.yml — checked into the consuming repo
version: "1.0"
gateway:
  url: "https://gw.example.com"
  # api_key NOT stored here — read from CONTRACTGATE_API_KEY env
contracts:
  dir: "./contracts"            # where .yaml contract files live
  pattern: "*.yaml"             # glob for `validate`, `push`
environments:                   # optional — maps branch to env
  main: production
  staging: staging
  "*": development
defaults:
  format: human                 # or json
```

JSON Schema lives at `cli/schema/contractgate-config.schema.json` and is
emitted via `contractgate config schema` (hidden subcommand).

### Subcommand surfaces

```
contractgate push [--dir <path>] [--dry-run] [--json]
  Walk contracts dir, parse each YAML, POST /contracts (or
  POST /contracts/:id/versions if name already exists). Per-file result.
  Exit 0 on all success, 1 on any failure.

contractgate pull [--name <contract>] [--out <dir>] [--json]
  GET /contracts (list) or GET /contracts/:id for one. Write each as
  <name>.yaml under out dir. Idempotent.

contractgate validate [--dir <path>] [--staged] [--json]
  Parse + compile each contract YAML locally. No network. Emit per-file
  pass/fail with line/column on parse errors. `--staged` reads paths from
  `git diff --cached --name-only` filtered by config glob.

contractgate hook install | uninstall
  Copy / remove scripts/pre-commit into .git/hooks/pre-commit.
```

### HTTP client (`cli/src/client.rs`)

- `reqwest::blocking::Client` with `User-Agent: contractgate-cli/0.1`.
- `x-api-key` header sourced once at startup from
  `CONTRACTGATE_API_KEY` (or `--api-key`).
- Strongly typed responses mirroring server: `ContractSummary`,
  `ContractDetail`, `IngestResult` — derive `serde::Deserialize`, share
  with gateway crate where possible.
- Errors: `ClientError { Network, Auth, NotFound, Server { status, body } }`
  → mapped to non-zero exit codes (10–19 reserved for client errors).

### Pre-commit hook script

```sh
#!/usr/bin/env sh
# .git/hooks/pre-commit — installed by `contractgate hook install`
set -e
exec contractgate validate --staged
```

If `contractgate` isn't on PATH the hook prints a one-line install hint and
exits non-zero.

### GitHub Actions workflow (reference)

```yaml
# .github/workflows/contractgate.yml
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
      - uses: contractgate/action@v1
        with:
          command: validate
  publish:
    if: github.event_name == 'push' && github.ref == 'refs/heads/main'
    needs: validate
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: contractgate/action@v1
        with:
          command: push
        env:
          CONTRACTGATE_API_KEY: ${{ secrets.CONTRACTGATE_API_KEY }}
```

The `contractgate/action` repo is created as part of this RFC's rollout.
Action is a thin shim that downloads the matching CLI release and runs the
requested subcommand.

## Test plan

- `cli/tests/push_pull.rs` — spin the gateway via `tokio::test` + `axum::serve`
  on an ephemeral port, push 3 contracts, pull them back, assert byte
  equivalence (modulo trailing newline).
- `cli/tests/validate.rs` — fixture corpus of valid + invalid YAML, assert
  exit codes and human/json output shapes.
- Snapshot tests on `--json` outputs (insta crate) so wire shape changes
  fail noisily in CI.
- Manual cross-compile smoke via `cross build --target ...` for each
  release target before tagging.

## Rollout

1. Sign-off this RFC.
2. Workspace split: extract `contractgate-core` crate (shared `Contract`,
   `compile`, `validate` types), keep gateway depending on it.
3. `cli/` crate scaffold + `push` + `pull` (network round-trip first; that
   forces all the auth/error/output plumbing into existence).
4. `validate` (local-only, no network).
5. `.contractgate.yml` parser + walk-up discovery.
6. `hook install`/`hook uninstall`.
7. Cross-compile config (cargo-dist or hand-rolled GitHub Actions release
   workflow).
8. `contractgate/action` repo + Marketplace listing.
9. Reference `.github/workflows/contractgate.yml` checked into this repo
   as both example and dogfood.
10. `cargo check && cargo test` (workspace-wide).
11. Update `MAINTENANCE_LOG.md`.

## Deferred

- `diff` subcommand → RFC-008.
- GitLab CI + Bitbucket Pipelines templates → follow-up after GH Actions
  proves out.
- GitOps reconciliation loop → separate RFC.
- Keychain auth, OAuth → revisit when usage pressure exists.
