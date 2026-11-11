locals {
  # Cost-allocation tags. AWS Budgets are denied on nmb-terraform-deploy
  # (and on the nmb-boundary), so this stack does not create budget alerts.
  # The existing $25 alert stays manual. Use these tags to filter Cost
  # Explorer later. Keys with a colon must be quoted in HCL.
  cost_tags = {
    ManagedBy         = "terraform"
    Environment       = "sandbox"
    Owner             = "nightmoose"
    "nmb:stack"       = "foundation"
    "nmb:cost-alloc"  = "aws-learning-plan"
    "nmb:do-not-prod" = "true"
  }

  boundary_arn = var.permissions_boundary_arn
  account_id   = var.aws_account_id
  region       = var.aws_region

  # Resource naming prefixes for later project stacks (P2–P5). Roles are
  # granted only on names that start with nmb-.
  name_prefix = "nmb"

  nova_model_arns = [
    "arn:aws:bedrock:${local.region}::foundation-model/amazon.nova-*",
    "arn:aws:bedrock:${local.region}:${local.account_id}:inference-profile/us.amazon.nova-*",
    "arn:aws:bedrock:${local.region}:${local.account_id}:inference-profile/amazon.nova-*",
    "arn:aws:bedrock:${local.region}:${local.account_id}:application-inference-profile/*",
  ]

  titan_embed_arns = [
    "arn:aws:bedrock:${local.region}::foundation-model/amazon.titan-embed-text-v2:*",
    "arn:aws:bedrock:${local.region}:${local.account_id}:inference-profile/amazon.titan-embed-text-v2:*",
    "arn:aws:bedrock:${local.region}:${local.account_id}:inference-profile/us.amazon.titan-embed-text-v2:*",
  ]
}
