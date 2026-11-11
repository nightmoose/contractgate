output "function_name" {
  value = aws_lambda_function.this.function_name
}

output "function_arn" {
  value = aws_lambda_function.this.arn
}

output "log_group_name" {
  value = aws_cloudwatch_log_group.this.name
}
