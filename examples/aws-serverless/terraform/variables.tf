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

variable "name_prefix" {
  type        = string
  description = "Resource name prefix. Roles are scoped to nmb-* names."
  default     = "nmb"
}

variable "lambda_zip" {
  type        = string
  description = "Path to the packaged validate Lambda zip (built by scripts/package.sh)."
  default     = "../dist/function.zip"
}

variable "lambda_timeout" {
  type    = number
  default = 60
}

variable "lambda_memory" {
  type    = number
  default = 256
}
