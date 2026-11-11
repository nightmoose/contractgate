terraform {
  required_providers {
    aws = {
      source  = "hashicorp/aws"
      version = ">= 5.0"
    }
  }
}

data "aws_iam_policy_document" "trust" {
  statement {
    sid     = "TrustedServices"
    effect  = "Allow"
    actions = ["sts:AssumeRole"]

    principals {
      type        = "Service"
      identifiers = var.trusted_services
    }
  }
}

resource "aws_iam_role" "this" {
  name                  = var.name
  path                  = var.path
  description           = var.description
  assume_role_policy    = data.aws_iam_policy_document.trust.json
  permissions_boundary  = var.permissions_boundary_arn
  max_session_duration  = var.max_session_duration
  force_detach_policies = true
  tags                  = var.tags

  lifecycle {
    precondition {
      condition     = var.path == "/nmb/"
      error_message = "Workload roles must use path /nmb/ so the deploy role can manage them."
    }
    precondition {
      condition     = var.permissions_boundary_arn == var.expected_boundary_arn
      error_message = "Every /nmb/ role must attach arn:aws:iam::065614567889:policy/nmb-boundary."
    }
    precondition {
      condition     = length(var.trusted_services) > 0
      error_message = "At least one trusted service principal is required."
    }
  }
}

resource "aws_iam_role_policy" "inline" {
  for_each = var.inline_policies

  name   = each.key
  role   = aws_iam_role.this.id
  policy = each.value
}

resource "aws_iam_role_policy_attachment" "managed" {
  for_each = toset(var.managed_policy_arns)

  role       = aws_iam_role.this.name
  policy_arn = each.value
}
