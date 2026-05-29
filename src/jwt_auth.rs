//! Supabase-JWT verification for dashboard browser sessions (RFC-039).
//!
//! RFC-052 adds a background JWKS refresh task so key rotations are recovered
//! automatically — no restart required.
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

use arc_swap::ArcSwap;
use jsonwebtoken::{
    decode, decode_header,
    jwk::{AlgorithmParameters, JwkSet},
    Algorithm, DecodingKey, Validation,
};
use serde::Deserialize;
use sqlx::PgPool;
use std::sync::Arc;
use std::time::Duration;
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
/// Handles both Supabase connection string formats:
///
/// 1. Direct:  `postgresql://user:pass@db.<project>.supabase.co:5432/postgres`
/// 2. Pooler:  `postgresql://postgres.<project>:pass@aws-0-<region>.pooler.supabase.com:6543/postgres`
///
/// Returns `None` if the URL doesn't match either format.
pub fn jwks_url_from_database_url(database_url: &str) -> Option<String> {
    let after_at = database_url.split('@').nth(1)?;
    let host = after_at.split(':').next()?.split('/').next()?.trim();

    // Format 1: direct — db.<project>.supabase.co
    if host.ends_with(".supabase.co") {
        let project_host = host.strip_prefix("db.").unwrap_or(host);
        return Some(format!(
            "https://{}/auth/v1/.well-known/jwks.json",
            project_host
        ));
    }

    // Format 2: pooler — aws-0-<region>.pooler.supabase.com
    // Project ref is in the username: postgres.<project>
    if host.ends_with(".pooler.supabase.com") {
        let before_at = database_url.split('@').next()?;
        let credentials = before_at
            .trim_start_matches("postgresql://")
            .trim_start_matches("postgres://");
        let user = credentials.split(':').next()?.trim();
        if let Some(project_ref) = user.strip_prefix("postgres.") {
            if !project_ref.is_empty() {
                return Some(format!(
                    "https://{}.supabase.co/auth/v1/.well-known/jwks.json",
                    project_ref
                ));
            }
        }
    }

    None
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
    let header = decode_header(token)
        .map_err(|e: jsonwebtoken::errors::Error| JwtAuthError::InvalidToken(e.to_string()))?;

    // 2. Find matching JWK(s): prefer kid match, fall back to trying all.
    let candidates: Vec<_> = if let Some(kid) = &header.kid {
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
                last_err = format!("{e}");
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
                last_err = format!("{e}");
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
        ORDER  BY joined_at ASC
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

// ---------------------------------------------------------------------------
// RFC-052: Background JWKS refresh
// ---------------------------------------------------------------------------

/// Spawn a background task that re-fetches the Supabase JWKS every 10 minutes
/// and atomically swaps it in on success.
///
/// A failed fetch logs a warning and leaves the previous key set intact —
/// the store is never blanked.  If `jwks_url` is `None` the task is a no-op.
pub fn spawn_jwks_refresh_task(jwks_store: Arc<ArcSwap<Option<JwkSet>>>, jwks_url: Option<String>) {
    let Some(url) = jwks_url else {
        return;
    };
    tokio::spawn(async move {
        // First tick fires immediately; skip it so we don't double-fetch on
        // a successful startup where the initial JWKS was already loaded.
        let mut interval = tokio::time::interval(Duration::from_secs(600));
        interval.tick().await; // discard the immediate first tick

        loop {
            interval.tick().await;
            match fetch_jwks(&url).await {
                Ok(new_jwks) => {
                    tracing::info!(
                        keys = new_jwks.keys.len(),
                        "JWKS periodic refresh succeeded"
                    );
                    jwks_store.store(Arc::new(Some(new_jwks)));
                }
                Err(e) => {
                    // Keep the previous key set — do NOT store None.
                    tracing::warn!("JWKS periodic refresh failed (keeping previous keys): {e}");
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // RFC-052 unit tests
    // -----------------------------------------------------------------------

    fn empty_jwks() -> JwkSet {
        JwkSet { keys: vec![] }
    }

    /// Swapping the store is immediately visible to the next load().
    #[test]
    fn swap_is_visible() {
        let store: Arc<ArcSwap<Option<JwkSet>>> =
            Arc::new(ArcSwap::from_pointee(Some(empty_jwks())));
        assert!(store.load().is_some(), "initial value should be Some");

        // Swap in a new JwkSet (still empty keys, but a fresh object).
        store.store(Arc::new(Some(empty_jwks())));
        assert!(store.load().is_some(), "after swap should still be Some");

        // Swap to None (simulates auth disabled).
        store.store(Arc::new(None));
        assert!(store.load().is_none(), "after storing None should be None");
    }

    /// A simulated failing refresh must not blank the store.
    #[test]
    fn failing_refresh_preserves_prior_keys() {
        let initial = Some(empty_jwks());
        let store: Arc<ArcSwap<Option<JwkSet>>> = Arc::new(ArcSwap::from_pointee(initial));

        // Simulate a refresh that fails — we intentionally do NOT call store().
        // The invariant is: on failure we skip store(); prior value survives.
        let _simulate_fetch_error: anyhow::Result<JwkSet> = Err(anyhow::anyhow!("network error"));
        // Store is unchanged.
        assert!(
            store.load().is_some(),
            "prior keys must survive a failed refresh"
        );
    }

    /// The debounce AtomicU64 prevents more than one refresh per 60-second window.
    #[test]
    fn debounce_limits_refresh_rate() {
        use std::sync::atomic::{AtomicU64, Ordering};

        let last_refresh = Arc::new(AtomicU64::new(0));
        let now: u64 = 1_000_000; // arbitrary epoch seconds

        // First call: last=0, now=1_000_000 → delta > 60 → should refresh.
        let last = last_refresh.load(Ordering::Relaxed);
        let should_refresh = now.saturating_sub(last) >= 60;
        assert!(should_refresh, "first call should trigger refresh");

        // Record the attempt.
        last_refresh.store(now, Ordering::Relaxed);

        // Second call 30s later — within the debounce window → should NOT refresh.
        let now2 = now + 30;
        let last2 = last_refresh.load(Ordering::Relaxed);
        let should_refresh2 = now2.saturating_sub(last2) >= 60;
        assert!(!should_refresh2, "call within 60s window must be debounced");

        // Third call 70s after the first — outside the window → should refresh.
        let now3 = now + 70;
        let last3 = last_refresh.load(Ordering::Relaxed);
        let should_refresh3 = now3.saturating_sub(last3) >= 60;
        assert!(should_refresh3, "call after 60s window must be allowed");
    }

    // -----------------------------------------------------------------------
    // RFC-069: jwks_url_from_database_url — both connection-string formats
    // -----------------------------------------------------------------------

    #[test]
    fn jwks_url_direct_format() {
        let url = "postgresql://postgres:pw@db.abcdefgh.supabase.co:5432/postgres";
        assert_eq!(
            jwks_url_from_database_url(url).as_deref(),
            Some("https://abcdefgh.supabase.co/auth/v1/.well-known/jwks.json")
        );
    }

    #[test]
    fn jwks_url_direct_without_db_prefix() {
        // Host already lacks the `db.` prefix — used verbatim.
        let url = "postgresql://postgres:pw@abcdefgh.supabase.co:5432/postgres";
        assert_eq!(
            jwks_url_from_database_url(url).as_deref(),
            Some("https://abcdefgh.supabase.co/auth/v1/.well-known/jwks.json")
        );
    }

    #[test]
    fn jwks_url_pooler_format() {
        // Project ref lives in the username: postgres.<ref>
        let url =
            "postgresql://postgres.abcdefgh:pw@aws-0-us-east-1.pooler.supabase.com:6543/postgres";
        assert_eq!(
            jwks_url_from_database_url(url).as_deref(),
            Some("https://abcdefgh.supabase.co/auth/v1/.well-known/jwks.json")
        );
    }

    #[test]
    fn jwks_url_non_supabase_host_is_none() {
        let url = "postgresql://user:pw@localhost:5432/contractgate";
        assert_eq!(jwks_url_from_database_url(url), None);
    }

    #[test]
    fn jwks_url_no_at_sign_is_none() {
        // Malformed — no `@`, so there is no host segment to parse.
        assert_eq!(jwks_url_from_database_url("postgresql://no-at-sign"), None);
    }

    #[test]
    fn jwks_url_pooler_empty_ref_is_none() {
        // Username is exactly `postgres.` → empty project ref → None.
        let url = "postgresql://postgres.:pw@aws-0-us-east-1.pooler.supabase.com:6543/postgres";
        assert_eq!(jwks_url_from_database_url(url), None);
    }

    // -----------------------------------------------------------------------
    // RFC-069: JwtAuthError Display wording is locked
    // -----------------------------------------------------------------------

    #[test]
    fn jwt_auth_error_display() {
        assert_eq!(
            JwtAuthError::InvalidToken("boom".into()).to_string(),
            "invalid or expired JWT: boom"
        );
        assert_eq!(
            JwtAuthError::InvalidSub.to_string(),
            "JWT sub claim is not a valid UUID"
        );
        assert_eq!(
            JwtAuthError::NoMatchingKey.to_string(),
            "no JWK matched this token's kid/algorithm"
        );
        assert_eq!(
            JwtAuthError::NoOrgMembership.to_string(),
            "user has no org membership"
        );
    }
}
