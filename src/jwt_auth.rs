//! Supabase-JWT verification for dashboard browser sessions (RFC-039).
//!
//! Supabase signs session JWTs with HS256 using the project's JWT secret
//! (`SUPABASE_JWT_SECRET`).  On a successful verification we look up the
//! user's primary org membership and return the same `ValidatedKey` struct
//! that the API-key path already injects into request extensions — so all
//! downstream handlers are unaware of which auth method was used.
//!
//! `api_key_id` is set to `Uuid::nil()` to signal "JWT-authed session".
//! Audit handlers that log key usage should treat nil as "dashboard session".

use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
use serde::Deserialize;
use sqlx::PgPool;
use uuid::Uuid;

use crate::api_key_auth::ValidatedKey;

// ---------------------------------------------------------------------------
// JWT claims
// ---------------------------------------------------------------------------

/// Minimal set of claims we care about from the Supabase-issued JWT.
/// Supabase uses standard fields: `sub` (user UUID), `exp` (expiry).
/// `role` is always `"authenticated"` for logged-in users.
#[derive(Debug, Deserialize)]
struct SupabaseClaims {
    /// Supabase user UUID (maps to `auth.users.id`).
    sub: String,
    /// Expiry epoch (seconds).  Validated by `jsonwebtoken` automatically.
    #[allow(dead_code)]
    exp: u64,
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum JwtAuthError {
    /// Token is missing, malformed, expired, or has an invalid signature.
    InvalidToken,
    /// `sub` claim is not a valid UUID.
    InvalidSub,
    /// The user has no live org membership.
    NoOrgMembership,
    /// Database error during membership lookup.
    DbError(sqlx::Error),
}

impl std::fmt::Display for JwtAuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidToken => write!(f, "invalid or expired JWT"),
            Self::InvalidSub => write!(f, "JWT sub claim is not a valid UUID"),
            Self::NoOrgMembership => write!(f, "user has no org membership"),
            Self::DbError(e) => write!(f, "DB error during JWT auth: {e}"),
        }
    }
}

// ---------------------------------------------------------------------------
// Core verification
// ---------------------------------------------------------------------------

/// Verify a Supabase-issued JWT, extract the user id, look up their primary
/// org, and return a `ValidatedKey` ready to inject into request extensions.
///
/// `secret` is the raw JWT secret string from `SUPABASE_JWT_SECRET`.
pub async fn verify_supabase_jwt(
    token: &str,
    secret: &str,
    db: &PgPool,
) -> Result<ValidatedKey, JwtAuthError> {
    // 1. Decode + verify: signature (HS256), expiry (`exp`), algorithm.
    let mut validation = Validation::new(Algorithm::HS256);
    // Supabase does not set `aud` consistently across project tiers; skip it.
    validation.validate_aud = false;

    let token_data = decode::<SupabaseClaims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &validation,
    )
    .map_err(|e| {
        tracing::warn!("JWT verification failed: {e}");
        JwtAuthError::InvalidToken
    })?;

    // 2. Parse `sub` as UUID.
    let user_id: Uuid = token_data
        .claims
        .sub
        .parse()
        .map_err(|_| JwtAuthError::InvalidSub)?;

    // 3. Look up the user's primary (oldest live) org membership.
    //    Backend connects as service role so no RLS interference here.
    let row = sqlx::query_as::<_, (Uuid,)>(
        r#"
        SELECT org_id
        FROM   org_memberships
        WHERE  user_id    = $1
          AND  deleted_at IS NULL
        ORDER  BY created_at ASC
        LIMIT  1
        "#,
    )
    .bind(user_id)
    .fetch_optional(db)
    .await
    .map_err(JwtAuthError::DbError)?;

    let org_id = row.ok_or(JwtAuthError::NoOrgMembership)?.0;

    // 4. Build a ValidatedKey with nil api_key_id (JWT session sentinel).
    Ok(ValidatedKey {
        api_key_id: Uuid::nil(),
        user_id,
        org_id,
        allowed_contract_ids: None, // JWT sessions are unrestricted
        rate_limit_rps: None,       // use default
        rate_limit_burst: None,     // use default
    })
}
