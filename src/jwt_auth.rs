//! Supabase-JWT verification for dashboard browser sessions (RFC-039).
//!
//! Supabase projects that have migrated to JWT Signing Keys issue RS256 (or
//! ES256) session tokens signed with an asymmetric key pair.  We verify these
//! by fetching the project's JWKS endpoint once at startup and caching the
//! key set.
//!
//! Legacy projects that still use the HS256 shared secret are NOT supported
//! by this module — use `SUPABASE_JWT_SECRET` only if your project has NOT
//! migrated.  For projects that show "Legacy JWT secret has been migrated to
//! new JWT Signing Keys" in the Supabase dashboard, use `SUPABASE_URL`.
//!
//! `api_key_id` in the returned `ValidatedKey` is set to `Uuid::nil()` to
//! signal "JWT-authed session".  Audit handlers should treat nil as "dashboard
//! session" rather than a real API key row.

use jsonwebtoken::{
    decode, decode_header,
    jwk::{AlgorithmParameters, JwkSet},
    Algorithm, DecodingKey, Validation,
};
use serde::Deserialize;
use sqlx::PgPool;
use uuid::Uuid;

use crate::api_key_auth::ValidatedKey;

// ---------------------------------------------------------------------------
// JWT claims
// ---------------------------------------------------------------------------

/// Minimal set of claims we need from the Supabase-issued JWT.
#[derive(Debug, Deserialize)]
struct SupabaseClaims {
    /// Supabase user UUID (maps to `auth.users.id`).
    sub: String,
    /// Expiry epoch (validated automatically by jsonwebtoken).
    #[allow(dead_code)]
    exp: u64,
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum JwtAuthError {
    /// Token is missing, malformed, expired, or has an invalid signature.
    InvalidToken(String),
    /// `sub` claim is not a valid UUID.
    InvalidSub,
    /// No JWK in the cached set matched the token's `kid` / algorithm.
    NoMatchingKey,
    /// The user has no live org membership.
    NoOrgMembership,
    /// Database error during membership lookup.
    DbError(sqlx::Error),
}

impl std::fmt::Display for JwtAuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidToken(e) => write!(f, "invalid or expired JWT: {e}"),
            Self::InvalidSub => write!(f, "JWT sub claim is not a valid UUID"),
            Self::NoMatchingKey => write!(f, "no JWK matched this token's kid/algorithm"),
            Self::NoOrgMembership => write!(f, "user has no org membership"),
            Self::DbError(e) => write!(f, "DB error during JWT auth: {e}"),
        }
    }
}

// ---------------------------------------------------------------------------
// JWKS fetch (called once at startup)
// ---------------------------------------------------------------------------

/// Derive the Supabase JWKS URL from `DATABASE_URL`.
///
/// Supabase database URLs have the form:
/// `postgresql://user:pass@db.<project>.supabase.co:5432/postgres`
///
/// The JWKS endpoint is at:
/// `https://<project>.supabase.co/auth/v1/.well-known/jwks.json`
///
/// Returns `None` if the URL doesn't look like a Supabase host.
pub fn jwks_url_from_database_url(database_url: &str) -> Option<String> {
    // Extract host: everything after "@" and before the next ":" or "/".
    let after_at = database_url.split('@').nth(1)?;
    let host = after_at
        .split(':')
        .next()?
        .split('/')
        .next()?
        .trim();

    // Must be a Supabase host: db.<project>.supabase.co
    if !host.ends_with(".supabase.co") {
        return None;
    }
    // Strip the "db." prefix added by Supabase for the Postgres endpoint.
    let project_host = host.strip_prefix("db.").unwrap_or(host);

    Some(format!(
        "https://{}/auth/v1/.well-known/jwks.json",
        project_host
    ))
}

/// Fetch the Supabase project's JWKS and return the key set.
///
/// `jwks_url` is the full URL to the JWKS endpoint.
pub async fn fetch_jwks(jwks_url: &str) -> anyhow::Result<JwkSet> {
    tracing::info!("Fetching Supabase JWKS from {jwks_url}");
    let jwks = reqwest::get(jwks_url)
        .await?
        .error_for_status()?
        .json::<JwkSet>()
        .await?;
    tracing::info!("Loaded {} JWK(s) from Supabase", jwks.keys.len());
    Ok(jwks)
}

// ---------------------------------------------------------------------------
// Core verification
// ---------------------------------------------------------------------------

/// Verify a Supabase-issued JWT against the cached JWKS, extract the user id,
/// look up their primary org, and return a `ValidatedKey`.
pub async fn verify_supabase_jwt(
    token: &str,
    jwks: &JwkSet,
    db: &PgPool,
) -> Result<ValidatedKey, JwtAuthError> {
    // 1. Decode header (no verification) to get `kid` and algorithm hint.
    let header = decode_header(token).map_err(|e| JwtAuthError::InvalidToken(e.to_string()))?;

    // 2. Find matching JWK(s): prefer kid match, fall back to trying all.
    let candidates: Vec<_> = if let Some(ref kid) = header.kid {
        jwks.keys
            .iter()
            .filter(|k| k.common.key_id.as_deref() == Some(kid.as_str()))
            .collect()
    } else {
        jwks.keys.iter().collect()
    };

    if candidates.is_empty() {
        tracing::warn!(
            kid = ?header.kid,
            "No JWK found for this token's kid"
        );
        return Err(JwtAuthError::NoMatchingKey);
    }

    // 3. Try each candidate key until one verifies.
    let mut last_err = String::from("no keys tried");
    let mut verified_claims: Option<SupabaseClaims> = None;

    'keys: for jwk in candidates {
        // Map JWK algorithm parameters to a jsonwebtoken Algorithm.
        let alg = match &jwk.algorithm {
            AlgorithmParameters::RSA(_) => {
                // Use the algorithm from the JWT header if available, default RS256.
                match header.alg {
                    Algorithm::RS384 => Algorithm::RS384,
                    Algorithm::RS512 => Algorithm::RS512,
                    _ => Algorithm::RS256,
                }
            }
            AlgorithmParameters::EllipticCurve(ec) => {
                use jsonwebtoken::jwk::EllipticCurve;
                match ec.curve {
                    EllipticCurve::P256 => Algorithm::ES256,
                    EllipticCurve::P384 => Algorithm::ES384,
                    _ => Algorithm::ES256,
                }
            }
            AlgorithmParameters::OctetKey(_) => Algorithm::HS256,
            _ => continue 'keys,
        };

        let decoding_key = match DecodingKey::from_jwk(jwk) {
            Ok(k) => k,
            Err(e) => {
                last_err = e.to_string();
                continue 'keys;
            }
        };

        let mut validation = Validation::new(alg);
        // Supabase does not set `aud` consistently across project tiers.
        validation.validate_aud = false;

        match decode::<SupabaseClaims>(token, &decoding_key, &validation) {
            Ok(data) => {
                verified_claims = Some(data.claims);
                break 'keys;
            }
            Err(e) => {
                last_err = e.to_string();
            }
        }
    }

    let claims = verified_claims.ok_or_else(|| {
        tracing::warn!("JWT verification failed against all candidate keys: {last_err}");
        JwtAuthError::InvalidToken(last_err)
    })?;

    // 4. Parse `sub` as UUID.
    let user_id: Uuid = claims.sub.parse().map_err(|_| JwtAuthError::InvalidSub)?;

    // 5. Look up the user's primary (oldest live) org membership.
    //    Backend connects as service role → no RLS interference.
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

    // 6. Build a ValidatedKey with nil api_key_id (JWT session sentinel).
    Ok(ValidatedKey {
        api_key_id: Uuid::nil(),
        user_id,
        org_id,
        allowed_contract_ids: None,
        rate_limit_rps: None,
        rate_limit_burst: None,
    })
}
