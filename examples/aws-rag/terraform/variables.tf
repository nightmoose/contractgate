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

variable "kb_role_arn" {
  type        = string
  description = "Existing P1 role for the Knowledge Base (path /nmb/, nmb-boundary attached). Hard-coded so apply does not need iam:GetRole as the bot user."
  default     = "arn:aws:iam::065614567889:role/nmb/nmb-bedrock-kb"
}

variable "embedding_model_arn" {
  type        = string
  description = "Amazon Titan Text Embeddings V2 (no Marketplace models)."
  default     = "arn:aws:bedrock:us-east-1::foundation-model/amazon.titan-embed-text-v2:0"
}

variable "embedding_dimensions" {
  type        = number
  description = "Titan Text Embeddings V2 default dimension."
  default     = 1024
}

variable "name_prefix" {
  type        = string
  description = "Resource name prefix. Must stay nmb- so the KB role's s3vectors:* grant matches."
  default     = "nmb-rag"
}
