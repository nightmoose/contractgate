# Releasing

DataContractGate and AISchemaGate ship from `main`. `staging` is where a batch of work is verified before that promotion.

When development reaches a stopping point, verify `staging` before opening the promotion:

- Staging CI is green, including the SDK jobs.
- A staging smoke test passed.
- Pending production migrations are listed.

The maintainer then opens one pull request from `staging` into `main`, and merges it with a merge commit. The pull request body lists the promoted pull requests, the migrations, and the risks.

Approve and apply production migrations before or with the deploy. After the deploy, `GET /ready` must return 200.

`main` must match what is shipped. Do not let `main` drift from the release that is in production.
