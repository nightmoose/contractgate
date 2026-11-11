# Local backend on purpose. This is a separate state from infra/aws/
# (the P1 foundation stack). Do not point this root at that state file.
# From the bot box, S3 CreateBucket is denied by the VPC endpoint policy,
# so this stack cannot use an S3 backend either.
terraform {
  backend "local" {
    path = "terraform.tfstate"
  }
}
