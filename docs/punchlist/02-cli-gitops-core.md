# Chunk 2 — CLI + GitOps Core

**Theme:** Ship the `contractgate` binary and the GitHub-native workflow around it.
**Why now:** CLI surface gates SDKs, CI templates, and the GitOps reconciliation loop. Build it once, well.

## Items

- [ ] `contractgate` CLI binary `[L]` — subcommands `push`, `pull`, `validate`, plus auth/config plumbing.
- [ ] `.contractgate.yml` config spec + JSON schema `[S]` — repo-level settings, env mapping, default contract dir.
- [ ] `contractgate validate` pre-commit hook + install script `[S]` — wraps the binary, picks up `.contractgate.yml`.
- [ ] GitHub Actions workflow `[M]` — validate on PR, publish on merge. Extends existing GitHub connector.

## Deferred to Chunk 3

- `contractgate diff` with breaking-change detection — slots in once base CLI exists.

## Deferred to later

- GitLab CI / Bitbucket Pipelines templates (do them after GH Actions proves the pattern).
- GitOps reconciliation loop `[L]` — schedule/webhook sync from a Git repo. Build after CLI stable.

## Open questions for the conversation

1. Language for CLI: Rust (share contract types crate, single static binary) or Go (tradition, easy cross-compile)? Rust likely wins given existing crate.
2. Auth: API token via env var only, or also OS keychain? OAuth deferred.
3. Output format: human + `--json` mode, or only one? CI consumers want JSON.
4. Distribution: Homebrew tap, scoop, GitHub Releases, `cargo install`? Decide before naming binary.
5. RFC required.

## Suggested first step

`docs/rfcs/00X-cli-surface.md` — lock subcommand grammar, config schema, auth model. Then scaffold the binary with `push`/`pull` only; add `validate` once shape proves out.
