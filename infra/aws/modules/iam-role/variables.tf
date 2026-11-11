variable "name" {
  type        = string
  description = "IAM role name (no path). ARN will be arn:aws:iam::ACCOUNT:role/nmb/NAME."
}

variable "path" {
  type        = string
  description = "Must be /nmb/. The deploy role is scoped to this path."
  default     = "/nmb/"
}

variable "description" {
  type        = string
  description = "Human-readable role description."
  default     = ""
}

variable "permissions_boundary_arn" {
  type        = string
  description = "Required. Must be the nmb-boundary policy ARN."
}

variable "trusted_services" {
  type        = list(string)
  description = "AWS service principals allowed to assume this role."
}

variable "inline_policies" {
  type        = map(string)
  description = "Map of inline policy name to JSON document."
  default     = {}
}

variable "managed_policy_arns" {
  type        = list(string)
  description = "Optional AWS-managed or nmb/* customer-managed policy ARNs to attach."
  default     = []
}

variable "max_session_duration" {
  type        = number
  description = "Maximum session duration in seconds."
  default     = 3600
}

variable "tags" {
  type        = map(string)
  description = "Extra tags merged with provider default_tags."
  default     = {}
}

variable "expected_boundary_arn" {
  type        = string
  description = "Hard check so a caller cannot point the boundary at a different policy."
  default     = "arn:aws:iam::065614567889:policy/nmb-boundary"
}
