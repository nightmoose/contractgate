# Finding — No forgot-password on login

| Field | Value |
|-------|-------|
| Date | 2026-07-20 |
| Surface | `/auth/login` |
| Severity | minor / UX |
| Env | production app.datacontractgate.com |

## Observed

Login page offers GitHub OAuth + email/password only. No “Forgot password?” link.
Password recovery works via Supabase Auth API (`POST /auth/v1/recover`) but is
undiscoverable in the product.

## Expected

Standard “Forgot password?” → email link → set new password.

## Next experiment

Add link on login page calling `supabase.auth.resetPasswordForEmail` with
redirect to a `/auth/reset` route (does not exist yet).
