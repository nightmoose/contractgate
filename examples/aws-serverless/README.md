# Serverless contract-enforcement pipeline (P5)

Stripe-shaped charge records go through an event-driven pipeline on AWS:
EventBridge (or a direct StartExecution) starts a Step Functions workflow,
which invokes a validate Lambda. The Lambda runs the same in-process
`contractgate` check used by [`examples/aws-kinesis`](../aws-kinesis/)
(`stripe_charges_raw`). Passing batches are recorded in a DynamoDB ledger;
failing rows go to an SQS quarantine queue with a DLQ. A CloudWatch
dashboard and a reject-rate alarm (threshold 25%) watch the run.

```
drive.py / PutEvents
        │
        ▼
  nmb-pipeline event bus  ──►  nmb-contract-pipeline (Step Functions)
  or StartExecution                    │
                                       ▼
                              nmb-validate (Lambda)
                               contractgate, stripe_charges_raw
                                       │
                    ┌──────────────────┼──────────────────┐
                    ▼                  ▼                  ▼
         DynamoDB ledger      SQS nmb-quarantine     CloudWatch
         (per-batch audit)        └─ nmb-quarantine-dlq   dashboard
                                                          + reject-rate alarm
```

This example does not call the ContractGate staging or production API, and
it does not touch the Fly apps. The validator is self-contained in the
Lambda zip (`contractgate` ≥ 0.4.1, pure Python).

## Why there is no S3 inbox

The learning plan called for an S3 drop to trigger EventBridge. From the
NightMoose bot box, the VPC endpoint policy denies `s3:CreateBucket` and
`s3:ListAllMyBuckets`, so this stack cannot create (or even list) an inbox
bucket here. The `nmb-lambda-validate` role is also GetObject-only; it
cannot write a passing prefix.

Drive the pipeline with something that works from this network:

- `python scripts/drive.py smoke` — 20 mixed records via
  `states:StartExecution`, then a 5-record EventBridge `PutEvents` on the
  `nmb-pipeline` bus (`source=nmb.contractgate`, `detail-type=BatchReady`).
- `python scripts/drive.py run --records 100000` — replay of the same
  synthetic Stripe-charge mix used in the Kinesis 100k run, 200 records
  per execution.
- `python scripts/drive.py poison` — 10 deliberately invalid rows, to put
  a high datapoint on the reject-rate alarm.
- Direct `lambda:InvokeFunction` with `{"batch_id": "...", "records": [...]}`.

The EventBridge rule is already wired so an S3 Object Created event could
be added later from a network that can create the bucket. Until then, do
not apply an inbox bucket from this box: a CreateBucket failure would
block the rest of the stack.

## Roles

The stack reuses the `/nmb/` roles from the Terraform foundation
([PR #410](https://github.com/nightmoose/contractgate/pull/410)):

| Role | Trust | Use |
|---|---|---|
| `nmb-lambda-validate` | `lambda.amazonaws.com` | Validate Lambda (S3 get, SQS send, DynamoDB put, basic logs) |
| `nmb-sfn-pipeline` | `states.amazonaws.com` | State machine (invoke `nmb-*` Lambdas, SQS, DynamoDB, logs) |
| `nmb-events-to-sfn` | `events.amazonaws.com` | EventBridge target (`states:StartExecution` on `nmb-*`) |

No new IAM roles are created. Apply through
`terraform/with-deploy-role.sh` so the `nightmoose-bots` environment keys
are not overwritten.

## Apply, run, destroy

Account `065614567889`, region `us-east-1`. IAM roles have no hourly cost;
Lambda, Step Functions standard, SQS, DynamoDB on-demand, EventBridge, and
the CloudWatch dashboard (within the first three free dashboards) are
request-priced. Tear the stack down the same day with `make destroy`.

```bash
export PATH=$HOME/.local/bin:$PATH AWS_DEFAULT_REGION=us-east-1
cd examples/aws-serverless
python -m venv .venv && . .venv/bin/activate
pip install -r requirements.txt
make test
make apply          # packages the zip, then terraform apply as nmb-terraform-deploy
python scripts/drive.py smoke
python scripts/drive.py run --records 100000
python scripts/drive.py poison
python scripts/drive.py status
make destroy        # removes this stack only, not the /nmb/ roles
```

`make plan` / `make apply` / `make destroy` run in `terraform/` through
`with-deploy-role.sh`. State is local (`terraform.tfstate`, gitignored)
for the same S3 reason as the foundation stack.

## Cost tags

Every tagged resource gets the foundation defaults, with a P5 stack name:

- `ManagedBy=terraform`
- `Environment=sandbox`
- `Owner=nightmoose`
- `nmb:stack=serverless-pipeline`
- `nmb:cost-alloc=aws-learning-plan`
- `nmb:do-not-prod=true`
- `nmb:project=P5`

Components also have `nmb:component` (lambda-validate, sfn-pipeline,
quarantine, ledger, and so on).

## Known gap (same as the Kinesis demo)

The Python `contractgate` validator (0.4.1) does not enforce `format: email`.
Records with `receipt_email: "billing at example dot com"` pass ingest here
the same way they did in the Kinesis run. Use the hosted gateway or the
Rust engine when you depend on `format`.

## Measured run (us-east-1, 2026-11-11)

Driven from the bot box. No S3. The 100k replay uses the same generator and
seed (42) as the Kinesis demo, so the ingest counts match that run.

| Step | Records | Path | Result |
|---|---|---|---|
| [Smoke](runs/20261111T113037-100000recs.json) | 20 + 5 | StartExecution, then PutEvents on `nmb-pipeline` | 19/20 passed (1 `txn_` id); EventBridge 5/5 passed, ledger item appeared within a few seconds |
| 100k replay | 100,000 in 500 executions of 200 | StartExecution, concurrency 10 | 94,982 passed, 5,018 failed (5.018%). All 500 executions SUCCEEDED. Wall clock 10.81 s (~9,248 records/s). Validate Lambda p50 on the smoke batch was 17.6 µs per record (in-process `contractgate`) |
| Poison | 10 all-bad rows | StartExecution | 0 passed, 10 failed. After settle, quarantine had 5,029 visible messages (1 smoke + 5,018 + 10), DLQ 0 |

What the 100k run caught at ingest (`stripe_charges_raw`):

| Why | Held back |
|---|---|
| negative `amount` | 1,038 |
| missing `created` | 1,030 |
| `txn_` id prefix | 989 |
| upper-case `currency` | 984 |
| `status: complete` | 977 |
| `receipt_email` not a real email | 0 (967 produced; Python validator does not check `format`, same gap as the Kinesis demo) |

3,085 guest checkouts (no `customer`) passed ingest, as the contract allows. They would fail a later `stripe_charges_staged` check; this pipeline only enforces the raw ingest contract.

The reject-rate alarm (`nmb-pipeline-reject-rate`, threshold 25%) went to `ALARM` at 6:32:36 AM ET on a 100% reject-rate datapoint from the poison batch (period starting 6:31:00 AM ET). The 100k window at 6:30 AM ET was 5.018% and did not trip it. Metric-filter metrics took about a minute to appear, so a check immediately after the poison batch still saw `OK` (missing treated as not-breaching).
