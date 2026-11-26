# `cg diff`: Drift Check Against the Deployed Contract

**Added 2026-11-26 | Part of the CLI GitOps flow (RFC-014), implements the drift half of RFC-015. Not in the 0.4.2 release; it ships with the next one.**

Compare the contract YAML files in your repo with the latest stable version
deployed on the gateway. `cg diff` changes nothing, locally or on the gateway.
It exits non-zero when they differ, so a scheduled job or a pull request check
can fail on drift.

---

## Synopsis

```
cg diff [--dir <DIR>] [--name <NAME>] [--fail-on-remote-only] [--json]
```

It needs an API key (`CONTRACTGATE_API_KEY` or `--api-key`) and the gateway URL
from `.contractgate.yml`, exactly like `cg push` and `cg pull`.

## Flags

| Flag | Default | Description |
|---|---|---|
| `--dir <DIR>` | `contracts.dir` from `.contractgate.yml` | Directory of contract YAML files. Files matched by `contracts.pattern`. |
| `--name <NAME>` | all | Check only the contract with this name (the `name:` in the YAML, not the file name). |
| `--fail-on-remote-only` | off | Also count a deployed contract with no local file as drift. |
| `--json` | off | One JSON object on stdout instead of text. |

## What counts as drift

Each local file is compared with the latest stable version of the contract that
has the same `name`. The comparison is on the file text, ignoring line endings
and trailing whitespace. This is the same text `cg pull` writes and `cg push`
sends, so `cg pull` followed by `cg diff` is always clean.

| Status | Meaning | Counts as drift |
|---|---|---|
| `in_sync` | The file matches the deployed stable version. | No |
| `drifted` | The file differs. Field-level changes (deployed to local) are listed when there are any. A comment-only or metadata-only change shows as a text difference. | Yes |
| `not_deployed` | No contract with that name is deployed, or it has no stable version yet. | Yes |
| `remote_only` | A contract is deployed that has no local file. | Only with `--fail-on-remote-only` |
| `error` | The file could not be read or parsed. | No, but the exit code is 2 |

Only the latest **stable** version is compared. A draft or a deprecated version
on the gateway is not what the pipeline enforces, so it is not the baseline.

## Exit codes

| Code | Meaning |
|---|---|
| `0` | No drift. |
| `1` | Drift: at least one file is `drifted` or `not_deployed` (or `remote_only` with `--fail-on-remote-only`). |
| `2` | The check could not complete: nothing matched the directory, a file could not be read or parsed, the gateway could not be reached or rejected the key, or `--name` matched no local file. Error wins over drift. |

These match `cg test` and `cg synth` (0 clean, 1 findings, 2 harness problem).
As for every command that talks to the gateway, a missing API key exits `11`
and an unreadable `.contractgate.yml` exits `10`.

## Human output

```
OK     orders (1.0.0)
DRIFT  customers (1.2.0)  1 field-level change(s)
         required_changed  email  required → optional
NEW    refunds (local 1.0)  no contract with this name is deployed
REMOTE legacy_events  deployed, but no local file
1 in sync, 2 drifted or not deployed, 1 deployed only, 0 error(s) (exit 1)
```

## JSON output

```json
{
  "drift": true,
  "exit_code": 1,
  "results": [
    {
      "contract": "orders",
      "status": "drifted",
      "file": "contracts/orders.yaml",
      "deployed_version": "1.0.0",
      "local_version": "1.0.0",
      "changes": [
        {"kind": "required_changed", "field": "order_id", "detail": "required → optional"}
      ],
      "detail": "1 field-level change(s)"
    }
  ]
}
```

`changes` uses the same `kind` values as `POST /contracts/diff`.

## Use in CI

A scheduled drift check, failing the job when the repo and the gateway disagree:

```yaml
name: contract-drift
on:
  schedule:
    - cron: "0 12 * * 1-5"
jobs:
  drift:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - name: Install the CLI
        run: |
          # Needs a release that includes `diff` (the first one after 0.4.2).
          # Set the repository variable CONTRACTGATE_VERSION to it, without the leading v.
          VERSION="${CONTRACTGATE_VERSION:?set to a release that includes diff}"
          gh release download "v${VERSION}" -R nightmoose/contractgate \
            -p "contractgate-${VERSION}-x86_64-unknown-linux-gnu.tar.gz"
          tar xzf "contractgate-${VERSION}-x86_64-unknown-linux-gnu.tar.gz"
          sudo mv contractgate /usr/local/bin/
        env:
          GH_TOKEN: ${{ github.token }}
          CONTRACTGATE_VERSION: ${{ vars.CONTRACTGATE_VERSION }}
      - name: Check for drift
        env:
          CONTRACTGATE_API_KEY: ${{ secrets.CONTRACTGATE_API_KEY }}
        run: contractgate diff --json
```

Exit `1` fails the job (drift). Exit `2` also fails it; read the log to tell a
broken check from real drift.

## Notes

- Read-only. It never pushes, pulls or deploys. To fix drift, run `cg push`
  (repo is right) or `cg pull` (gateway is right).
- It does not say whether a change is breaking. For that, see
  [Impact analysis](impact-analysis-reference.md) and `POST /contracts/diff`.
- Contracts that share a name in one organization are matched on the first one
  the gateway lists.
