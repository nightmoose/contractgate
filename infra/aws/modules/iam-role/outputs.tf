output "name" {
  description = "IAM role name."
  value       = aws_iam_role.this.name
}

output "arn" {
  description = "IAM role ARN (path /nmb/)."
  value       = aws_iam_role.this.arn
}

output "path" {
  description = "IAM role path."
  value       = aws_iam_role.this.path
}

output "permissions_boundary" {
  description = "Permissions boundary ARN attached to the role."
  value       = aws_iam_role.this.permissions_boundary
}
