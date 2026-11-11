# Example S3 backend once a state bucket exists outside the box VPC.
# Do not apply this from the box until CreateBucket (or an already-created
# bucket plus PutObject/GetObject) works from that network.
#
# Suggested bucket hardening: Block Public Access, BucketOwnerEnforced,
# AES256 or KMS encryption, versioning, HTTPS-only bucket policy, and
# native lockfile (Terraform 1.10+), no DynamoDB lock table.
bucket       = "nmb-tfstate-065614567889"
key          = "foundation/terraform.tfstate"
region       = "us-east-1"
encrypt      = true
use_lockfile = true
