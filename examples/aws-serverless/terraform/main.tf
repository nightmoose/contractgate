# P5 serverless contract-enforcement pipeline.
# Uses the /nmb/ roles created by the foundation stack (PR #410). This
# stack does not create IAM roles, does not touch ContractGate prod or
# Fly, and does not create an S3 inbox bucket: the box VPC endpoint
# policy denies s3:CreateBucket. Drive the pipeline with Step Functions
# StartExecution or EventBridge PutEvents (see scripts/drive.py).

data "aws_iam_role" "lambda_validate" {
  name = "nmb-lambda-validate"
}

data "aws_iam_role" "sfn_pipeline" {
  name = "nmb-sfn-pipeline"
}

data "aws_iam_role" "events_to_sfn" {
  name = "nmb-events-to-sfn"
}

# --- Quarantine queue + DLQ -------------------------------------------------

resource "aws_sqs_queue" "dlq" {
  name                      = "${local.quarantine_queue}-dlq"
  sqs_managed_sse_enabled   = true
  message_retention_seconds = 1209600
  tags                      = { "nmb:component" = "quarantine-dlq" }
}

resource "aws_sqs_queue" "quarantine" {
  name                       = local.quarantine_queue
  sqs_managed_sse_enabled    = true
  visibility_timeout_seconds = 60
  message_retention_seconds  = 345600
  redrive_policy = jsonencode({
    deadLetterTargetArn = aws_sqs_queue.dlq.arn
    maxReceiveCount     = 3
  })
  tags = { "nmb:component" = "quarantine" }
}

# --- Per-batch audit ledger -------------------------------------------------

resource "aws_dynamodb_table" "ledger" {
  name         = local.ledger_table
  billing_mode = "PAY_PER_REQUEST"
  hash_key     = "batch_id"

  attribute {
    name = "batch_id"
    type = "S"
  }

  tags = { "nmb:component" = "ledger" }
}

# --- Validate Lambda --------------------------------------------------------

resource "aws_cloudwatch_log_group" "lambda" {
  name              = "/aws/lambda/${local.function_name}"
  retention_in_days = 14
  tags              = { "nmb:component" = "lambda-validate" }
}

resource "aws_lambda_function" "validate" {
  function_name                  = local.function_name
  role                           = data.aws_iam_role.lambda_validate.arn
  filename                       = var.lambda_zip
  source_code_hash               = filebase64sha256(var.lambda_zip)
  handler                        = "handler.handler"
  runtime                        = "python3.13"
  timeout                        = var.lambda_timeout
  memory_size                    = var.lambda_memory
  reserved_concurrent_executions = 20
  depends_on                     = [aws_cloudwatch_log_group.lambda]

  environment {
    variables = {
      LEDGER_TABLE         = aws_dynamodb_table.ledger.name
      QUARANTINE_QUEUE_URL = aws_sqs_queue.quarantine.url
      CONTRACT_NAME        = "stripe_charges_raw"
    }
  }

  tags = { "nmb:component" = "lambda-validate" }
}

# --- Step Functions ---------------------------------------------------------

resource "aws_cloudwatch_log_group" "sfn" {
  name              = "/aws/vendedlogs/states/${local.state_machine_name}"
  retention_in_days = 14
  tags              = { "nmb:component" = "sfn-pipeline" }
}

resource "aws_sfn_state_machine" "pipeline" {
  name     = local.state_machine_name
  role_arn = data.aws_iam_role.sfn_pipeline.arn
  type     = "STANDARD"
  tags     = { "nmb:component" = "sfn-pipeline" }

  logging_configuration {
    log_destination        = "${aws_cloudwatch_log_group.sfn.arn}:*"
    include_execution_data = false
    level                  = "ERROR"
  }

  definition = jsonencode({
    Comment = "P5 contract-enforcement pipeline. The validate Lambda writes the DynamoDB ledger and sends failures to SQS quarantine."
    StartAt = "ValidateBatch"
    States = {
      ValidateBatch = {
        Type     = "Task"
        Resource = "arn:aws:states:::lambda:invoke"
        Parameters = {
          FunctionName = aws_lambda_function.validate.arn
          "Payload.$"  = "$"
        }
        OutputPath = "$.Payload"
        Retry = [
          {
            ErrorEquals     = ["Lambda.ServiceException", "Lambda.AWSLambdaException", "Lambda.SdkClientException", "Lambda.TooManyRequestsException"]
            IntervalSeconds = 2
            MaxAttempts     = 3
            BackoffRate     = 2
          }
        ]
        Catch = [
          {
            ErrorEquals = ["States.ALL"]
            ResultPath  = "$.error"
            Next        = "InvokeFailed"
          }
        ]
        Next = "Route"
      }
      Route = {
        Type = "Choice"
        Choices = [
          {
            Variable           = "$.failed"
            NumericGreaterThan = 0
            Next               = "Quarantined"
          }
        ]
        Default = "AllPassed"
      }
      Quarantined = {
        Type    = "Succeed"
        Comment = "Failures were already sent to the quarantine queue by the Lambda."
      }
      AllPassed = {
        Type = "Succeed"
      }
      InvokeFailed = {
        Type  = "Fail"
        Error = "ValidateInvokeFailed"
        Cause = "The validate Lambda could not be invoked or failed."
      }
    }
  })
}

# --- EventBridge (custom bus; no S3 events) ---------------------------------

resource "aws_cloudwatch_event_bus" "pipeline" {
  name = local.event_bus_name
  tags = { "nmb:component" = "events-to-sfn" }
}

resource "aws_cloudwatch_event_rule" "batch_ready" {
  name           = "${local.prefix}-pipeline-batch-ready"
  description    = "Start the contract pipeline when a BatchReady event is put on the nmb-pipeline bus."
  event_bus_name = aws_cloudwatch_event_bus.pipeline.name
  event_pattern = jsonencode({
    source        = ["nmb.contractgate"]
    "detail-type" = ["BatchReady"]
  })
  tags = { "nmb:component" = "events-to-sfn" }
}

resource "aws_cloudwatch_event_target" "sfn" {
  rule           = aws_cloudwatch_event_rule.batch_ready.name
  event_bus_name = aws_cloudwatch_event_bus.pipeline.name
  arn            = aws_sfn_state_machine.pipeline.arn
  role_arn       = data.aws_iam_role.events_to_sfn.arn
}

# --- Metrics, dashboard, alarm ----------------------------------------------

resource "aws_cloudwatch_log_metric_filter" "passed" {
  name           = "${local.prefix}-pipeline-passed"
  log_group_name = aws_cloudwatch_log_group.lambda.name
  pattern        = "[marker=\"BATCH_RESULT\", passed, failed, total, reject_pct, batch_id]"

  metric_transformation {
    name          = "PassedRecords"
    namespace     = "NightMoose/Pipeline"
    value         = "$passed"
    default_value = "0"
    unit          = "Count"
  }
}

resource "aws_cloudwatch_log_metric_filter" "failed" {
  name           = "${local.prefix}-pipeline-failed"
  log_group_name = aws_cloudwatch_log_group.lambda.name
  pattern        = "[marker=\"BATCH_RESULT\", passed, failed, total, reject_pct, batch_id]"

  metric_transformation {
    name          = "FailedRecords"
    namespace     = "NightMoose/Pipeline"
    value         = "$failed"
    default_value = "0"
    unit          = "Count"
  }
}

resource "aws_cloudwatch_metric_alarm" "reject_rate" {
  alarm_name          = "${local.prefix}-pipeline-reject-rate"
  alarm_description   = "Reject rate of the P5 contract pipeline exceeded 25% in a 1-minute window."
  comparison_operator = "GreaterThanThreshold"
  evaluation_periods  = 1
  threshold           = 25
  treat_missing_data  = "notBreaching"
  tags                = { "nmb:component" = "reject-rate-alarm" }

  metric_query {
    id          = "passed"
    return_data = false
    metric {
      metric_name = "PassedRecords"
      namespace   = "NightMoose/Pipeline"
      period      = 60
      stat        = "Sum"
    }
  }

  metric_query {
    id          = "failed"
    return_data = false
    metric {
      metric_name = "FailedRecords"
      namespace   = "NightMoose/Pipeline"
      period      = 60
      stat        = "Sum"
    }
  }

  metric_query {
    id          = "rate"
    expression  = "IF((passed+failed)>0, 100*failed/(passed+failed), 0)"
    label       = "RejectRatePct"
    return_data = true
  }
}

resource "aws_cloudwatch_dashboard" "pipeline" {
  dashboard_name = local.dashboard_name
  dashboard_body = jsonencode({
    widgets = [
      {
        type   = "metric"
        x      = 0
        y      = 0
        width  = 12
        height = 6
        properties = {
          title  = "Records passed vs failed"
          region = local.region
          stat   = "Sum"
          period = 60
          metrics = [
            ["NightMoose/Pipeline", "PassedRecords"],
            [".", "FailedRecords"],
          ]
        }
      },
      {
        type   = "metric"
        x      = 12
        y      = 0
        width  = 12
        height = 6
        properties = {
          title  = "Reject rate (%)"
          region = local.region
          period = 60
          metrics = [
            [
              {
                expression = "IF((m1+m2)>0, 100*m2/(m1+m2), 0)"
                label      = "RejectRatePct"
                id         = "e1"
              }
            ],
            ["NightMoose/Pipeline", "PassedRecords", { id = "m1", visible = false, stat = "Sum" }],
            [".", "FailedRecords", { id = "m2", visible = false, stat = "Sum" }],
          ]
          annotations = {
            horizontal = [{ value = 25, label = "alarm" }]
          }
        }
      },
      {
        type   = "metric"
        x      = 0
        y      = 6
        width  = 8
        height = 6
        properties = {
          title  = "Lambda"
          region = local.region
          stat   = "Sum"
          period = 60
          metrics = [
            ["AWS/Lambda", "Invocations", "FunctionName", local.function_name],
            [".", "Errors", ".", "."],
            [".", "Throttles", ".", "."],
            [".", "Duration", ".", ".", { stat = "p95", yAxis = "right" }],
          ]
        }
      },
      {
        type   = "metric"
        x      = 8
        y      = 6
        width  = 8
        height = 6
        properties = {
          title  = "Step Functions executions"
          region = local.region
          stat   = "Sum"
          period = 60
          metrics = [
            ["AWS/States", "ExecutionsSucceeded", "StateMachineArn", aws_sfn_state_machine.pipeline.arn],
            [".", "ExecutionsFailed", ".", "."],
            [".", "ExecutionsTimedOut", ".", "."],
          ]
        }
      },
      {
        type   = "metric"
        x      = 16
        y      = 6
        width  = 8
        height = 6
        properties = {
          title  = "Quarantine queue depth"
          region = local.region
          stat   = "Average"
          period = 60
          metrics = [
            ["AWS/SQS", "ApproximateNumberOfMessagesVisible", "QueueName", aws_sqs_queue.quarantine.name],
            [".", "ApproximateNumberOfMessagesVisible", ".", aws_sqs_queue.dlq.name],
          ]
        }
      }
    ]
  })
}
