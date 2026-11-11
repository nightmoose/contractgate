variable "aws_region" {
  type        = string
  description = "AWS region for this stack. Proof projects stay in us-east-1."
  default     = "us-east-1"
}

variable "aws_account_id" {
  type        = string
  description = "NightMoose AWS account ID."
  default     = "065614567889"
}

variable "permissions_boundary_arn" {
  type        = string
  description = "Customer-managed permissions boundary attached to every /nmb/ role."
  default     = "arn:aws:iam::065614567889:policy/nmb-boundary"
}

variable "role_path" {
  type        = string
  description = "IAM path for workload roles. The deploy role can only manage roles under /nmb/."
  default     = "/nmb/"
}
