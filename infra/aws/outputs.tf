output "permissions_boundary_arn" {
  description = "Boundary attached to every role in this stack."
  value       = data.aws_iam_policy.boundary.arn
}

output "lexsi_sagemaker_exec_arn" {
  description = "Existing SageMaker execution role (not managed by this stack; used by P6)."
  value       = data.aws_iam_role.lexsi_sagemaker_exec.arn
}

output "role_arns" {
  description = "P2–P5 workload role ARNs created under path /nmb/."
  value = {
    nmb-agentcore-runtime = module.agentcore_runtime.arn
    nmb-agentcore-gateway = module.agentcore_gateway.arn
    nmb-bedrock-kb        = module.bedrock_kb.arn
    nmb-bedrock-eval      = module.bedrock_eval.arn
    nmb-lambda-validate   = module.lambda_validate.arn
    nmb-sfn-pipeline      = module.sfn_pipeline.arn
    nmb-events-to-sfn     = module.events_to_sfn.arn
  }
}

output "cost_tags" {
  description = "Default cost-allocation tags applied to tagged resources."
  value       = local.cost_tags
}
