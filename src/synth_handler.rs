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
    /// RFC 3339 time that `freshness` rules are measured from. Now when
    /// omitted. Fix it (with `seed`) for rows that repeat exactly (RFC-118).
    #[serde(default)]
    pub anchor_time: Option<String>,
    /// Also return the rows wrapped in the contract's `envelope` as
    /// `envelope_payload` (RFC-118). 400 when the contract has none.
    #[serde(default)]
    pub envelope: bool,
    /// For `yaml_content` in ODCS with several `schema` objects: which one to
    /// use, by name. The first when omitted.
    #[serde(default)]
    pub schema_object: Option<String>,
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

    let anchor = match &req.anchor_time {
        Some(t) => chrono::DateTime::parse_from_rfc3339(t)
            .map_err(|e| {
                AppError::BadRequest(format!(
                    "anchor_time must be RFC 3339 (e.g. 2026-01-01T00:00:00Z): {e}"
                ))
            })?
            .with_timezone(&chrono::Utc),
        None => chrono::Utc::now(),
    };
    let want_envelope = req.envelope;
    let schema_object = req.schema_object.clone();
    let seed = req.seed.unwrap_or_else(rand::random);
    let opts = SynthOptions {
        rows: req.rows,
        mode: req.mode,
        seed,
        now_unix: anchor.timestamp(),
    };

    let started = std::time::Instant::now();
    // CPU-bound; keep it off the async reactor.
    let result = tokio::task::spawn_blocking(move || {
        let (compiled, notes) = synth::compile_yaml_with(&yaml, schema_object.as_deref())?;
        let name = compiled.contract.name.clone();
        let version = compiled.contract.version.clone();
        let out = synth::generate(&compiled, &opts)?;
        let payload = if want_envelope {
            let records = match out.mode {
                SynthMode::Valid => out.rows.clone(),
                SynthMode::Invalid => out.invalid_rows.iter().map(|r| r.record.clone()).collect(),
            };
            Some(synth::wrap_envelope(&compiled.contract, records)?)
        } else {
            None
        };
        Ok::<_, synth::SynthError>((name, version, out, payload, notes))
    })
    .await
    .map_err(|e| AppError::Internal(format!("synth task failed: {e}")))?;

    match result {
        Ok((name, version, out, payload, notes)) => {
            let count = out.rows.len() + out.invalid_rows.len();
            let rows: Value = match out.mode {
                SynthMode::Valid => Value::Array(out.rows),
                SynthMode::Invalid => serde_json::to_value(&out.invalid_rows)
                    .map_err(|e| AppError::Internal(e.to_string()))?,
            };
            let mut body = json!({
                "contract": {"name": name, "version": version},
                "source": source,
                "mode": out.mode.as_str(),
                "seed": out.seed,
                "anchor_time": anchor.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                "row_count": count,
                "rows": rows,
                "skipped_rules": out.skipped_rules,
                "generation_ms": started.elapsed().as_millis() as u64,
            });
            if let Some(p) = payload {
                body["envelope_payload"] = p;
            }
            if !notes.is_empty() {
                body["notes"] = json!(notes);
            }
            Ok(Json(body).into_response())
        }
        Err(e) => {
            let status = if e.rule == "rows" || e.rule == "envelope" {
                StatusCode::BAD_REQUEST
            } else {
                StatusCode::UNPROCESSABLE_ENTITY
            };
            let code = match e.rule.as_str() {
                "contract" => "invalid_contract",
                "rows" => "invalid_rows",
                "envelope" => "no_envelope",
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
        "error": {"type": "string", "enum": ["unsatisfiable_rule", "invalid_contract", "invalid_rows", "no_envelope"]},
        "rule": {"type": "string"},
        "field": {"type": ["string", "null"]},
        "message": {"type": "string"},
        "status": {"type": "integer"}
    }});
    paths.insert("/contracts/synth".into(), json!({"post": {
        "operationId": "synth_rows",
        "summary": "Generate synthetic rows from a contract's rules",
        "description": "Rows come only from the contract (types, required, enum, min/max, lengths, pattern, format, quality rules, metrics, cross_field rules, array items). No stored events are read and nothing is written. mode=invalid returns rows that each break exactly one rule, with a broken_rule label. Deterministic for a given seed.",
        "requestBody": {"required": true, "content": {"application/json": {"schema": {"type": "object", "properties": {
            "yaml_content": {"type": "string", "description": "Contract YAML. Send this or contract_id."},
            "contract_id": {"type": "string", "format": "uuid"},
            "version": {"type": "string", "description": "Version of contract_id. Latest stable when omitted."},
            "rows": {"type": "integer", "minimum": 1, "maximum": 10000, "default": 100},
            "mode": {"type": "string", "enum": ["valid", "invalid"], "default": "valid"},
            "seed": {"type": "integer", "minimum": 0},
            "anchor_time": {"type": "string", "format": "date-time", "description": "Time freshness rules are measured from. Now when omitted. Set it with seed for rows that repeat exactly."},
            "schema_object": {"type": "string", "description": "For ODCS yaml_content with several schema objects: the one to use, by name. The first when omitted."},
            "envelope": {"type": "boolean", "default": false, "description": "Also return the rows wrapped in the contract's envelope as envelope_payload. 400 no_envelope when the contract has none."}
        }}}}},
        "responses": {
            "200": {"description": "Generated rows", "content": {"application/json": {"schema": {"type": "object", "properties": {
                "contract": {"type": "object", "properties": {"name": {"type": "string"}, "version": {"type": "string"}}},
                "source": {"type": "object"},
                "mode": {"type": "string"},
                "seed": {"type": "integer"},
                "anchor_time": {"type": "string", "format": "date-time"},
                "envelope_payload": {"type": "object", "description": "Present when envelope=true: the records wrapped in the contract's envelope."},
                "notes": {"type": "array", "items": {"type": "string"}, "description": "ODCS content that did not map (ignored schema objects, composite key, relationships). Omitted when empty."},
                "row_count": {"type": "integer"},
                "rows": {"type": "array", "description": "valid: records. invalid: {record, broken_rule: {field, rule, metric?, violation_kind, message, constraint?}}.", "items": {"type": "object"}},
                "skipped_rules": {"type": "array", "items": {"type": "object", "properties": {"field": {"type": "string"}, "rule": {"type": "string"}, "reason": {"type": "string"}}}},
                "generation_ms": {"type": "integer", "description": "Server time spent compiling and generating, in milliseconds."}
            }}}}},
            "400": {"description": "Bad request, rows outside 1 to 10000, a bad anchor_time, or envelope=true on a contract with no envelope", "content": {"application/json": {"schema": rule_error.clone()}}},
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

    const ODCS_MULTI: &str = "apiVersion: v3.1.0\nkind: DataContract\nversion: 1.0.0\nschema:\n  - name: a\n    properties:\n      - name: x\n        logicalType: integer\n        required: true\n  - name: b\n    properties:\n      - name: y\n        logicalType: string\n        required: true\n";

    #[tokio::test]
    async fn odcs_schema_object_selects_and_notes_are_returned() {
        let r = server()
            .post("/contracts/synth")
            .json(&json!({"yaml_content": ODCS_MULTI, "rows": 3, "seed": 1}))
            .await;
        assert_eq!(r.status_code(), StatusCode::OK);
        let b = r.json::<Value>();
        assert!(b["rows"][0]["x"].is_number());
        assert!(b["notes"][0].as_str().unwrap().contains("ignored: b"));
        let r = server()
            .post("/contracts/synth")
            .json(&json!({"yaml_content": ODCS_MULTI, "rows": 3, "seed": 1, "schema_object": "b"}))
            .await;
        let b = r.json::<Value>();
        assert!(b["rows"][0]["y"].is_string());
        assert!(b["notes"][0].as_str().unwrap().contains("ignored: a"));
        let r = server()
            .post("/contracts/synth")
            .json(&json!({"yaml_content": ODCS_MULTI, "schema_object": "zzz"}))
            .await;
        assert_eq!(r.status_code(), StatusCode::UNPROCESSABLE_ENTITY);
        assert!(r.json::<Value>()["message"]
            .as_str()
            .unwrap()
            .contains("'zzz' not found"));
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

        let r = s
            .post("/contracts/synth")
            .json(&json!({"yaml_content": YAML, "rows": 5, "envelope": true}))
            .await;
        assert_eq!(r.status_code(), StatusCode::BAD_REQUEST);
        assert_eq!(r.json::<Value>()["error"], "no_envelope");

        let r = s
            .post("/contracts/synth")
            .json(&json!({"yaml_content": YAML, "rows": 5, "anchor_time": "yesterday"}))
            .await;
        assert_eq!(r.status_code(), StatusCode::BAD_REQUEST);
    }

    const V11: &str = "version: \"1\"\nname: t\nenvelope:\n  records_path: data\nontology:\n  entities:\n    - name: start\n      type: string\n      format: date\n    - name: end\n      type: string\n      format: date\n    - name: ts\n      type: integer\ncross_field:\n  - field: end\n    op: gte\n    other: start\nquality:\n  - field: ts\n    type: freshness\n    max_age_seconds: 60\n";

    #[tokio::test]
    async fn anchor_and_envelope() {
        let s = server();
        let body = json!({"yaml_content": V11, "rows": 15, "seed": 9,
            "anchor_time": "2026-01-01T00:00:00Z", "envelope": true});
        let a = s.post("/contracts/synth").json(&body).await;
        assert_eq!(a.status_code(), StatusCode::OK, "{}", a.text());
        let a = a.json::<Value>();
        let b = s.post("/contracts/synth").json(&body).await.json::<Value>();
        assert_eq!(a["rows"], b["rows"]);
        assert_eq!(a["anchor_time"], "2026-01-01T00:00:00Z");
        let anchor = 1_767_225_600_i64;
        for r in a["rows"].as_array().unwrap() {
            assert!(r["end"].as_str() >= r["start"].as_str());
            let ts = r["ts"].as_i64().unwrap();
            assert!(ts <= anchor && anchor - ts <= 60);
        }
        assert_eq!(a["envelope_payload"]["data"], a["rows"]);

        let inv = s
            .post("/contracts/synth")
            .json(
                &json!({"yaml_content": V11, "rows": 30, "seed": 9, "mode": "invalid",
                "anchor_time": "2026-01-01T00:00:00Z"}),
            )
            .await
            .json::<Value>();
        assert!(inv["rows"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["broken_rule"]["rule"] == "cross_field"));
    }
}
