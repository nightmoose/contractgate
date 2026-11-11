# P3: two Bedrock Knowledge Bases on one S3 Vectors bucket, different
# chunking strategies, CUSTOM data sources (documents are ingested from
# the box via the Bedrock API so we never need s3:CreateBucket).
#
# The execution role already exists (P1): arn:aws:iam::065614567889:role/nmb/nmb-bedrock-kb
# This stack does not create or modify IAM roles.

resource "aws_s3vectors_vector_bucket" "rag" {
  vector_bucket_name = "${var.name_prefix}-vectors"
  force_destroy      = true
}

resource "aws_s3vectors_index" "this" {
  for_each = toset(["fixed", "semantic"])

  vector_bucket_name = aws_s3vectors_vector_bucket.rag.vector_bucket_name
  index_name         = each.key
  data_type          = "float32"
  dimension          = var.embedding_dimensions
  distance_metric    = "cosine"

  # Bedrock writes large text/metadata keys; they must be non-filterable
  # or index creation / ingestion can fail the 2 KB filterable limit.
  metadata_configuration {
    non_filterable_metadata_keys = [
      "AMAZON_BEDROCK_TEXT",
      "AMAZON_BEDROCK_METADATA",
      "x-amz-bedrock-kb-document-metadata",
      "source_path",
    ]
  }
}

resource "aws_bedrockagent_knowledge_base" "this" {
  for_each = aws_s3vectors_index.this

  name     = "${var.name_prefix}-${each.key}"
  role_arn = var.kb_role_arn
  tags = {
    "nmb:component" = "knowledge-base-${each.key}"
  }

  knowledge_base_configuration {
    type = "VECTOR"
    vector_knowledge_base_configuration {
      embedding_model_arn = var.embedding_model_arn
      embedding_model_configuration {
        bedrock_embedding_model_configuration {
          dimensions          = var.embedding_dimensions
          embedding_data_type = "FLOAT32"
        }
      }
    }
  }

  storage_configuration {
    type = "S3_VECTORS"
    s3_vectors_configuration {
      index_arn = each.value.index_arn
    }
  }
}

resource "aws_bedrockagent_data_source" "custom" {
  for_each = aws_bedrockagent_knowledge_base.this

  name                 = "${var.name_prefix}-docs-${each.key}"
  knowledge_base_id    = each.value.id
  data_deletion_policy = "DELETE"

  data_source_configuration {
    type = "CUSTOM"
  }

  vector_ingestion_configuration {
    chunking_configuration {
      chunking_strategy = each.key == "fixed" ? "FIXED_SIZE" : "SEMANTIC"

      dynamic "fixed_size_chunking_configuration" {
        for_each = each.key == "fixed" ? [1] : []
        content {
          max_tokens         = 512
          overlap_percentage = 20
        }
      }

      dynamic "semantic_chunking_configuration" {
        for_each = each.key == "semantic" ? [1] : []
        content {
          max_token                       = 300
          buffer_size                     = 0
          breakpoint_percentile_threshold = 95
        }
      }
    }
  }
}

# Contextual grounding / relevance checks for the eval harness.
# Guardrails have no idle charge. Do not reuse the Lexsi injection guardrail.
resource "aws_bedrock_guardrail" "grounding" {
  name                      = "${var.name_prefix}-grounding"
  blocked_input_messaging   = "This request cannot be processed."
  blocked_outputs_messaging = "This response cannot be given."
  description               = "P3 RAG eval: contextual grounding and relevance filters only."
  tags = {
    "nmb:component" = "guardrail-grounding"
  }

  contextual_grounding_policy_config {
    filters_config {
      type      = "GROUNDING"
      threshold = 0.75
    }
    filters_config {
      type      = "RELEVANCE"
      threshold = 0.75
    }
  }
}

resource "aws_bedrock_guardrail_version" "grounding" {
  guardrail_arn = aws_bedrock_guardrail.grounding.guardrail_arn
  description   = "P3 eval snapshot"
  skip_destroy  = true
}
