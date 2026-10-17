# Code Scan Findings — 2026-05-28

**Scope:** Read-only scan of `src/` directory (Rust backend only). Ignored: `dashboard/`, `tests/`, `docs/`, entire `src/tests.rs`, and every `#[cfg(test)]` block / `mod tests` across all files. No test code examined. No code was edited, no cargo invoked.

**Methodology:** Used ripgrep-based searches for the four exact patterns. For every candidate hit, read 5–15 lines of surrounding context (including locating the nearest `#[cfg(test)]` guard) to confirm it is prod code. Manually traced call graphs from Axum route handlers in `main.rs:build_router` (and delegated modules: ingest, egress, v1_ingest, infer_*, collaboration, publication, replay, scorecard, scaffold_handler, etc.) to classify reachability. Startup code (main(), AppState::new, install_recorder, warm_cache, etc.) noted separately.

---

## Pattern 1: Panic on the Request Path

**Summary:** 9 hits in prod (non-test) code. 5 are reachable from HTTP handlers. 3 are defensive `.expect` on operations the authors documented as impossible; 2 are post-condition `.expect` after role/revoke checks. No bare `panic!`, `unreachable!`, or `todo!` in request paths. Several guarded index ops still flagged per spec.

#### src/collaboration.rs:186
```rust
185|    validate_collaborator_role(&body.role)?;
186|    let granted_by = org_id.expect("owner check guarantees org_id is set");
187|    let row = storage::grant_collaborator(
```
**Context (grant_collaborator_handler):** Called from POST /contracts/{name}/collaborators after require_role(..., Owner). org_id comes from org_id_from_req (main.rs:326).
**Why it might matter:** If require_role ever returns Ok while org_id is None (logic bug, future refactor of dev_no_auth path, or JWT extraction change), this panics on every owner-level collaborator grant.
**Tag:** [request-path]

#### src/collaboration.rs:262
```rust
260|    require_role(&state, &contract_name, org_id, CallerRole::Viewer).await?;
261|
262|    let caller_org = org_id.expect("viewer check guarantees org_id is set");
263|    let Json(body): Json<AddCommentRequest> = ...
```
**Context (add_comment_handler):** POST /contracts/{name}/comments after Viewer role check.
**Why it might matter:** Same invariant assumption as line 186. Panic would take down the worker on a comment POST from a legitimately authenticated collaborator.
**Tag:** [request-path]

#### src/collaboration.rs:329
```rust
327|    require_role(&state, &contract_name, org_id, CallerRole::Editor).await?;
328|
329|    let caller_org = org_id.expect("editor check guarantees org_id is set");
330|    let Json(body): Json<CreateProposalRequest> = ...
```
**Context (create_proposal_handler):** POST /contracts/{name}/proposals.
**Why it might matter:** Identical pattern; panic surface on proposal creation path.
**Tag:** [request-path]

#### src/collaboration.rs:356
```rust
354|    require_role(&state, &contract_name, org_id, CallerRole::Reviewer).await?;
355|
356|    let caller_org = org_id.expect("reviewer check guarantees org_id is set");
357|    let Json(body): Json<DecideProposalRequest> = ...
```
**Context (decide_proposal_handler):** POST /contracts/{name}/proposals/{id}/decide.
**Why it might matter:** Reviewer decision path now carries the same panic risk.
**Tag:** [request-path]

#### src/publication.rs:184
```rust
182|    Ok(Json(RevokeResponse {
183|        publication_ref: row.publication_ref,
184|        revoked_at: row.revoked_at.expect("revoke always sets revoked_at"),
185|    }))
```
**Context (revoke_handler):** DELETE /contracts/publications/{publication_ref} after storage::revoke_publication.
**Why it might matter:** If the UPDATE in storage ever fails to set revoked_at (constraint change, partial migration, concurrent delete), a legitimate revoke returns 200 but then panics before the response is sent.
**Tag:** [request-path]

#### src/transform.rs:173
```rust
169|    // The `.expect()` is defensive — the concrete `new_from_slice` on
170|    // `HmacSha256` can only fail for algorithms with a fixed key size,
171|    // which SHA-256 is not.
172|    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC-SHA256 accepts any key length");
```
**Context (hmac_sha256_hex):** Called from apply_transforms (kind: hash) and format_preserving paths during every ingest and egress that uses RFC-004 transforms.
**Why it might matter:** Reachable on every PII-masking ingest/egress. Currently impossible per comment, but a future hmac crate change or key material edge case would panic the request worker.
**Tag:** [request-path]

#### src/transform.rs:196
```rust
195|    let seed_bytes = {
196|        let mut mac = HmacSha256::new_from_slice(salt).expect("HMAC-SHA256 accepts any key length");
197|        ...
```
**Context (format_preserving_mask):** Same hot path as above, used for `format_preserving` transform in both ingest and egress.
**Why it might matter:** Same risk surface as line 173.
**Tag:** [request-path]

#### src/transform.rs:219
```rust
218|    }
219|    String::from_utf8(out).expect("format-preserving mask produced invalid UTF-8")
```
**Context (format_preserving_mask):** End of the mask function (builds only ASCII replacements + passthrough bytes).
**Why it might matter:** In theory unreachable (all paths preserve UTF-8 validity), but still a panic on the transform hot path used by ingest/egress.
**Tag:** [request-path]

#### src/ingest.rs:341
```rust
335|        if events.len() != 1 {
336|            return Err(...);
337|        }
338|        }
339|        let payload = &events[0];
340|        let result: BatchValidationResult =
```
**Context (ingest_handler, envelope short-circuit):** After explicit len check.
**Why it might matter:** Guarded, but still direct indexing on request-derived Vec. If the check is ever moved or events mutated between, panic.
**Tag:** [request-path]

#### src/replay.rs:419
```rust
418|            } => {
419|                let idx = *ordinal_for_id.get(&source_id).unwrap();
420|                if stamped.contains(&source_id) {
```
**Context (replay_handler):** Inside loop over `pending` items that were just inserted into ordinal_for_id from the same set of source_ids.
**Why it might matter:** Logic invariant (every pending source_id has an ordinal entry), but a bug in the preceding HashMap construction or concurrent modification would panic on replay POST.
**Tag:** [request-path]

#### src/replay.rs:436 (identical pattern)
```rust
435|            } => {
436|                let idx = *ordinal_for_id.get(&source_id).unwrap();
```
**Context:** Same function, Fail arm.
**Tag:** [request-path]

#### src/infer_url.rs:252
```rust
233|    if addrs.is_empty() {
234|        return Err(...);
235|    }
236|    ...
252|    Ok((host, addrs[0]))
```
**Context (check_ssrf, called by infer_url_handler):** Guarded.
**Why it might matter:** Direct [0] after DNS lookup on /contracts/infer/url requests. Guarded today.
**Tag:** [request-path]

#### src/infer_url.rs:266 (and 270,274,278,282,286,...)
```rust
265|    let o = ip.octets();
266|    if o[0] == 0 { ... }
270|    if o[0] == 127 { ... }
```
**Context (is_blocked_v4 / is_blocked_v6):** Fixed-size arrays from Ipv4Addr/Ipv6Addr (4/16 bytes).
**Why it might matter:** Always in-bounds; benign.
**Tag:** [request-path] [benign]

### Startup Panics (Lower Priority)

#### src/main.rs:1533
```rust
1532|            .init();
1533|        let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
```
**Context:** scorecard-rollup CLI subcommand path (early in main, before any HTTP).
**Tag:** [startup]

#### src/main.rs:1554 (identical)
```rust
1554|    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
```
**Context:** Normal server boot path.
**Tag:** [startup]

#### src/observability.rs:60
```rust
59|                VALIDATION_BUCKETS,
60|            )
61|            .expect("valid bucket values")
62|            .install_recorder()
63|            .expect("failed to install Prometheus recorder")
```
**Context (install_recorder):** Called once from main() before build_router / axum::serve.
**Tag:** [startup]

#### src/stream_demo.rs:154
```rust
153|            hist: Mutex::new(
154|                Histogram::<u64>::new_with_bounds(1, 60_000_000, 3)
155|                    .expect("valid HDR histogram bounds"),
```
**Context (LaneCounters::new):** Called from StreamDemoState::new at AppState construction (startup).
**Tag:** [startup]

#### src/stream_demo.rs:249
```rust
248|            let parsed: Contract = serde_yaml::from_str(yaml_src)
249|                .unwrap_or_else(|e| panic!("demo scenario '{name}' YAML invalid: {e}"));
```
**Context (StreamDemoState::new):** Same startup path as above. Demo-only scenarios.
**Tag:** [startup]

#### src/stream_demo.rs:251 (identical pattern for compile)
**Tag:** [startup]

---

## Pattern 2: Swallowed Errors

**Summary:** ~12 candidates in prod code. Vast majority are the intentional `let _ = foo()?;` pattern (call only for its error side-effect, discard the successful value) used for org-scoping permission checks and cache warm-ups. A few `.ok()` on env var / header parsing. One fire-and-forget cache preload that truly discards errors. No empty `Err(_) => {}` arms that hide failures in request paths. No dangerous `let _ =` on fallible operations whose errors should have been surfaced.

#### src/main.rs:520
```rust
519|    if org_id.is_some() {
520|        let _ = storage::get_contract_identity(&state.db, contract_id, org_id).await?;
521|    }
```
**Context (patch_contract_handler):** Typical org-scope guard before mutation.
**Why it might matter:** The `?` still turns "not found / wrong org" into 404/401. The `let _` only discards the identity row we do not need. Correct but easy to misread as swallowing.
**Tag:** [request-path] [benign]

#### src/main.rs:555
```rust
554|    // Warm the cache for the new stable version so the first ingest is fast.
555|    let _ = state.get_compiled(contract_id, &version).await;
556|    Ok(Json(VersionResponse::from(&v)))
```
**Context (deploy_contract_handler):** After successful deploy, best-effort preload.
**Why it might matter:** Compilation or cache error is completely swallowed. Next ingest will pay the compile cost. Intentional (deploy latency > cache warmth).
**Tag:** [request-path] [benign]

#### src/main.rs:1153 (identical pattern in v1_contract_version_handler)
**Tag:** [request-path] [benign]

#### src/ingest.rs:917
```rust
916|    if org_id.is_some() {
917|        let _ = storage::get_contract_identity(&state.db, contract_id, None).await?;
918|    }
```
**Context (ingest_stats_handler):** Permission check only.
**Tag:** [request-path] [benign]

#### src/storage.rs:415,807,939,1027 (four sites)
All identical `let _ = get_contract_identity(...)?;` guards inside storage helpers called from handlers (list_versions, replay, etc.).
**Tag:** [request-path] [benign]

#### src/main.rs:964
```rust
964|        .duration_since(std::time::UNIX_EPOCH)
965|        .unwrap_or_default()
966|        .as_secs();
```
**Context (name-history timestamp formatting):** SystemTime before epoch is extremely unlikely; defaulting to 0 is acceptable.
**Tag:** [request-path] [benign]

#### src/api_key_auth.rs:205
```rust
204|                        {
205|                            Ok(_) => {}
206|                            Err(e) => {
207|                                tracing::warn!(... "failed to update last_used_at: {e}");
208|                            }
```
**Context (fire-and-forget last_used_at update inside validate):** Error is logged at warn, not hidden. Acceptable for non-critical side effect.
**Tag:** [request-path] [benign]

#### Additional benign .ok() patterns (header / env parsing)
Dozens of `headers.get("x-foo").and_then(|v| v.to_str().ok())` and `std::env::var("FOO").ok()` throughout ingest, egress, v1_ingest, main, jwt_auth, etc. All are for optional headers or feature flags; absence is a normal case.
**Tag:** [request-path] [benign]

---

## Pattern 3: Secret / PII in Logs

**Summary:** 0 findings. Every tracing/log/println/eprintln call in prod request paths was inspected. No raw API keys, bearer tokens, JWTs, full header values, passwords, salts, or raw request/event bodies are ever interpolated into log arguments.

Representative safe patterns verified:

- `tracing::warn!("JWT verification failed: {e}")` — only the error string, never the token (main.rs:1066, jwt_auth.rs:222).
- `tracing::info!(keys = new_jwks.keys.len(), "JWKS refreshed...")` — only count (main.rs:989).
- `tracing::warn!("Rejected request: missing or invalid x-api-key")` — no value (main.rs:1116).
- `tracing::warn!(api_key_id = %key_id, "failed to update last_used_at")` — UUID only, never the secret (api_key_auth.rs:207).
- All ingest/egress audit paths log `contract_id`, `version`, `violation_count`, `passed` — never the payload bytes or PII fields.
- eprintln! only in scaffold CLI paths for human progress (scaffold/mod.rs:137,151) and profiler budget warnings — no secrets.
- demo/ and bin/ paths occasionally print counts or URLs, never credentials in the server binary.

No `debug!` or `trace!` of full `body` or `headers` maps. The observability middleware (track_requests) records only method + matched path.

**Tag:** [request-path] [benign] (clean)

---

## Pattern 4: Lossy Numeric Casts

**Summary:** 27 hits in prod code. The dominant pattern is `violations.len() as i32` and `validation_us as i64` for DB insert columns that are typed `integer` / `bigint`. Also `as u64` for Prometheus counters and `as usize` for limits. All are on values that are small in practice (batch size ≤ 1000 events, microsecond durations << 2^63, byte lengths << 2^32 on 64-bit).

#### src/ingest.rs:543,568,877
```rust
543|                violation_count: vr.violations.len() as i32,
549|                validation_us: vr.validation_us as i64,
877|    let violation_count = failed_indices.len() as i32,
```
**Context:** Batch ingest success + quarantine paths, and wholesale quarantine writer. Also used by v1_ingest (647,666,758) and egress (492,514).
**Why it might matter:** If the 1 MB / 10 MB body limit is ever raised or an adversary sends a huge number of tiny records that each produce many violations, the count can truncate. The batch size guard (ingest.rs:276) currently makes this impossible, but the cast is still lossy in type.
**Tag:** [request-path]

#### src/validation.rs:364,410,511,530,544,574
```rust
364|    let validation_us = t0.elapsed().as_micros() as u64;
410|    ...
544|            validation_us: t0.elapsed().as_micros() as u64,
```
**Context:** Every validation path (single, batch, envelope, wrapper). Result flows to audit rows (as i64 in storage) and metrics.
**Why it might matter:** as_micros() is u128; cast to u64 loses nothing for any realistic validation (< 1 second). Still a narrowing cast on request-derived timing.
**Tag:** [request-path]

#### src/main.rs:297
```rust
296|    let summaries = storage::list_versions(db, id.id, None).await?;
297|    let version_count = summaries.len() as i64;
```
**Context (get_contract_handler):** Returned in ContractResponse.
**Why it might matter:** Number of versions for one contract will never approach 2^31 in any realistic system.
**Tag:** [request-path] [benign]

#### src/rate_limit.rs:78,81
```rust
78|            ((deficit / self.rate) * 1_000.0).ceil() as u64
81|        (allowed, self.tokens.floor() as u32, reset_ms)
```
**Context (TokenBucket::take):** reset_ms and remaining tokens returned in rate-limit headers.
**Why it might matter:** tokens start at burst (≤ 10k in practice); floor as u32 cannot truncate. Reset calc is fine.
**Tag:** [request-path] [benign]

#### src/api_key_auth.rs:336
```rust
336|        rate_limit_rps: row.rate_limit_rps.map(|v| v as u32),
337|        rate_limit_burst: row.rate_limit_burst.map(|v| v as u32),
```
**Context (row mapping):** DB i32 → internal u32. CHECK constraint guarantees >0.
**Tag:** [request-path] [benign]

#### src/odcs.rs:153,158,645,648 and src/validation.rs:957
Various `v as u64` / `n as usize` / `as i64` on length/age fields parsed from ODCS YAML or freshness rules. All come from user-controlled contract documents but are small integers (field counts, seconds).
**Tag:** [request-path] [benign]

#### Other minor casts (metrics, replay, storage, bin/demo, infer_url octets)
All `len() as u64`, `as_millis() as u64`, octet extraction `(s[6] >> 8) as u8` (fixed-size), etc. None are on unbounded request values that could realistically wrap.
**Tag:** [request-path] [benign] or [startup] for demo-seeder.

---

## Summary Table

| Category                  | Total Hits (prod) | [request-path] | [startup] | [benign] |
|---------------------------|-------------------|----------------|-----------|----------|
| Pattern 1: Panics         | 9                 | 5              | 4         | 4        |
| Pattern 2: Swallowed Errors | 12              | 10             | 0         | 12       |
| Pattern 3: Secrets/PII in Logs | 0            | 0              | 0         | 0 (clean)|
| Pattern 4: Lossy Numeric Casts | 27           | 18             | 2         | 25       |
| **TOTAL**                 | **48**            | **33**         | **6**     | **41**   |

---

## Top 5 Priorities for Human Review

1. **Four identical `.expect("... check guarantees org_id is set")` in collaboration.rs:186,262,329,356**  
   All four collaboration mutation handlers (grant, comment, proposal create, decide) assume that a successful `require_role` call implies `org_id.is_some()`. A single future change to `org_id_from_req` or the dev_no_auth path could turn an authenticated request into a worker panic. Highest leverage item in the scan.

2. **transform.rs:173,196,219 — defensive `.expect` on HMAC and UTF-8 in the RFC-004 hot path**  
   These three sites are executed on every ingest or egress that uses `kind: hash` or `format_preserving`. Documented as impossible today, but they are the only remaining panic surface on the core validation/transform path that handles customer event data.

3. **publication.rs:184 — `.expect("revoke always sets revoked_at")` after storage call**  
   Post-condition on a DB write that is not enforced by the type system or a RETURNING clause in a way that the compiler can see. A data-integrity bug here would panic a revoke request that had already succeeded at the DB layer.

4. **All `violations.len() as i32` (ingest, egress, v1_ingest, replay, kinesis) and `validation_us as i64`**  
   The casts are currently safe only because of the 1 MB / 10 MB body caps and the 1000-record batch limit. If either limit is raised (or an NDJSON stream with millions of tiny records is ever allowed), the counts truncate silently before hitting the audit/quarantine tables.

5. **Guarded but still-present direct indexing (`events[0]`, `addrs[0]`, `ordinal_for_id.get(...).unwrap()`)**  
   ingest:341, infer_url:252, replay:419/436. All are preceded by checks today, but they are the exact shape the prompt asked to flag. A one-line refactor to `.first().ok_or(...)` or `get().ok_or(...)` would remove the entire class of theoretical panics.

---

**End of findings.** All analysis derived from static grep + manual context reads. No execution, no suggestions for patches.
