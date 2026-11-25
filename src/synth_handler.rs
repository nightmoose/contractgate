//! `POST /contracts/synth`: synthetic rows from a contract (RFC-117).
//!
//! The contract comes inline (`yaml_content`) or from the caller's org by
//! `contract_id` (plus an optional `version`; the latest stable version
//! otherwise). Only the contract YAML is read. No stored events, quarantine
//! rows or other real samples are touched, and nothing is written.

use std::sync::Arc;

use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use contractgate::synth::{self, SynthMode, SynthOptions};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::{storage, AppState, OrgId};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SynthRequest {
    /// Contract YAML. Mutually exclusive with `contract_id`.
    #[serde(default)]
    pub yaml_content: Option<String>,
    /// A contract in the caller's org. Mutually exclusive with `yaml_content`.
    #[serde(default)]
    pub contract_id: Option<Uuid>,
    /// Version of `contract_id`. Latest stable when omitted.
    #[serde(default)]
    pub version: Option<String>,
    /// 1 to 10000. Default 100.
    #[serde(default = "default_rows")]
    pub rows: usize,
    #[serde(default)]
    pub mode: SynthMode,
    /// Same seed, same contract, same rows. Random (and echoed) when omitted.
    #[serde(default)]
    pub seed: Option<u64>,
}

fn default_rows() -> usize {
    100
}

pub async fn synth_handler(
    State(state): State<Arc<AppState>>,
    OrgId(org_id): OrgId,
    Json(req): Json<SynthRequest>,
) -> AppResult<Response> {
    let (yaml, source) = match (&req.yaml_content, req.contract_id) {
        (Some(_), Some(_)) => {
            return Err(AppError::BadRequest(
                "send either yaml_content or contract_id, not both".into(),
            ))
        }
        (None, None) => {
            return Err(AppError::BadRequest(
                "send yaml_content or contract_id".into(),
            ))
        }
        (Some(y), None) => (y.clone(), json!({"type": "inline"})),
        (None, Some(id)) => {
            if state.auth_configured() && org_id.is_none() {
                return Err(AppError::Unauthorized);
            }
            // Org check first: 404 for a contract outside the caller's org.
            storage::get_contract_identity(&state.db, id, org_id).await?;
            let cv = match &req.version {
                Some(v) => storage::get_version(&state.db, id, v, org_id).await?,
                None => storage::get_latest_stable_version(&state.db, id)
                    .await?
                    .ok_or_else(|| {
                        AppError::BadRequest(
                            "contract has no stable version; pass `version`".into(),
                        )
                    })?,
            };
            let source = json!({"type": "stored", "contract_id": id, "version": cv.version});
            (cv.yaml_content, source)
        }
    };

    let seed = req.seed.unwrap_or_else(rand::random);
    let opts = SynthOptions {
        rows: req.rows,
        mode: req.mode,
        seed,
        now_unix: chrono::Utc::now().timestamp(),
    };

    // CPU-bound; keep it off the async reactor.
    let result = tokio::task::spawn_blocking(move || {
        let compiled = synth::compile_yaml(&yaml)?;
        let name = compiled.contract.name.clone();
        let version = compiled.contract.version.clone();
        synth::generate(&compiled, &opts).map(|o| (name, version, o))
    })
    .await
    .map_err(|e| AppError::Internal(format!("synth task failed: {e}")))?;

    match result {
        Ok((name, version, out)) => {
            let count = out.rows.len() + out.invalid_rows.len();
            let rows: Value = match out.mode {
                SynthMode::Valid => Value::Array(out.rows),
                SynthMode::Invalid => serde_json::to_value(&out.invalid_rows)
                    .map_err(|e| AppError::Internal(e.to_string()))?,
            };
            Ok(Json(json!({
                "contract": {"name": name, "version": version},
                "source": source,
                "mode": out.mode.as_str(),
                "seed": out.seed,
                "row_count": count,
                "rows": rows,
                "skipped_rules": out.skipped_rules,
            }))
            .into_response())
        }
        Err(e) => {
            let status = if e.rule == "rows" {
                StatusCode::BAD_REQUEST
            } else {
                StatusCode::UNPROCESSABLE_ENTITY
            };
            let code = match e.rule.as_str() {
                "contract" => "invalid_contract",
                "rows" => "invalid_rows",
                _ => "unsatisfiable_rule",
            };
            Ok((
                status,
                Json(json!({
                    "error": code,
                    "rule": e.rule,
                    "field": e.field,
                    "message": e.message,
                    "status": status.as_u16(),
                })),
            )
                .into_response())
        }
    }
}

/// OpenAPI entry for the route (merged by `v1_ingest::openapi_handler`).
pub fn merge_openapi(paths: &mut serde_json::Map<String, Value>) {
    let rule_error = json!({"type": "object", "properties": {
        "error": {"type": "string", "enum": ["unsatisfiable_rule", "invalid_contract", "invalid_rows"]},
        "rule": {"type": "string"},
        "field": {"type": ["string", "null"]},
        "message": {"type": "string"},
        "status": {"type": "integer"}
    }});
    paths.insert("/contracts/synth".into(), json!({"post": {
        "operationId": "synth_rows",
        "summary": "Generate synthetic rows from a contract's rules",
        "description": "Rows come only from the contract (types, required, enum, min/max, lengths, pattern, format, quality rules, metrics). No stored events are read and nothing is written. mode=invalid returns rows that each break exactly one rule, with a broken_rule label. Deterministic for a given seed.",
        "requestBody": {"required": true, "content": {"application/json": {"schema": {"type": "object", "properties": {
            "yaml_content": {"type": "string", "description": "Contract YAML. Send this or contract_id."},
            "contract_id": {"type": "string", "format": "uuid"},
            "version": {"type": "string", "description": "Version of contract_id. Latest stable when omitted."},
            "rows": {"type": "integer", "minimum": 1, "maximum": 10000, "default": 100},
            "mode": {"type": "string", "enum": ["valid", "invalid"], "default": "valid"},
            "seed": {"type": "integer", "minimum": 0}
        }}}}},
        "responses": {
            "200": {"description": "Generated rows", "content": {"application/json": {"schema": {"type": "object", "properties": {
                "contract": {"type": "object", "properties": {"name": {"type": "string"}, "version": {"type": "string"}}},
                "source": {"type": "object"},
                "mode": {"type": "string"},
                "seed": {"type": "integer"},
                "row_count": {"type": "integer"},
                "rows": {"type": "array", "description": "valid: records. invalid: {record, broken_rule: {field, rule, metric?, violation_kind, message}}.", "items": {"type": "object"}},
                "skipped_rules": {"type": "array", "items": {"type": "object", "properties": {"field": {"type": "string"}, "rule": {"type": "string"}, "reason": {"type": "string"}}}}
            }}}}},
            "400": {"description": "Bad request, or rows outside 1 to 10000", "content": {"application/json": {"schema": rule_error.clone()}}},
            "401": {"description": "Missing or invalid credentials"},
            "404": {"description": "contract_id or version not found in the caller's org"},
            "422": {"description": "Invalid contract, or a rule cannot be satisfied. rule and field name it.", "content": {"application/json": {"schema": rule_error}}}
        },
        "security": [{"ApiKeyAuth": []}]
    }}));
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;
    use axum_test::TestServer;
    use serde_json::{json, Value};

    const YAML: &str = "version: \"1\"\nname: t\nontology:\n  entities:\n    - name: id\n      type: string\n      pattern: \"^A[0-9]{4}$\"\n    - name: n\n      type: integer\n      min: 1\n      max: 9\n";

    fn server() -> TestServer {
        let pool = sqlx::PgPool::connect_lazy("postgres://invalid:5432/none").expect("lazy pool");
        let state = std::sync::Arc::new(crate::AppState::new(pool, true, None, None));
        let app = axum::Router::new()
            .route(
                "/contracts/synth",
                axum::routing::post(super::synth_handler),
            )
            .with_state(state);
        TestServer::new(app)
    }

    #[tokio::test]
    async fn inline_valid_rows() {
        let r = server()
            .post("/contracts/synth")
            .json(&json!({"yaml_content": YAML, "rows": 20, "seed": 3}))
            .await;
        assert_eq!(r.status_code(), StatusCode::OK);
        let b = r.json::<Value>();
        assert_eq!(b["mode"], "valid");
        assert_eq!(b["seed"], 3);
        assert_eq!(b["row_count"], 20);
        assert_eq!(b["source"]["type"], "inline");
        let row = &b["rows"][0];
        assert!(row["id"].as_str().unwrap().starts_with('A'));
    }

    #[tokio::test]
    async fn inline_invalid_rows_are_labeled() {
        let r = server()
            .post("/contracts/synth")
            .json(&json!({"yaml_content": YAML, "rows": 10, "mode": "invalid", "seed": 3}))
            .await;
        assert_eq!(r.status_code(), StatusCode::OK);
        let b = r.json::<Value>();
        for row in b["rows"].as_array().unwrap() {
            assert!(row["record"].is_object());
            assert!(row["broken_rule"]["rule"].is_string());
        }
    }

    #[tokio::test]
    async fn errors_name_the_rule() {
        let s = server();
        let bad = "version: \"1\"\nname: t\nontology:\n  entities:\n    - name: c\n      type: string\n      min_length: 5\n      max_length: 2\n";
        let r = s
            .post("/contracts/synth")
            .json(&json!({"yaml_content": bad, "rows": 5}))
            .await;
        assert_eq!(r.status_code(), StatusCode::UNPROCESSABLE_ENTITY);
        let b = r.json::<Value>();
        assert_eq!(b["error"], "unsatisfiable_rule");
        assert_eq!(b["rule"], "min_length");
        assert_eq!(b["field"], "c");

        let r = s
            .post("/contracts/synth")
            .json(&json!({"yaml_content": YAML, "rows": 10001}))
            .await;
        assert_eq!(r.status_code(), StatusCode::BAD_REQUEST);
        assert_eq!(r.json::<Value>()["error"], "invalid_rows");

        let r = s.post("/contracts/synth").json(&json!({"rows": 5})).await;
        assert_eq!(r.status_code(), StatusCode::BAD_REQUEST);

        let r = s
            .post("/contracts/synth")
            .json(&json!({"yaml_content": "not: [valid", "rows": 5}))
            .await;
        assert_eq!(r.status_code(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(r.json::<Value>()["error"], "invalid_contract");
    }
}
