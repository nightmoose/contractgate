output "vector_bucket_arn" {
  description = "S3 Vectors bucket that holds both indexes."
  value       = aws_s3vectors_vector_bucket.rag.vector_bucket_arn
}

output "vector_bucket_name" {
  value = aws_s3vectors_vector_bucket.rag.vector_bucket_name
}

output "knowledge_bases" {
  description = "Knowledge Base ids and data source ids, keyed by chunking strategy."
  value = {
    for key, kb in aws_bedrockagent_knowledge_base.this :
    key => {
      knowledge_base_id  = kb.id
      knowledge_base_arn = kb.arn
      data_source_id     = aws_bedrockagent_data_source.custom[key].data_source_id
      index_arn          = aws_s3vectors_index.this[key].index_arn
    }
  }
}

output "guardrail_id" {
  value = aws_bedrock_guardrail.grounding.guardrail_id
}

output "guardrail_arn" {
  value = aws_bedrock_guardrail.grounding.guardrail_arn
}

output "guardrail_version" {
  value = aws_bedrock_guardrail_version.grounding.version
}

output "kb_role_arn" {
  value = var.kb_role_arn
}

output "embedding_model_arn" {
  value = var.embedding_model_arn
}
