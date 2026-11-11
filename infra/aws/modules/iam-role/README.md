# iam-role

Creates an IAM role under path `/nmb/` and always attaches the
`nmb-boundary` permissions boundary. The NightMoose deploy role can only
create roles that meet both of those constraints.

Do not call this module with a different path or boundary. Preconditions
will fail the plan.
