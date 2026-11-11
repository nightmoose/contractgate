terraform {
  required_providers {
    aws = {
      source  = "hashicorp/aws"
      version = ">= 5.0"
    }
  }
}

# P3 will instantiate this module once the docs bucket and S3 Vectors index
# exist. The foundation stack does not call it, so nothing here is applied
# in P1.
resource "aws_bedrockagent_knowledge_base" "this" {
  name     = var.name
  role_arn = var.role_arn
  tags     = var.tags

  knowledge_base_configuration {
    type = "VECTOR"
    vector_knowledge_base_configuration {
      embedding_model_arn = var.embedding_model_arn
    }
  }

  storage_configuration {
    type = "S3_VECTORS"
    s3_vectors_configuration {
      vector_bucket_arn = var.vector_bucket_arn
      index_arn         = var.vector_index_arn
      index_name        = var.vector_index_name
    }
  }
}

resource "aws_bedrockagent_data_source" "s3" {
  name                 = "${var.name}-docs"
  knowledge_base_id    = aws_bedrockagent_knowledge_base.this.id
  data_deletion_policy = "RETAIN"

  data_source_configuration {
    type = "S3"
    s3_configuration {
      bucket_arn         = var.s3_bucket_arn
      inclusion_prefixes = length(var.s3_inclusion_prefixes) > 0 ? var.s3_inclusion_prefixes : null
    }
  }
}
