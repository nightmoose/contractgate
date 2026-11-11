variable "function_name" {
  type        = string
  description = "Lambda function name. Use the nmb- prefix."
}

variable "role_arn" {
  type        = string
  description = "Execution role ARN (typically the nmb-lambda-validate role from the foundation stack)."
}

variable "filename" {
  type        = string
  description = "Path to a deployment zip."
}

variable "handler" {
  type    = string
  default = "index.handler"
}

variable "runtime" {
  type    = string
  default = "python3.13"
}

variable "timeout" {
  type    = number
  default = 30
}

variable "memory_size" {
  type    = number
  default = 256
}

variable "environment" {
  type    = map(string)
  default = {}
}

variable "tags" {
  type    = map(string)
  default = {}
}

variable "source_code_hash" {
  type        = string
  description = "Optional hash to force an update when the zip changes."
  default     = null
}
