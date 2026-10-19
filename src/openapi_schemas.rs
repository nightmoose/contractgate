//! Response schemas for routes whose OpenAPI entries were description-only.
//!
//! Shapes match the structs the handlers serialize (`ReplayResponse`,
//! `ContractSummary`, `MeResponse`, `EgressResponse`, and the other 2xx
//! bodies). Nullable fields are `Option` without `skip_serializing_if`.

use serde_json::{json, Map, Value};

const REPLAY_EMPTY: &str = "Invalid request: replay request must include at least one id";
const REPLAY_CAP: &str = "Invalid request: replay request exceeds cap of 1000 ids (got N)";
const REPLAY_CONTRACT: &str =
    "Invalid request: event_ids span multiple contracts or a contract other than contract_id";

pub(crate) fn apply(paths: &mut Map<String, Value>) {
    let violation = violation_schema();
    patch_contract_list(paths);
    patch_org_replay(paths, &violation);
    fill_replay_me_egress_ingest(paths, &violation);
    fill_version_bodies(paths);
    fill_sharing_and_collab(paths);
    fill_ingress(paths);
}

/// Replace placeholder `{}` schemas that mean "any JSON value".
pub(crate) fn expand_empty_schemas(value: &mut Value) {
    match value {
        Value::Array(items) => {
            for child in items {
                expand_empty_schemas(child);
            }
        }
        Value::Object(map) => {
            for (key, child) in map.iter_mut() {
                let empty = matches!(child, Value::Object(obj) if obj.is_empty());
                if empty && is_any_json_key(key) {
                    *child = any_json_schema();
                } else {
                    expand_empty_schemas(child);
                }
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

fn is_any_json_key(key: &str) -> bool {
    matches!(
        key,
        "raw_event" | "violation_details" | "transformed_event" | "value" | "received"
    )
}

fn any_json_schema() -> Value {
    json!({
        "description": "Any JSON value",
        "type": ["object", "array", "string", "number", "integer", "boolean", "null"]
    })
}

fn violation_schema() -> Value {
    let mut schema = json!({
        "type": "object",
        "required": ["field", "message", "kind"],
        "properties": {
            "field": {"type": "string"},
            "message": {"type": "string"},
            "kind": {
                "type": "string",
                "enum": [
                    "missing_required_field",
                    "type_mismatch",
                    "pattern_mismatch",
                    "enum_violation",
                    "range_violation",
                    "length_violation",
                    "metric_range_violation",
                    "unknown_field",
                    "undeclared_field",
                    "leakage_violation",
                    "completeness_violation",
                    "freshness_violation",
                    "uniqueness_violation",
                    "contract_version_deprecated"
                ]
            },
            "expected": {"type": "string"},
            "suggestion": {"type": "string"}
        }
    });
    let mut received = any_json_schema();
    received["description"] = json!("Offending value. Omitted when unset.");
    schema["properties"]["received"] = received;
    schema
}

fn error_body_schema() -> Value {
    json!({
        "type": "object",
        "required": ["error", "status"],
        "properties": {
            "error": {"type": "string"},
            "status": {"type": "integer"}
        }
    })
}

fn array_of(items: Value) -> Value {
    json!({"type": "array", "items": items})
}

fn ensure_response<'a>(
    paths: &'a mut Map<String, Value>,
    path: &str,
    method: &str,
    status: &str,
) -> &'a mut Value {
    let item = paths
        .get_mut(path)
        .and_then(Value::as_object_mut)
        .unwrap_or_else(|| panic!("openapi missing {path}"));
    let op = item
        .get_mut(method)
        .and_then(Value::as_object_mut)
        .unwrap_or_else(|| panic!("openapi missing {method} {path}"));
    let responses = op
        .entry("responses")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .unwrap_or_else(|| panic!("openapi responses for {method} {path}"));
    responses.entry(status).or_insert_with(|| json!({}))
}

fn put_schema(
    paths: &mut Map<String, Value>,
    path: &str,
    method: &str,
    status: &str,
    media: &str,
    schema: Value,
) {
    let resp = ensure_response(paths, path, method, status);
    if resp.get("description").and_then(Value::as_str).is_none() {
        resp["description"] = json!("Success");
    }
    if !resp["content"].is_object() {
        resp["content"] = json!({});
    }
    resp["content"][media] = json!({"schema": schema});
}

fn put_json(paths: &mut Map<String, Value>, path: &str, method: &str, status: &str, schema: Value) {
    put_schema(paths, path, method, status, "application/json", schema);
}

fn set_description(
    paths: &mut Map<String, Value>,
    path: &str,
    method: &str,
    status: &str,
    description: &str,
) {
    ensure_response(paths, path, method, status)["description"] = json!(description);
}

fn put_error(
    paths: &mut Map<String, Value>,
    path: &str,
    method: &str,
    status: &str,
    description: &str,
) {
    set_description(paths, path, method, status, description);
    put_json(paths, path, method, status, error_body_schema());
}

fn replay_bounds_text(tail: &str) -> String {
    format!("Empty id list: `{REPLAY_EMPTY}`. More than 1000 ids: `{REPLAY_CAP}`.{tail}")
}

fn patch_contract_list(paths: &mut Map<String, Value>) {
    let items = &mut paths.get_mut("/contracts").expect("GET /contracts")["get"]["responses"]
        ["200"]["content"]["application/json"]["schema"]["items"];
    items["properties"]["multi_stable_resolution"] = json!({
        "type": "string",
        "enum": ["strict", "fallback"],
        "description": "Unpinned resolution policy. Always present."
    });
    items["properties"]["latest_stable_version"] = json!({
        "type": ["string", "null"],
        "description": "Null when the contract has no stable version."
    });
    items["properties"]["version_count"] = json!({
        "type": "integer",
        "description": "Number of versions on the contract. Never null."
    });
    items["required"] = json!([
        "id",
        "name",
        "multi_stable_resolution",
        "latest_stable_version",
        "version_count",
        "store_event_payloads"
    ]);
}

fn patch_org_replay(paths: &mut Map<String, Value>, violation: &Value) {
    {
        let schema = &mut paths.get_mut("/quarantine/replay").expect("org replay")["post"]
            ["responses"]["200"]["content"]["application/json"]["schema"];
        schema["required"] = json!(["replayed", "outcomes"]);
        let item = &mut schema["properties"]["outcomes"]["items"];
        item["required"] = json!([
            "event_id",
            "version",
            "passed",
            "violations",
            "replayed_at",
            "redacted"
        ]);
        item["properties"]["violations"] = json!({"type": "array", "items": violation});
    }
    let history = &mut paths
        .get_mut("/quarantine/replay-history")
        .expect("replay history")["get"]["responses"]["200"]["content"]["application/json"]
        ["schema"];
    history["items"]["properties"]["violations"] = json!({"type": "array", "items": violation});
    put_error(
        paths,
        "/quarantine/replay",
        "post",
        "400",
        &replay_bounds_text(&format!(" `{REPLAY_CONTRACT}`.")),
    );
}

fn replay_response_schema(violation: &Value) -> Value {
    json!({
        "type": "object",
        "required": [
            "total",
            "replayed",
            "still_quarantined",
            "already_replayed",
            "not_found",
            "wrong_contract",
            "purged",
            "redacted",
            "target_version",
            "target_version_source",
            "target_is_draft",
            "results"
        ],
        "properties": {
            "total": {"type": "integer"},
            "replayed": {"type": "integer"},
            "still_quarantined": {"type": "integer"},
            "already_replayed": {"type": "integer"},
            "not_found": {"type": "integer"},
            "wrong_contract": {"type": "integer"},
            "purged": {"type": "integer"},
            "redacted": {"type": "integer"},
            "target_version": {"type": "string"},
            "target_version_source": {
                "type": "string",
                "enum": ["explicit", "default_stable"]
            },
            "target_is_draft": {"type": "boolean"},
            "results": {
                "type": "array",
                "items": {
                    "type": "object",
                    "required": ["quarantine_id", "outcome"],
                    "properties": {
                        "quarantine_id": {"type": "string", "format": "uuid"},
                        "outcome": {
                            "type": "string",
                            "enum": [
                                "replayed",
                                "still_quarantined",
                                "already_replayed",
                                "not_found",
                                "wrong_contract",
                                "purged",
                                "redacted"
                            ]
                        },
                        "replayed_into_audit_id": {"type": "string", "format": "uuid"},
                        "contract_version_matched": {"type": "string"},
                        "new_quarantine_id": {"type": "string", "format": "uuid"},
                        "contract_version_attempted": {"type": "string"},
                        "violation_count": {"type": "integer"},
                        "violations": {"type": "array", "items": violation}
                    }
                }
            }
        }
    })
}

fn me_response_schema() -> Value {
    json!({
        "type": "object",
        "required": [
            "org_id",
            "org_name",
            "plan",
            "plan_display",
            "paid_features",
            "self_hosted",
            "auth",
            "api_key_id",
            "monthly_event_limit",
            "rate_limit_rps",
            "rate_limit_burst"
        ],
        "properties": {
            "org_id": {"type": "string", "format": "uuid"},
            "org_name": {
                "type": ["string", "null"],
                "description": "Null when the stored name looks like an email."
            },
            "plan": {
                "type": "string",
                "description": "Stored org plan. On a hosted process the monthly limit and default rate limit follow this value. CONTRACTGATE_SELF_HOSTED leaves plan unchanged and reports the unlimited cap and the paid rate limit."
            },
            "paid_features": {
                "type": "boolean",
                "description": "True when paid features are allowed. True for a paid plan, and for every plan when CONTRACTGATE_SELF_HOSTED is set. A self-hosted free org still reports plan free."
            },
            "self_hosted": {
                "type": "boolean",
                "description": "True when CONTRACTGATE_SELF_HOSTED is set on this process. Dashboard Enterprise gates follow this. Hosted responses are false."
            },
            "plan_display": {
                "type": "string",
                "enum": [
                    "Free",
                    "AISchemaGate Developer",
                    "Pro",
                    "AISchemaGate Pro",
                    "Enterprise",
                    "Unknown"
                ]
            },
            "auth": {"type": "string", "enum": ["api_key", "session"]},
            "api_key_id": {
                "type": ["string", "null"],
                "format": "uuid",
                "description": "Null for session (JWT) auth."
            },
            "monthly_event_limit": {
                "type": ["integer", "null"],
                "description": "Null when the plan is unlimited."
            },
            "rate_limit_rps": {"type": "integer"},
            "rate_limit_burst": {"type": "integer"}
        }
    })
}

fn egress_response_schema(violation: &Value) -> Value {
    let payload_item = any_json_schema();
    json!({
        "type": "object",
        "required": [
            "total",
            "passed",
            "failed",
            "dry_run",
            "disposition",
            "egress_leakage_mode",
            "resolved_version",
            "payload",
            "outcomes"
        ],
        "properties": {
            "total": {"type": "integer"},
            "passed": {"type": "integer"},
            "failed": {"type": "integer"},
            "dry_run": {"type": "boolean"},
            "disposition": {"type": "string", "enum": ["block", "fail", "tag"]},
            "egress_leakage_mode": {"type": "string", "enum": ["off", "strip", "fail"]},
            "resolved_version": {"type": "string"},
            "requested_version": {
                "type": "string",
                "description": "Present only when the pin resolved to a differently written stored version."
            },
            "payload": {"type": "array", "items": payload_item},
            "outcomes": {
                "type": "array",
                "items": {
                    "type": "object",
                    "required": ["index", "passed", "violations", "validation_us", "action"],
                    "properties": {
                        "index": {"type": "integer"},
                        "passed": {"type": "boolean"},
                        "violations": {"type": "array", "items": violation},
                        "validation_us": {"type": "integer"},
                        "action": {
                            "type": "string",
                            "enum": ["included", "blocked", "rejected", "tagged"]
                        },
                        "stripped_fields": {
                            "type": "array",
                            "items": {"type": "string"},
                            "description": "Omitted when no undeclared fields were stripped."
                        }
                    }
                }
            }
        }
    })
}

fn v1_ingest_response_schema(violation: &Value) -> Value {
    let event = any_json_schema();
    json!({
        "type": "object",
        "required": [
            "total",
            "passed",
            "failed",
            "dry_run",
            "atomic",
            "resolved_version",
            "version_pin_source",
            "results"
        ],
        "properties": {
            "total": {"type": "integer"},
            "passed": {"type": "integer"},
            "failed": {"type": "integer"},
            "dry_run": {"type": "boolean"},
            "atomic": {"type": "boolean"},
            "resolved_version": {"type": "string"},
            "requested_version": {
                "type": "string",
                "description": "Present only when the pin resolved to a differently written stored version."
            },
            "version_pin_source": {
                "type": "string",
                "enum": ["query_param", "path", "default_stable", "pinned_deprecated"]
            },
            "results": {
                "type": "array",
                "items": {
                    "type": "object",
                    "required": [
                        "index",
                        "passed",
                        "violations",
                        "validation_us",
                        "forwarded",
                        "contract_version",
                        "quarantine_id",
                        "transformed_event"
                    ],
                    "properties": {
                        "index": {"type": "integer"},
                        "passed": {"type": "boolean"},
                        "violations": {"type": "array", "items": violation},
                        "validation_us": {"type": "integer"},
                        "forwarded": {"type": "boolean"},
                        "contract_version": {"type": "string"},
                        "quarantine_id": {"type": ["string", "null"], "format": "uuid"},
                        "transformed_event": event
                    }
                }
            }
        }
    })
}

fn batch_ingest_response_schema(violation: &Value) -> Value {
    let event = any_json_schema();
    json!({
        "type": "object",
        "required": [
            "total",
            "passed",
            "failed",
            "dry_run",
            "atomic",
            "resolved_version",
            "version_pin_source",
            "results"
        ],
        "properties": {
            "total": {"type": "integer"},
            "passed": {"type": "integer"},
            "failed": {"type": "integer"},
            "dry_run": {"type": "boolean"},
            "atomic": {"type": "boolean"},
            "resolved_version": {"type": "string"},
            "requested_version": {
                "type": "string",
                "description": "Present only when the pin resolved to a differently written stored version."
            },
            "version_pin_source": {
                "type": "string",
                "enum": ["header", "path", "default_stable", "pinned_deprecated"]
            },
            "results": {
                "type": "array",
                "items": {
                    "type": "object",
                    "required": [
                        "passed",
                        "violations",
                        "validation_us",
                        "forwarded",
                        "contract_version",
                        "transformed_event"
                    ],
                    "properties": {
                        "passed": {"type": "boolean"},
                        "violations": {"type": "array", "items": violation},
                        "validation_us": {"type": "integer"},
                        "forwarded": {"type": "boolean"},
                        "contract_version": {"type": "string"},
                        "transformed_event": event
                    }
                }
            }
        }
    })
}

fn fill_replay_me_egress_ingest(paths: &mut Map<String, Value>, violation: &Value) {
    let replay = replay_response_schema(violation);
    put_json(
        paths,
        "/contracts/{id}/quarantine/replay",
        "post",
        "200",
        replay,
    );
    put_error(
        paths,
        "/contracts/{id}/quarantine/replay",
        "post",
        "400",
        &replay_bounds_text(""),
    );
    let me = me_response_schema();
    put_json(paths, "/me", "get", "200", me.clone());
    put_json(paths, "/v1/me", "get", "200", me);
    let egress = egress_response_schema(violation);
    put_json(
        paths,
        "/egress/{contract_id}",
        "post",
        "200",
        egress.clone(),
    );
    put_json(paths, "/egress/{contract_id}", "post", "207", egress);
    let v1 = v1_ingest_response_schema(violation);
    put_json(paths, "/v1/ingest/{contract_id}", "post", "200", v1.clone());
    put_json(paths, "/v1/ingest/{contract_id}", "post", "207", v1);
    let ingest = batch_ingest_response_schema(violation);
    put_json(
        paths,
        "/ingest/{contract_id}",
        "post",
        "200",
        ingest.clone(),
    );
    put_json(paths, "/ingest/{contract_id}", "post", "207", ingest);
    set_description(
        paths,
        "/ingest/{contract_id}",
        "post",
        "207",
        "Some events failed",
    );
}

fn version_response_schema() -> Value {
    json!({
        "type": "object",
        "required": [
            "id",
            "contract_id",
            "version",
            "state",
            "yaml_content",
            "created_at",
            "promoted_at",
            "deprecated_at",
            "compliance_mode",
            "egress_leakage_mode",
            "import_source",
            "requires_review"
        ],
        "properties": {
            "id": {"type": "string", "format": "uuid"},
            "contract_id": {"type": "string", "format": "uuid"},
            "version": {"type": "string"},
            "state": {"type": "string", "enum": ["draft", "stable", "deprecated"]},
            "yaml_content": {"type": "string"},
            "created_at": {"type": "string", "format": "date-time"},
            "promoted_at": {"type": ["string", "null"], "format": "date-time"},
            "deprecated_at": {"type": ["string", "null"], "format": "date-time"},
            "compliance_mode": {"type": "boolean"},
            "egress_leakage_mode": {"type": "string", "enum": ["off", "strip", "fail"]},
            "import_source": {
                "type": "string",
                "enum": ["native", "odcs", "odcs_stripped", "publication"]
            },
            "requires_review": {"type": "boolean"}
        }
    })
}

fn version_summary_schema() -> Value {
    json!({
        "type": "object",
        "required": [
            "version",
            "state",
            "created_at",
            "promoted_at",
            "deprecated_at",
            "import_source",
            "requires_review"
        ],
        "properties": {
            "version": {"type": "string"},
            "state": {"type": "string", "enum": ["draft", "stable", "deprecated"]},
            "created_at": {"type": "string", "format": "date-time"},
            "promoted_at": {"type": ["string", "null"], "format": "date-time"},
            "deprecated_at": {"type": ["string", "null"], "format": "date-time"},
            "import_source": {
                "type": "string",
                "enum": ["native", "odcs", "odcs_stripped", "publication"]
            },
            "requires_review": {"type": "boolean"}
        }
    })
}

fn conformance_schema() -> Value {
    let flags = json!({
        "type": "object",
        "required": ["api_version", "kind", "id", "version", "status"],
        "properties": {
            "api_version": {"type": "boolean"},
            "kind": {"type": "boolean"},
            "id": {"type": "boolean"},
            "version": {"type": "boolean"},
            "status": {"type": "boolean"}
        }
    });
    let extensions = json!({
        "type": "object",
        "required": ["x_contractgate_version", "x_contractgate_ontology"],
        "properties": {
            "x_contractgate_version": {"type": "boolean"},
            "x_contractgate_ontology": {"type": "boolean"}
        }
    });
    json!({
        "type": "object",
        "required": [
            "version",
            "mandatory_fields_score",
            "mandatory_fields_detail",
            "extensions_score",
            "extensions_detail",
            "round_trip_fidelity_score",
            "round_trip_note",
            "quality_coverage_pct",
            "quality_covered_fields",
            "total_fields",
            "overall_score"
        ],
        "properties": {
            "version": {"type": "string"},
            "mandatory_fields_score": {"type": "number"},
            "mandatory_fields_detail": flags,
            "extensions_score": {"type": "number"},
            "extensions_detail": extensions,
            "round_trip_fidelity_score": {"type": "number"},
            "round_trip_note": {"type": "string"},
            "quality_coverage_pct": {"type": "number"},
            "quality_covered_fields": {"type": "integer"},
            "total_fields": {"type": "integer"},
            "overall_score": {"type": "number"}
        }
    })
}

fn deploy_schema() -> Value {
    json!({
        "type": "object",
        "required": [
            "contract_id",
            "version_id",
            "name",
            "version",
            "source",
            "deployed_by",
            "deployed_at",
            "deprecated_count"
        ],
        "properties": {
            "contract_id": {"type": "string", "format": "uuid"},
            "version_id": {"type": "string", "format": "uuid"},
            "name": {"type": "string"},
            "version": {"type": "string"},
            "source": {"type": ["string", "null"]},
            "deployed_by": {"type": ["string", "null"]},
            "deployed_at": {"type": "string", "format": "date-time"},
            "deprecated_count": {"type": "integer"}
        }
    })
}

fn fill_version_bodies(paths: &mut Map<String, Value>) {
    let version = version_response_schema();
    for (path, method, status) in [
        ("/contracts/{id}/versions/latest-stable", "get", "200"),
        ("/contracts/{id}/versions/{version}", "get", "200"),
        ("/contracts/{id}/versions/{version}", "patch", "200"),
        ("/contracts/{id}/versions/{version}/promote", "post", "200"),
        (
            "/contracts/{id}/versions/{version}/deprecate",
            "post",
            "200",
        ),
        (
            "/contracts/{id}/versions/{version}/approve-import",
            "post",
            "200",
        ),
        ("/contracts/import", "post", "201"),
        ("/contracts/{id}/versions", "post", "201"),
    ] {
        put_json(paths, path, method, status, version.clone());
    }
    put_json(
        paths,
        "/contracts/{id}/versions",
        "get",
        "200",
        array_of(version_summary_schema()),
    );
    put_json(
        paths,
        "/contracts/{id}/versions/{version}/odcs-conformance",
        "get",
        "200",
        conformance_schema(),
    );
    put_json(paths, "/contracts/deploy", "post", "201", deploy_schema());
    put_schema(
        paths,
        "/contracts/{id}/export",
        "post",
        "200",
        "text/csv",
        json!({"type": "string"}),
    );
}

fn publish_schema() -> Value {
    json!({
        "type": "object",
        "required": [
            "publication_ref",
            "visibility",
            "link_token",
            "contract_name",
            "contract_version",
            "published_at"
        ],
        "properties": {
            "publication_ref": {"type": "string"},
            "visibility": {"type": "string", "enum": ["public", "link", "org"]},
            "link_token": {"type": ["string", "null"]},
            "contract_name": {"type": "string"},
            "contract_version": {"type": "string"},
            "published_at": {"type": "string", "format": "date-time"}
        }
    })
}

fn fetched_publication_schema() -> Value {
    json!({
        "type": "object",
        "required": [
            "publication_ref",
            "contract_name",
            "contract_version",
            "visibility",
            "published_at",
            "yaml_content"
        ],
        "properties": {
            "publication_ref": {"type": "string"},
            "contract_name": {"type": "string"},
            "contract_version": {"type": "string"},
            "visibility": {"type": "string", "enum": ["public", "link", "org"]},
            "published_at": {"type": "string", "format": "date-time"},
            "yaml_content": {"type": "string"}
        }
    })
}

fn catalog_entry_schema() -> Value {
    json!({
        "type": "object",
        "required": [
            "publication_ref",
            "contract_name",
            "contract_version",
            "published_by",
            "published_at"
        ],
        "properties": {
            "publication_ref": {"type": "string"},
            "contract_name": {"type": "string"},
            "contract_version": {"type": "string"},
            "published_by": {"type": ["string", "null"]},
            "published_at": {"type": "string", "format": "date-time"}
        }
    })
}

fn my_publication_schema() -> Value {
    json!({
        "type": "object",
        "required": [
            "publication_ref",
            "contract_id",
            "contract_name",
            "contract_version",
            "visibility",
            "published_at",
            "import_count"
        ],
        "properties": {
            "publication_ref": {"type": "string"},
            "contract_id": {"type": "string", "format": "uuid"},
            "contract_name": {"type": "string"},
            "contract_version": {"type": "string"},
            "visibility": {"type": "string", "enum": ["public", "link", "org"]},
            "published_at": {"type": "string", "format": "date-time"},
            "import_count": {"type": "integer"}
        }
    })
}

fn import_published_schema() -> Value {
    json!({
        "type": "object",
        "required": [
            "contract_id",
            "version",
            "state",
            "message",
            "import_mode",
            "imported_from_ref"
        ],
        "properties": {
            "contract_id": {"type": "string", "format": "uuid"},
            "version": {"type": "string"},
            "state": {"type": "string", "enum": ["draft", "stable", "deprecated"]},
            "message": {"type": "string"},
            "import_mode": {"type": "string", "enum": ["snapshot", "subscribe"]},
            "imported_from_ref": {"type": "string"}
        }
    })
}

fn collaborator_schema() -> Value {
    json!({
        "type": "object",
        "required": ["contract_name", "org_id", "role", "granted_by", "granted_at"],
        "properties": {
            "contract_name": {"type": "string"},
            "org_id": {"type": "string", "format": "uuid"},
            "role": {"type": "string", "enum": ["viewer", "editor", "reviewer"]},
            "granted_by": {"type": "string", "format": "uuid"},
            "granted_at": {"type": "string", "format": "date-time"}
        }
    })
}

fn comment_schema() -> Value {
    json!({
        "type": "object",
        "required": [
            "id",
            "contract_name",
            "field",
            "org_id",
            "author",
            "body",
            "resolved",
            "created_at"
        ],
        "properties": {
            "id": {"type": "string", "format": "uuid"},
            "contract_name": {"type": "string"},
            "field": {"type": ["string", "null"]},
            "org_id": {"type": "string", "format": "uuid"},
            "author": {"type": "string"},
            "body": {"type": "string"},
            "resolved": {"type": "boolean"},
            "created_at": {"type": "string", "format": "date-time"}
        }
    })
}

fn proposal_schema() -> Value {
    json!({
        "type": "object",
        "required": [
            "id",
            "contract_name",
            "proposed_by",
            "proposed_yaml",
            "status",
            "decided_by",
            "created_at"
        ],
        "properties": {
            "id": {"type": "string", "format": "uuid"},
            "contract_name": {"type": "string"},
            "proposed_by": {"type": "string", "format": "uuid"},
            "proposed_yaml": {"type": "string"},
            "status": {
                "type": "string",
                "enum": ["open", "approved", "rejected", "applied"]
            },
            "decided_by": {"type": ["string", "null"], "format": "uuid"},
            "created_at": {"type": "string", "format": "date-time"}
        }
    })
}

fn fill_sharing_and_collab(paths: &mut Map<String, Value>) {
    put_json(
        paths,
        "/published/{publication_ref}",
        "get",
        "200",
        fetched_publication_schema(),
    );
    put_json(
        paths,
        "/catalog",
        "get",
        "200",
        array_of(catalog_entry_schema()),
    );
    put_json(
        paths,
        "/contracts/publications",
        "get",
        "200",
        array_of(my_publication_schema()),
    );
    put_json(
        paths,
        "/contracts/publications/{publication_ref}",
        "delete",
        "200",
        json!({
            "type": "object",
            "required": ["publication_ref", "revoked_at"],
            "properties": {
                "publication_ref": {"type": "string"},
                "revoked_at": {"type": "string", "format": "date-time"}
            }
        }),
    );
    put_json(
        paths,
        "/contracts/import-published",
        "post",
        "201",
        import_published_schema(),
    );
    put_json(
        paths,
        "/contracts/{id}/versions/{version}/publish",
        "post",
        "201",
        publish_schema(),
    );
    let collaborator = collaborator_schema();
    put_json(
        paths,
        "/contracts/{name}/collaborators",
        "get",
        "200",
        array_of(collaborator.clone()),
    );
    put_json(
        paths,
        "/contracts/{name}/collaborators",
        "post",
        "201",
        collaborator.clone(),
    );
    put_json(
        paths,
        "/contracts/{name}/collaborators/{org_id}",
        "patch",
        "200",
        collaborator,
    );
    let comment = comment_schema();
    put_json(
        paths,
        "/contracts/{name}/comments",
        "get",
        "200",
        array_of(comment.clone()),
    );
    put_json(
        paths,
        "/contracts/{name}/comments",
        "post",
        "201",
        comment.clone(),
    );
    put_json(
        paths,
        "/contracts/{name}/comments/{id}/resolve",
        "post",
        "200",
        comment,
    );
    let proposal = proposal_schema();
    put_json(
        paths,
        "/contracts/{name}/proposals",
        "get",
        "200",
        array_of(proposal.clone()),
    );
    put_json(
        paths,
        "/contracts/{name}/proposals",
        "post",
        "201",
        proposal.clone(),
    );
    put_json(
        paths,
        "/contracts/{name}/proposals/{id}/decide",
        "post",
        "200",
        proposal.clone(),
    );
    put_json(
        paths,
        "/contracts/{name}/proposals/{id}/apply",
        "post",
        "200",
        proposal,
    );
}

fn kafka_schema() -> Value {
    json!({
        "type": "object",
        "required": [
            "id",
            "contract_id",
            "enabled",
            "bootstrap_servers",
            "sasl_username",
            "topic_raw",
            "topic_clean",
            "topic_quarantine",
            "partition_count",
            "created_at"
        ],
        "properties": {
            "id": {"type": "string", "format": "uuid"},
            "contract_id": {"type": "string", "format": "uuid"},
            "enabled": {"type": "boolean"},
            "bootstrap_servers": {"type": "string"},
            "sasl_username": {"type": "string"},
            "sasl_password": {
                "type": "string",
                "description": "Present only on the first enable response."
            },
            "topic_raw": {"type": "string"},
            "topic_clean": {"type": "string"},
            "topic_quarantine": {"type": "string"},
            "partition_count": {"type": "integer"},
            "created_at": {"type": "string", "format": "date-time"}
        }
    })
}

fn kinesis_schema() -> Value {
    json!({
        "type": "object",
        "required": [
            "id",
            "contract_id",
            "enabled",
            "aws_region",
            "stream_raw",
            "stream_clean",
            "stream_quarantine",
            "raw_stream_arn",
            "clean_stream_arn",
            "quarantine_stream_arn",
            "iam_access_key_id",
            "shard_count",
            "created_at"
        ],
        "properties": {
            "id": {"type": "string", "format": "uuid"},
            "contract_id": {"type": "string", "format": "uuid"},
            "enabled": {"type": "boolean"},
            "aws_region": {"type": "string"},
            "stream_raw": {"type": "string"},
            "stream_clean": {"type": "string"},
            "stream_quarantine": {"type": "string"},
            "raw_stream_arn": {"type": ["string", "null"]},
            "clean_stream_arn": {"type": ["string", "null"]},
            "quarantine_stream_arn": {"type": ["string", "null"]},
            "iam_access_key_id": {"type": ["string", "null"]},
            "iam_secret_access_key": {
                "type": "string",
                "description": "Present only on first enable or credential rotation."
            },
            "shard_count": {"type": "integer"},
            "created_at": {"type": "string", "format": "date-time"}
        }
    })
}

fn fill_ingress(paths: &mut Map<String, Value>) {
    let kafka = kafka_schema();
    put_json(
        paths,
        "/contracts/{id}/kafka-ingress",
        "get",
        "200",
        kafka.clone(),
    );
    put_json(
        paths,
        "/contracts/{id}/kafka-ingress/enable",
        "post",
        "200",
        kafka.clone(),
    );
    put_json(
        paths,
        "/contracts/{id}/kafka-ingress/enable",
        "post",
        "201",
        kafka,
    );
    let kinesis = kinesis_schema();
    put_json(
        paths,
        "/contracts/{id}/kinesis-ingress",
        "get",
        "200",
        kinesis.clone(),
    );
    put_json(
        paths,
        "/contracts/{id}/kinesis-ingress/enable",
        "post",
        "200",
        kinesis.clone(),
    );
    put_json(
        paths,
        "/contracts/{id}/kinesis-ingress/enable",
        "post",
        "201",
        kinesis.clone(),
    );
    put_json(
        paths,
        "/contracts/{id}/kinesis-ingress/rotate-credentials",
        "post",
        "200",
        kinesis,
    );
}
