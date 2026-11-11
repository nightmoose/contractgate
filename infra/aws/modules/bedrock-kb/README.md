# bedrock-kb

Bedrock Knowledge Base on S3 Vectors plus an S3 data source. Not
instantiated by the foundation stack. P3 should pass `role_arn` from
`nmb-bedrock-kb`. Confirm the `s3_vectors_configuration` argument names
against the AWS provider docs when wiring P3; they may change.
