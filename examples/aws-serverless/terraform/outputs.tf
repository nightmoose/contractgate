output "function_name" {
  value = aws_lambda_function.validate.function_name
}

output "function_arn" {
  value = aws_lambda_function.validate.arn
}

output "state_machine_arn" {
  value = aws_sfn_state_machine.pipeline.arn
}

output "event_bus_name" {
  value = aws_cloudwatch_event_bus.pipeline.name
}

output "ledger_table" {
  value = aws_dynamodb_table.ledger.name
}

output "quarantine_queue_url" {
  value = aws_sqs_queue.quarantine.url
}

output "quarantine_dlq_url" {
  value = aws_sqs_queue.dlq.url
}

output "dashboard_name" {
  value = aws_cloudwatch_dashboard.pipeline.dashboard_name
}

output "reject_rate_alarm" {
  value = aws_cloudwatch_metric_alarm.reject_rate.alarm_name
}

output "how_to_drive" {
  value = "Do not use S3 from this box. python scripts/drive.py smoke  or  python scripts/drive.py run --records 100000"
}
