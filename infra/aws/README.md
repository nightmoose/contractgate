# NightMoose AWS foundation (Terraform)

Project 1 of the AWS learning plan in `nmb-chief/aws/LEARNING-PLAN.md`. This
stack defines the `/nmb/` IAM roles for projects 2–5 and the shared modules
later stacks reuse. It does not create billable compute, does not touch
ContractGate production or the Fly apps, and must not change application
runtime config before the Show HN on Thu Nov 12.

Account `065614567889`, region `us-east-1`. Apply only after assuming
`arn:aws:iam::065614567889:role/nmb-terraform-deploy`.

## What this stack creates

| Role | Path | Trust | Project |
|---|---|---|---|
| `nmb-agentcore-runtime` | `/nmb/` | `bedrock-agentcore.amazonaws.com` | P2 |
| `nmb-agentcore-gateway` | `/nmb/` | `bedrock-agentcore.amazonaws.com` | P2 |
| `nmb-bedrock-kb` | `/nmb/` | `bedrock.amazonaws.com` | P3 |
| `nmb-bedrock-eval` | `/nmb/` | `bedrock.amazonaws.com` | P4 |
| `nmb-lambda-validate` | `/nmb/` | `lambda.amazonaws.com` | P5 |
| `nmb-sfn-pipeline` | `/nmb/` | `states.amazonaws.com` | P5 |
| `nmb-events-to-sfn` | `/nmb/` | `events.amazonaws.com` | P5 |

Every role has the `nmb-boundary` permissions boundary. The `iam-role`
module refuses to plan if the path or boundary is wrong.

The stack also reads (and does not manage) the existing
`lexsi-sagemaker-exec` role, which P6 already uses.

Reusable modules under `modules/` — `iam-role`, `lambda`, `bedrock-kb`,
`ecs-service` — are not instantiated here except `iam-role`. Later
projects should call them from their own state and ship a `make destroy`.

## Backend choice: local, not S3

The plan called for S3 remote state with native locking. From the bot box,
the VPC endpoint policy denies `s3:CreateBucket` and `s3:ListAllMyBuckets`
(`AccessDenied` / "no VPC endpoint policy allows …"). A HeadBucket on a
non-existent name did reach S3 (HTTP 404), so the API is not fully
unreachable, but this stack cannot bootstrap a state bucket from the box.

This directory therefore uses a **local backend** (`terraform.tfstate`).
The state file is gitignored and must not be committed. CI plan-only
workflows start from empty local state, so after the roles exist they
will still plan as "7 to add". That is expected, not a signal to apply
from Actions.

To migrate later, from a laptop or from GitHub Actions (neither path uses
the box VPC):

1. Create a private bucket (suggested name `nmb-tfstate-065614567889`)
   with Block Public Access, BucketOwnerEnforced, AES256, versioning, and
   an HTTPS-only bucket policy.
2. Copy this box's `terraform.tfstate` off the box if the apply already
   happened here.
3. Run `terraform init -migrate-state -backend-config=backend-s3.example.hcl`.

## How to apply (roles only)

On the box, `nightmoose-bots` keys live in `AWS_ACCESS_KEY_ID` /
`AWS_SECRET_ACCESS_KEY`. The Terraform AWS provider prefers those env
vars over `AWS_PROFILE`, so a profile alone would still run as the bot
user (which cannot create IAM roles). Use `with-deploy-role.sh`, which
assumes `nmb-terraform-deploy` in a subshell and does not overwrite the
caller's env:

```bash
export PATH=$HOME/.local/bin:$PATH AWS_DEFAULT_REGION=us-east-1
cd infra/aws
make fmt
make init
make validate
make plan
make apply
```

Or: `./with-deploy-role.sh terraform plan`. CI does not use the wrapper;
GitHub Actions OIDC already assumes the same role into the job env.

IAM roles have no hourly cost. Do not apply the unused `lambda`,
`bedrock-kb`, or `ecs-service` modules from this stack.

## Cost tags

`nmb-terraform-deploy` and `nmb-boundary` deny `budgets:*` and `ce:*`, so
Terraform cannot create the $100 / $250 budget alerts the plan asked for
once P1 landed. The existing $25 alert stays manual in the console. This
stack instead applies these default tags to every tagged resource:

- `ManagedBy=terraform`
- `Environment=sandbox`
- `Owner=nightmoose`
- `nmb:stack=foundation`
- `nmb:cost-alloc=aws-learning-plan`
- `nmb:do-not-prod=true`

Each role also has `nmb:project` (`P2`–`P5`) and `nmb:component`.

## CI

`.github/workflows/terraform-aws.yml` runs on pull requests that touch
`infra/aws/**` (and on pushes to `staging`). It uses GitHub OIDC to assume
`nmb-terraform-deploy` — no stored AWS keys — and runs `terraform fmt
-check`, `validate`, and `plan`. There is no apply job.

## Destroy

`make destroy` in this directory removes only the P2–P5 roles. Each later
project stack should have its own `make destroy` and should run it the
same day it applies anything with an hourly cost.
