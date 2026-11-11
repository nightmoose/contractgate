locals {
  cost_tags = {
    ManagedBy         = "terraform"
    Environment       = "sandbox"
    Owner             = "nightmoose"
    "nmb:stack"       = "serverless-pipeline"
    "nmb:cost-alloc"  = "aws-learning-plan"
    "nmb:do-not-prod" = "true"
    "nmb:project"     = "P5"
  }

  account_id = var.aws_account_id
  region     = var.aws_region
  prefix     = var.name_prefix

  function_name      = "${local.prefix}-validate"
  state_machine_name = "${local.prefix}-contract-pipeline"
  ledger_table       = "${local.prefix}-pipeline-ledger"
  quarantine_queue   = "${local.prefix}-quarantine"
  event_bus_name     = "${local.prefix}-pipeline"
  dashboard_name     = "${local.prefix}-pipeline"
}
