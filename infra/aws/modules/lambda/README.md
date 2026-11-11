# lambda

Reusable Lambda function plus a 14-day log group. Not instantiated by the
foundation stack. P5 should pass `role_arn` from `nmb-lambda-validate`.
Tear down with `terraform destroy` in the project stack (or `make destroy`).
