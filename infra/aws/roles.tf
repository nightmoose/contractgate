# Workload roles for learning-plan projects P2–P5. Each one is created under
# path /nmb/ with nmb-boundary attached. No billable compute is declared in
# this stack: later projects (examples/aws-agentcore, examples/aws-rag,
# examples/aws-pipeline) pass these role ARNs in.
#
# Existing account objects that this stack does not manage:
# - lexsi-sagemaker-exec (read-only data source; already used by Lexsi/P6)
# - the $25 AWS Budget alert (budgets:* is denied on the deploy role)
# - IAM user nightmoose-bots and its policies (the deploy role cannot touch users)

data "aws_iam_policy" "boundary" {
  arn = local.boundary_arn
}

data "aws_iam_role" "lexsi_sagemaker_exec" {
  name = "lexsi-sagemaker-exec"
}

data "aws_iam_policy_document" "agentcore_runtime" {
  statement {
    sid       = "InvokeNova"
    effect    = "Allow"
    actions   = ["bedrock:InvokeModel", "bedrock:InvokeModelWithResponseStream"]
    resources = local.nova_model_arns
  }

  statement {
    sid     = "Logs"
    effect  = "Allow"
    actions = ["logs:CreateLogGroup", "logs:CreateLogStream", "logs:PutLogEvents", "logs:DescribeLogGroups", "logs:DescribeLogStreams"]
    resources = [
      "arn:aws:logs:${local.region}:${local.account_id}:log-group:/aws/bedrock-agentcore/${local.name_prefix}-*",
      "arn:aws:logs:${local.region}:${local.account_id}:log-group:/aws/bedrock-agentcore/${local.name_prefix}-*:*",
      "arn:aws:logs:${local.region}:${local.account_id}:log-group:/aws/amazonbedrockagentcore/*",
      "arn:aws:logs:${local.region}:${local.account_id}:log-group:/aws/amazonbedrockagentcore/*:*",
    ]
  }

  statement {
    sid       = "XRay"
    effect    = "Allow"
    actions   = ["xray:PutTraceSegments", "xray:PutTelemetryRecords", "xray:GetSamplingRules", "xray:GetSamplingTargets"]
    resources = ["*"]
  }

  statement {
    sid       = "CloudWatchMetrics"
    effect    = "Allow"
    actions   = ["cloudwatch:PutMetricData"]
    resources = ["*"]
    condition {
      test     = "StringEquals"
      variable = "cloudwatch:namespace"
      values   = ["NightMoose/AgentCore", "Bedrock/AgentCore"]
    }
  }

  statement {
    sid       = "EcrAuth"
    effect    = "Allow"
    actions   = ["ecr:GetAuthorizationToken"]
    resources = ["*"]
  }

  statement {
    sid       = "EcrPull"
    effect    = "Allow"
    actions   = ["ecr:BatchGetImage", "ecr:GetDownloadUrlForLayer", "ecr:BatchCheckLayerAvailability"]
    resources = ["arn:aws:ecr:${local.region}:${local.account_id}:repository/${local.name_prefix}-*"]
  }
}

data "aws_iam_policy_document" "agentcore_gateway" {
  statement {
    sid     = "InvokeNmbLambdas"
    effect  = "Allow"
    actions = ["lambda:InvokeFunction"]
    resources = [
      "arn:aws:lambda:${local.region}:${local.account_id}:function:${local.name_prefix}-*",
      "arn:aws:lambda:${local.region}:${local.account_id}:function:${local.name_prefix}-*:*",
    ]
  }

  statement {
    sid       = "ReadNmbSecrets"
    effect    = "Allow"
    actions   = ["secretsmanager:GetSecretValue", "secretsmanager:DescribeSecret"]
    resources = ["arn:aws:secretsmanager:${local.region}:${local.account_id}:secret:${local.name_prefix}-*"]
  }
}

data "aws_iam_policy_document" "bedrock_kb" {
  statement {
    sid       = "InvokeTitanEmbed"
    effect    = "Allow"
    actions   = ["bedrock:InvokeModel"]
    resources = local.titan_embed_arns
  }

  statement {
    sid       = "ListDocsBucket"
    effect    = "Allow"
    actions   = ["s3:ListBucket", "s3:GetBucketLocation"]
    resources = ["arn:aws:s3:::${local.name_prefix}-*"]
  }

  statement {
    sid       = "ReadDocsObjects"
    effect    = "Allow"
    actions   = ["s3:GetObject"]
    resources = ["arn:aws:s3:::${local.name_prefix}-*/*"]
  }

  statement {
    sid     = "S3VectorsIndex"
    effect  = "Allow"
    actions = ["s3vectors:*"]
    resources = [
      "arn:aws:s3vectors:${local.region}:${local.account_id}:bucket/${local.name_prefix}-*",
      "arn:aws:s3vectors:${local.region}:${local.account_id}:bucket/${local.name_prefix}-*/*",
    ]
  }
}

data "aws_iam_policy_document" "bedrock_eval" {
  statement {
    sid       = "EvalBucketList"
    effect    = "Allow"
    actions   = ["s3:ListBucket", "s3:GetBucketLocation"]
    resources = ["arn:aws:s3:::${local.name_prefix}-*"]
  }

  statement {
    sid       = "EvalBucketObjects"
    effect    = "Allow"
    actions   = ["s3:GetObject", "s3:PutObject", "s3:DeleteObject", "s3:AbortMultipartUpload"]
    resources = ["arn:aws:s3:::${local.name_prefix}-*/*"]
  }

  statement {
    sid     = "InvokeEvalModels"
    effect  = "Allow"
    actions = ["bedrock:InvokeModel", "bedrock:InvokeModelWithResponseStream", "bedrock:CreateEvaluationJob", "bedrock:GetEvaluationJob", "bedrock:ListEvaluationJobs", "bedrock:StopEvaluationJob"]
    resources = concat(local.nova_model_arns, [
      "arn:aws:bedrock:${local.region}:${local.account_id}:evaluation-job/*",
    ])
  }
}

data "aws_iam_policy_document" "lambda_validate" {
  statement {
    sid       = "InboxList"
    effect    = "Allow"
    actions   = ["s3:ListBucket", "s3:GetBucketLocation"]
    resources = ["arn:aws:s3:::${local.name_prefix}-*"]
  }

  statement {
    sid       = "InboxGet"
    effect    = "Allow"
    actions   = ["s3:GetObject"]
    resources = ["arn:aws:s3:::${local.name_prefix}-*/*"]
  }

  statement {
    sid       = "SqsSend"
    effect    = "Allow"
    actions   = ["sqs:SendMessage", "sqs:GetQueueUrl", "sqs:GetQueueAttributes"]
    resources = ["arn:aws:sqs:${local.region}:${local.account_id}:${local.name_prefix}-*"]
  }

  statement {
    sid     = "LedgerWrite"
    effect  = "Allow"
    actions = ["dynamodb:PutItem", "dynamodb:UpdateItem", "dynamodb:GetItem", "dynamodb:BatchWriteItem"]
    resources = [
      "arn:aws:dynamodb:${local.region}:${local.account_id}:table/${local.name_prefix}-*",
      "arn:aws:dynamodb:${local.region}:${local.account_id}:table/${local.name_prefix}-*/index/*",
    ]
  }
}

data "aws_iam_policy_document" "sfn_pipeline" {
  statement {
    sid     = "InvokeNmbLambdas"
    effect  = "Allow"
    actions = ["lambda:InvokeFunction"]
    resources = [
      "arn:aws:lambda:${local.region}:${local.account_id}:function:${local.name_prefix}-*",
      "arn:aws:lambda:${local.region}:${local.account_id}:function:${local.name_prefix}-*:*",
    ]
  }

  statement {
    sid       = "SqsSend"
    effect    = "Allow"
    actions   = ["sqs:SendMessage", "sqs:GetQueueUrl", "sqs:GetQueueAttributes"]
    resources = ["arn:aws:sqs:${local.region}:${local.account_id}:${local.name_prefix}-*"]
  }

  statement {
    sid       = "LedgerWrite"
    effect    = "Allow"
    actions   = ["dynamodb:PutItem", "dynamodb:UpdateItem", "dynamodb:GetItem"]
    resources = ["arn:aws:dynamodb:${local.region}:${local.account_id}:table/${local.name_prefix}-*"]
  }

  # Step Functions log delivery requires a mix of starred and scoped log APIs.
  statement {
    sid    = "SfnLogDelivery"
    effect = "Allow"
    actions = [
      "logs:CreateLogDelivery",
      "logs:GetLogDelivery",
      "logs:UpdateLogDelivery",
      "logs:DeleteLogDelivery",
      "logs:ListLogDeliveries",
      "logs:PutResourcePolicy",
      "logs:DescribeResourcePolicies",
      "logs:DescribeLogGroups",
    ]
    resources = ["*"]
  }

  statement {
    sid     = "SfnLogWrite"
    effect  = "Allow"
    actions = ["logs:CreateLogGroup", "logs:CreateLogStream", "logs:PutLogEvents", "logs:DescribeLogStreams"]
    resources = [
      "arn:aws:logs:${local.region}:${local.account_id}:log-group:/aws/vendedlogs/states/${local.name_prefix}-*",
      "arn:aws:logs:${local.region}:${local.account_id}:log-group:/aws/vendedlogs/states/${local.name_prefix}-*:*",
      "arn:aws:logs:${local.region}:${local.account_id}:log-group:/aws/states/${local.name_prefix}-*",
      "arn:aws:logs:${local.region}:${local.account_id}:log-group:/aws/states/${local.name_prefix}-*:*",
    ]
  }
}

data "aws_iam_policy_document" "events_to_sfn" {
  statement {
    sid       = "StartNmbPipeline"
    effect    = "Allow"
    actions   = ["states:StartExecution"]
    resources = ["arn:aws:states:${local.region}:${local.account_id}:stateMachine:${local.name_prefix}-*"]
  }
}

module "agentcore_runtime" {
  source = "./modules/iam-role"

  name                     = "nmb-agentcore-runtime"
  path                     = var.role_path
  description              = "P2 Bedrock AgentCore runtime (Nova invoke, logs, X-Ray, ECR pull)."
  permissions_boundary_arn = data.aws_iam_policy.boundary.arn
  trusted_services         = ["bedrock-agentcore.amazonaws.com"]
  inline_policies          = { runtime = data.aws_iam_policy_document.agentcore_runtime.json }
  tags = {
    "nmb:project"   = "P2"
    "nmb:component" = "agentcore-runtime"
  }
}

module "agentcore_gateway" {
  source = "./modules/iam-role"

  name                     = "nmb-agentcore-gateway"
  path                     = var.role_path
  description              = "P2 Bedrock AgentCore gateway (invoke nmb-* Lambdas, read nmb-* secrets)."
  permissions_boundary_arn = data.aws_iam_policy.boundary.arn
  trusted_services         = ["bedrock-agentcore.amazonaws.com"]
  inline_policies          = { gateway = data.aws_iam_policy_document.agentcore_gateway.json }
  tags = {
    "nmb:project"   = "P2"
    "nmb:component" = "agentcore-gateway"
  }
}

module "bedrock_kb" {
  source = "./modules/iam-role"

  name                     = "nmb-bedrock-kb"
  path                     = var.role_path
  description              = "P3 Bedrock Knowledge Base (Titan Embed v2, docs bucket, S3 Vectors)."
  permissions_boundary_arn = data.aws_iam_policy.boundary.arn
  trusted_services         = ["bedrock.amazonaws.com"]
  inline_policies          = { knowledgebase = data.aws_iam_policy_document.bedrock_kb.json }
  tags = {
    "nmb:project"   = "P3"
    "nmb:component" = "bedrock-kb"
  }
}

module "bedrock_eval" {
  source = "./modules/iam-role"

  name                     = "nmb-bedrock-eval"
  path                     = var.role_path
  description              = "P4 Bedrock evaluation jobs (Nova models, nmb-* eval bucket)."
  permissions_boundary_arn = data.aws_iam_policy.boundary.arn
  trusted_services         = ["bedrock.amazonaws.com"]
  inline_policies          = { evaluation = data.aws_iam_policy_document.bedrock_eval.json }
  tags = {
    "nmb:project"   = "P4"
    "nmb:component" = "bedrock-eval"
  }
}

module "lambda_validate" {
  source = "./modules/iam-role"

  name                     = "nmb-lambda-validate"
  path                     = var.role_path
  description              = "P5 Lambda validator (inbox S3 get, SQS send, DynamoDB ledger put)."
  permissions_boundary_arn = data.aws_iam_policy.boundary.arn
  trusted_services         = ["lambda.amazonaws.com"]
  inline_policies          = { validate = data.aws_iam_policy_document.lambda_validate.json }
  managed_policy_arns      = ["arn:aws:iam::aws:policy/service-role/AWSLambdaBasicExecutionRole"]
  tags = {
    "nmb:project"   = "P5"
    "nmb:component" = "lambda-validate"
  }
}

module "sfn_pipeline" {
  source = "./modules/iam-role"

  name                     = "nmb-sfn-pipeline"
  path                     = var.role_path
  description              = "P5 Step Functions pipeline (invoke nmb-* Lambdas, SQS, DynamoDB, logs)."
  permissions_boundary_arn = data.aws_iam_policy.boundary.arn
  trusted_services         = ["states.amazonaws.com"]
  inline_policies          = { pipeline = data.aws_iam_policy_document.sfn_pipeline.json }
  tags = {
    "nmb:project"   = "P5"
    "nmb:component" = "sfn-pipeline"
  }
}

module "events_to_sfn" {
  source = "./modules/iam-role"

  name                     = "nmb-events-to-sfn"
  path                     = var.role_path
  description              = "P5 EventBridge to Step Functions (StartExecution on nmb-* state machines)."
  permissions_boundary_arn = data.aws_iam_policy.boundary.arn
  trusted_services         = ["events.amazonaws.com"]
  inline_policies          = { start = data.aws_iam_policy_document.events_to_sfn.json }
  tags = {
    "nmb:project"   = "P5"
    "nmb:component" = "events-to-sfn"
  }
}
