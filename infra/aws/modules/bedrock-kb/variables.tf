variable "name" {
  type        = string
  description = "Knowledge Base name. Use the nmb- prefix."
}

variable "role_arn" {
  type        = string
  description = "KB role ARN (nmb-bedrock-kb from the foundation stack)."
}

variable "embedding_model_arn" {
  type        = string
  description = "Titan Text Embeddings v2 model ARN."
  default     = "arn:aws:bedrock:us-east-1::foundation-model/amazon.titan-embed-text-v2:0"
}

variable "s3_bucket_arn" {
  type        = string
  description = "ARN of the documents bucket."
}

variable "s3_inclusion_prefixes" {
  type        = list(string)
  default     = []
  description = "Optional key prefixes to include from the documents bucket."
}

variable "vector_bucket_arn" {
  type        = string
  description = "S3 Vectors bucket ARN for the index."
}

variable "vector_index_arn" {
  type        = string
  description = "S3 Vectors index ARN."
}

variable "vector_index_name" {
  type        = string
  description = "S3 Vectors index name used by the storage configuration."
}

variable "tags" {
  type    = map(string)
  default = {}
}
