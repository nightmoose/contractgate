# ecs-service

Fargate cluster, task definition, service, and log group. Not instantiated
by the foundation stack. P7 must point this at the staging database only,
never ContractGate prod or the Fly apps. Destroy the same day: Fargate and
any later ALB/IPv4 have an hourly cost.
