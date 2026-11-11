# Local backend on purpose. From the NightMoose bot box, the VPC endpoint
# policy denies s3:CreateBucket and s3:ListAllMyBuckets, so this stack cannot
# bootstrap an S3 remote state bucket from here. Object APIs against an
# already-created bucket were not used, because no state bucket exists yet.
#
# To migrate later from a network that can create the bucket (a laptop, or
# GitHub Actions OIDC): create a private versioned bucket, then
#   terraform init -migrate-state -backend-config=backend-s3.example.hcl
# See README.md.
terraform {
  backend "local" {
    path = "terraform.tfstate"
  }
}
