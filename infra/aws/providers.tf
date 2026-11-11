provider "aws" {
  region = var.aws_region

  default_tags {
    tags = local.cost_tags
  }

  # Do not hard-code an AWS profile here. On the box, run with
  # AWS_PROFILE=nmb-terraform (credential_source=Environment, role
  # nmb-terraform-deploy). In GitHub Actions, OIDC sets env credentials
  # for the same role. A profile in this file would break CI.
}
