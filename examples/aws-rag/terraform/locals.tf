locals {
  cost_tags = {
    ManagedBy         = "terraform"
    Environment       = "sandbox"
    Owner             = "nightmoose"
    "nmb:stack"       = "rag"
    "nmb:project"     = "P3"
    "nmb:cost-alloc"  = "aws-learning-plan"
    "nmb:do-not-prod" = "true"
  }
}
