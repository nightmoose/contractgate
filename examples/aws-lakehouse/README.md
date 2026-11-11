# Lakehouse data contracts on S3 + Iceberg + Glue + Athena (P8)

Write-time ContractGate checks on Stripe-shaped charges, then two Iceberg
tables in the Glue Data Catalog, queried with Athena.

```
generate charges  ──►  contractgate (stripe_charges_raw, in-process)
                            │
              ┌─────────────┴──────────────┐
              ▼                            ▼
     charges_raw (Iceberg)      charges_quarantine (Iceberg)
              │                            │
              └──────── Athena SQL ────────┘
                 counts, leak checks, drift
```

The validator is the same in-process `contractgate` package and
`stripe_charges_raw` contract used in the Kinesis and serverless pipeline
demos. It does not call the staging or production ContractGate API, and it
does not touch Fly or production.

## Why this runs in GitHub Actions on this repo

From the NightMoose bot box, the VPC endpoint policy denies
`s3:CreateBucket` and `s3:ListAllMyBuckets` (confirmed as both
`nightmoose-bots` and `nmb-terraform-deploy`). The first attempt used
GitHub Actions OIDC from private repo `nightmoose/schemasmith` (PR #9).
That assume failed: schemasmith has Actions OIDC subject customization
`use_immutable_subject: true`, so the token `sub` is prefixed
`repo:nightmoose@91439862/schemasmith@1451682729` and does not match the
IAM trust `StringLike` value `repo:nightmoose/schemasmith:*`. Contractgate
uses the default subject (`use_immutable_subject: false`, prefix
`repo:nightmoose/contractgate`), and `nmb-terraform-deploy` already works
from this repo (for example Terraform AWS plan run 43013009689). This
example therefore runs here.

The workflow is branch-scoped: it runs only on push to
`examples/aws-lakehouse` (and `workflow_dispatch`). It never runs on
`main`. The same job tears the stack down in an `always()` step.

## What it creates (and deletes)

| Resource | Name |
|---|---|
| S3 bucket (private, AES256, versioned, HTTPS-only, Block Public Access) | `nmb-lakehouse-065614567889` |
| Athena workgroup (engine v3, 1 GiB scan cap) | `nmb-lakehouse` |
| Glue/Athena database | `nmb_lakehouse` |
| Iceberg tables | `charges_raw`, `charges_quarantine` |

Tags: `nmb:project=P8`, `nmb:stack=lakehouse`, `nmb:cost-alloc=aws-learning-plan`,
`nmb:do-not-prod=true`, `Environment=sandbox`, `Owner=nightmoose`,
`ManagedBy=github-actions`. Region `us-east-1`.

Glue Data Quality (plan item R12 `nmb-glue`) is not used: that role was
not created in P1, and the Athena leak/drift queries cover the same
checks without a crawler or DQ DPU-hours.

## Local check (no AWS)

```bash
pip install -r examples/aws-lakehouse/requirements.txt
python examples/aws-lakehouse/scripts/local_check.py
```

The apply / query / destroy path is
`.github/workflows/aws-p8-lakehouse.yml`.
