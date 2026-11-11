# Local backend on purpose. From the NightMoose bot box, the VPC endpoint
# policy denies s3:CreateBucket and s3:ListAllMyBuckets, so this stack cannot
# bootstrap an S3 remote state bucket from here. Same constraint as the
# foundation stack in infra/aws. The state file is gitignored.
#
# To migrate later from a network that can create the bucket (a laptop, or
# GitHub Actions OIDC): create a private versioned bucket, then
#   terraform init -migrate-state -backend-config=backend-s3.example.hcl
terraform {
  backend "local" {
    path = "terraform.tfstate"
  }
}
