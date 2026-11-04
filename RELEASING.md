# Releasing

DataContractGate and AISchemaGate ship from `main`. `staging` is where a batch of work is verified before that promotion.

When development reaches a stopping point, verify `staging` before opening the promotion:

- Staging CI is green, including the SDK jobs.
- A staging smoke test passed.
- Pending production migrations are listed.

The maintainer then opens one pull request from `staging` into `main`, and merges it with a merge commit. The pull request body lists the promoted pull requests, the migrations, and the risks.

Approve and apply production migrations before or with the deploy. After the deploy, `GET /ready` must return 200.

`main` must match what is shipped. Do not let `main` drift from the release that is in production.

## Staging and main run the same checks

Staging exists to prove main will be fine, so a staging-to-main pull request should never fail a check. Every check that runs on a pull request into `main` also runs on pull requests into `staging` and on pushes to `staging`. That includes CodeQL, which runs from `.github/workflows/codeql.yml` rather than GitHub's default setup, because default setup only scans the default branch.

Before opening a release pull request:

- [ ] Diff the checks between `main` and `staging`. Compare the latest check runs on a pull request into each branch, plus any workflow with a `branches:` filter, and close any gap first.
