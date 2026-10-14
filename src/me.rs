//! `GET /me` and `GET /v1/me` — who am I?
//!
//! Returns the caller's org id and name, the org's plan (which governs the
//! API key's limits) and how the request authenticated.  Never returns emails.

use axum::{extract::State, response::Json};
use serde::Serialize;
use std::sync::Arc;
use uuid::Uuid;

use crate::api_key_auth::ValidatedKey;
use crate::error::{AppError, AppResult, DbOpContext};
use crate::AppState;

#[derive(Debug, Serialize)]
pub struct MeResponse {
    pub org_id: Uuid,
    /// `null` if the stored name looks like an email address.
    pub org_name: Option<String>,
    pub plan: String,
    pub plan_display: &'static str,
    /// `"api_key"` or `"session"` (dashboard JWT).
    pub auth: &'static str,
    /// The API key's id; `null` for session (JWT) auth.
    pub api_key_id: Option<Uuid>,
    pub monthly_event_limit: Option<i64>,
    pub rate_limit_rps: u32,
    pub rate_limit_burst: u32,
}

pub fn plan_display(plan: &str) -> &'static str {
    match plan {
        "free" => "Free",
        "aisg_free" => "AISchemaGate Developer",
        "growth" | "pro" => "Pro",
        "aisg_pro" => "AISchemaGate Pro",
        "enterprise" => "Enterprise",
        _ => "Unknown",
    }
}

/// Drop anything that looks like an email so it never leaks.
pub fn safe_org_name(name: String) -> Option<String> {
    if name.contains('@') {
        None
    } else {
        Some(name)
    }
}

pub fn build(key: &ValidatedKey, org_name: String, plan: String) -> MeResponse {
    let session = key.api_key_id.is_nil();
    let (def_rps, def_burst) = crate::plan::default_rate_limit(&plan);
    MeResponse {
        org_id: key.org_id,
        org_name: safe_org_name(org_name),
        plan_display: plan_display(&plan),
        auth: if session { "session" } else { "api_key" },
        api_key_id: (!session).then_some(key.api_key_id),
        monthly_event_limit: crate::plan::monthly_event_limit(&plan),
        rate_limit_rps: key.rate_limit_rps.unwrap_or(def_rps),
        rate_limit_burst: key.rate_limit_burst.unwrap_or(def_burst),
        plan,
    }
}

pub async fn me_handler(
    State(state): State<Arc<AppState>>,
    req: axum::extract::Request,
) -> AppResult<Json<MeResponse>> {
    let key = req
        .extensions()
        .get::<ValidatedKey>()
        .cloned()
        .ok_or(AppError::Unauthorized)?;
    let row: Option<(String, String)> = sqlx::query_as("SELECT name, plan FROM orgs WHERE id = $1")
        .bind(key.org_id)
        .fetch_optional(&state.db)
        .await
        .db_op("me_org")?;
    let (name, plan) = row.ok_or_else(|| AppError::NotFound("org not found".into()))?;
    Ok(Json(build(&key, name, plan)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(api_key_id: Uuid) -> ValidatedKey {
        ValidatedKey {
            api_key_id,
            user_id: Uuid::new_v4(),
            org_id: Uuid::new_v4(),
            allowed_contract_ids: None,
            rate_limit_rps: None,
            rate_limit_burst: None,
        }
    }

    #[test]
    fn api_key_auth_reports_key_and_plan() {
        let id = Uuid::new_v4();
        let me = build(&key(id), "acme".into(), "growth".into());
        assert_eq!(me.auth, "api_key");
        assert_eq!(me.api_key_id, Some(id));
        assert_eq!(me.plan_display, "Pro");
        assert_eq!(me.org_name.as_deref(), Some("acme"));
    }

    #[test]
    fn jwt_auth_has_no_key_id() {
        let me = build(&key(Uuid::nil()), "acme".into(), "free".into());
        assert_eq!(me.auth, "session");
        assert_eq!(me.api_key_id, None);
        assert_eq!(me.monthly_event_limit, Some(100_000));
    }

    #[test]
    fn email_like_org_name_is_dropped() {
        let me = build(&key(Uuid::nil()), "a@b.com".into(), "free".into());
        assert!(me.org_name.is_none());
        let json = serde_json::to_string(&me).unwrap();
        assert!(!json.contains('@'));
        assert!(!json.contains("user_id"));
    }
}
