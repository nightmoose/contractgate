# ContractGate — Haiku Execution Prompts

**Companion to:** `TEST-PLAN-2026-05-24-public-launch.md`
**Date:** 2026-05-24
**Executor:** a Claude Haiku agent running live against production.

## How to use

Each prompt below is self-contained. Paste one prompt at a time into a fresh
Haiku conversation. Before pasting, replace every `<…>` placeholder with the
real value. Haiku runs the commands, then reports a results table.

Run the prompts in order. Prompt 0 is a connectivity gate — if it fails, fix
credentials before continuing. Prompts 1–8 map one-to-one to the eight test
suites; each creates its own test data so they can be run independently.

**Credentials (same for every prompt):**

```
BASE       = https://contractgate-api.fly.dev
DASHBOARD  = https://app.datacontractgate.com
KEY_A      = cg_live_2e2463c06dbec6ac3ea404336383fe16af97526d750e7c57
KEY_B      = cg_live_b1ca199013507a9f4e7b009ab9dea7b7ce26198643c698aa
JWT        = <Supabase session token, optional — only used by test 2.9>
CG_BIN     = <path to pre-built contractgate binary — only used by Suite 7>
LOGIN_A    = <dashboard email + password — only used by Suite 8>
```

KEY_A and KEY_B are pre-provisioned test keys. They belong to two dedicated,
empty, isolated orgs — **QA Test Org A** and **QA Test Org B** — created only
for this test run, so the tests touch no real customer data. The keys are
live (`cg_live_…`) DB-backed keys on `free` plan; rate limits are unset
(defaults apply). After testing, both orgs can be deleted to remove every
test artifact in one step.

Every prompt ends by asking for a results table:
`ID | Test | Expected | Observed | Pass/Fail`, plus a one-line count and the
raw response for any failure.

**Critical for the executor:** shell calls do **not** share variables or
working directory between invocations. Run all the shell commands inside a
single prompt as **one bash script in one call**. Echo a marker line such as
`=== 2.3 ===` before each test so its output can be attributed. Each prompt's
commands are written top-to-bottom in run order — paste them as one script.

---

## Prompt 0 — Connectivity & credential check

```text
You are QA-testing the ContractGate production API. This is a connectivity gate
before the real test suites. Use the bash tool with curl. Run the commands
below in ONE bash call — shell variables do not persist between calls.

Set these variables (replace the placeholders):
  BASE=https://contractgate-api.fly.dev
  KEY_A=<Org A DB-backed API key>
  KEY_B=<Org B DB-backed API key, different org>

Run each command and record the HTTP status:
  1. curl -s -w "\n%{http_code}\n" "$BASE/health"
  2. curl -s -o /dev/null -w "%{http_code}\n" "$BASE/ready"
  3. curl -s -o /dev/null -w "%{http_code}\n" -H "x-api-key: $KEY_A" "$BASE/contracts"
  4. curl -s -o /dev/null -w "%{http_code}\n" -H "x-api-key: $KEY_B" "$BASE/contracts"

Expected: (1) 200 with "status":"ok"; (2) 200; (3) 200; (4) 200.

Report a 4-row table: check | observed status | Pass/Fail.
If ANY row fails, stop and tell me exactly which credential or endpoint is bad.
Do not continue to other suites.
```

---

## Prompt 1 — Suite 1: Infrastructure & health

```text
You are QA-testing the ContractGate production API. Run Suite 1 (infrastructure
and health). Use the bash tool with curl. Run the commands below in ONE bash
call.

  BASE=https://contractgate-api.fly.dev

Run these and check each result:

1.1 GET /health
    curl -s -w "\n%{http_code}\n" "$BASE/health"
    PASS if status 200 AND body contains "status":"ok".

1.2 GET /ready
    curl -s -o /dev/null -w "%{http_code}\n" "$BASE/ready"
    PASS if status 200.

1.3 GET /metrics
    curl -s -w "\n%{http_code}\n" "$BASE/metrics" | tail -20
    PASS if 200 AND body looks like Prometheus text (lines with metric names).

1.4 GET /openapi.json
    curl -s -w "\n%{http_code}\n" "$BASE/openapi.json" | head -5
    PASS if 200 AND body is JSON containing the word "openapi".

1.5 GET /catalog (public route, no auth)
    curl -s -w "\n%{http_code}\n" "$BASE/catalog"
    PASS if 200 AND body is a JSON array.

Report: table ID | Test | Expected | Observed status | Pass/Fail.
Then "Suite 1: N passed, M failed". Show raw response for any failure.
```

---

## Prompt 2 — Suite 2: Authentication & security regressions

This is the highest-priority suite. It regression-tests every launch-blocker fix.

```text
You are QA-testing the ContractGate production API security fixes. Run Suite 2.
Use the bash tool with curl. Be precise: status codes matter exactly. Run
every command below in ONE bash call — shell variables do not persist between
calls.

  BASE=https://contractgate-api.fly.dev
  KEY_A=<Org A DB-backed API key>
  KEY_B=<Org B DB-backed API key, DIFFERENT org>
  JWT=<Supabase session token, or leave blank to skip 2.9>

2.1 No API key:
    curl -s -o /dev/null -w "%{http_code}\n" "$BASE/contracts"
    PASS if 401.

2.2 Malformed key:
    curl -s -o /dev/null -w "%{http_code}\n" -H "x-api-key: cg_live_bogus" "$BASE/contracts"
    PASS if 401.

Setup for 2.3-2.5 — create a contract owned by Org A. NOTE: POST /contracts
needs BOTH a top-level "name" and "yaml_content".
    TS=$(date +%s); NAME="qa_sec_$TS"
    cat > /tmp/qa_sec.yaml <<EOF
version: "1.0"
name: "$NAME"
description: "QA security test contract"

ontology:
  entities:
    - name: user_id
      type: string
      required: true
EOF
    BODY=$(python3 -c "import json;print(json.dumps({'name':'$NAME','yaml_content':open('/tmp/qa_sec.yaml').read()}))")
    RESP=$(curl -s -X POST "$BASE/contracts" -H "x-api-key: $KEY_A" -H "Content-Type: application/json" -d "$BODY")
    echo "$RESP"
    CID=$(echo "$RESP" | python3 -c 'import json,sys;print(json.load(sys.stdin)["id"])')
    echo "Contract ID: $CID"

2.3 Cross-org READ (RFC-047 IDOR fix):
    curl -s -o /dev/null -w "B=%{http_code}\n" -H "x-api-key: $KEY_B" "$BASE/contracts/$CID"
    curl -s -o /dev/null -w "A=%{http_code}\n" -H "x-api-key: $KEY_A" "$BASE/contracts/$CID"
    PASS if KEY_B returns 404 AND KEY_A returns 200.
    FAIL HARD if KEY_B returns 200 or 403 — that is a tenant-isolation breach.

2.4 Cross-org WRITE (RFC-047):
    curl -s -o /dev/null -w "patch=%{http_code}\n" -X PATCH -H "x-api-key: $KEY_B" -H "Content-Type: application/json" -d '{"description":"hacked"}' "$BASE/contracts/$CID"
    curl -s -o /dev/null -w "delete=%{http_code}\n" -X DELETE -H "x-api-key: $KEY_B" "$BASE/contracts/$CID"
    curl -s -w "\n%{http_code}\n" -H "x-api-key: $KEY_A" "$BASE/contracts/$CID"
    PASS if PATCH and DELETE from KEY_B both return 404 AND the KEY_A re-read
    still returns 200 with the contract intact (description NOT "hacked").

2.5 x-org-id header is ignored (RFC-048):
    curl -s -o /dev/null -w "%{http_code}\n" -H "x-api-key: $KEY_A" -H "x-org-id: 00000000-0000-0000-0000-000000000000" "$BASE/contracts/$CID"
    PASS if 200 — the forged x-org-id header has no effect; org comes from the key.

2.6 CORS — disallowed origin (RFC-050):
    curl -s -D - -o /dev/null -X OPTIONS "$BASE/contracts" -H "Origin: https://evil.example.com" -H "Access-Control-Request-Method: GET" | grep -i "access-control-allow-origin" || echo "NO ACAO HEADER"
    PASS if there is NO access-control-allow-origin header echoing evil.example.com.

2.7 CORS — allowed origin (RFC-050):
    curl -s -D - -o /dev/null -X OPTIONS "$BASE/contracts" -H "Origin: https://app.datacontractgate.com" -H "Access-Control-Request-Method: GET" | grep -i "access-control-allow"
    curl -s -D - -o /dev/null "$BASE/health" -H "Origin: https://anything.example.com" | grep -i "access-control-allow-origin"
    PASS if /contracts echoes access-control-allow-origin for app.datacontractgate.com
    AND the public /health route returns access-control-allow-origin: *.

2.8 SSRF block (RFC-049) — the body needs "name" and "url":
    curl -s -w "\n%{http_code}\n" -X POST "$BASE/contracts/infer/url" -H "x-api-key: $KEY_A" -H "Content-Type: application/json" -d '{"name":"qa_ssrf","url":"http://169.254.169.254/latest/meta-data/"}'
    PASS if the status is 4xx AND the body is an SSRF/private-IP rejection with
    NO cloud metadata content (no "ami-id", "iam", "instance-id"). FAIL HARD if
    metadata is returned. (A 422 "missing field name" means the body was wrong,
    not a real result — re-run with the name field.)

2.9 JWT auth path (skip if JWT blank):
    curl -s -o /dev/null -w "%{http_code}\n" -H "Authorization: Bearer $JWT" "$BASE/contracts"
    PASS if 200.

Report: table ID | Test | Expected | Observed | Pass/Fail.
Then "Suite 2: N passed, M failed". For 2.3, 2.4, and 2.8 specifically, paste
the raw responses regardless of pass/fail — these are critical regression tests.
```

---

## Prompt 3 — Suite 3: Contract lifecycle (API)

```text
You are QA-testing the ContractGate production contract API. Run Suite 3.
Use the bash tool with curl and python3. Run every command below in ONE bash
call — shell variables do not persist between calls.

  BASE=https://contractgate-api.fly.dev
  KEY_A=<Org A DB-backed API key>

Setup — create a contract:
    TS=$(date +%s)
    NAME="qa_life_$TS"
    cat > /tmp/qa_life.yaml <<EOF
version: "1.0"
name: "qa_life_$TS"
description: "QA lifecycle contract"

ontology:
  entities:
    - name: user_id
      type: string
      required: true
      pattern: "^[a-zA-Z0-9_-]+$"
    - name: event_type
      type: string
      required: true
      enum: ["click", "view", "purchase", "login"]
    - name: timestamp
      type: integer
      required: true
      min: 0
EOF
    BODY=$(python3 -c "import json;print(json.dumps({'name':'$NAME','yaml_content':open('/tmp/qa_life.yaml').read()}))")

3.1 Create (POST /contracts needs BOTH "name" and "yaml_content"):
    RESP=$(curl -s -w "\n%{http_code}" -X POST "$BASE/contracts" -H "x-api-key: $KEY_A" -H "Content-Type: application/json" -d "$BODY")
    echo "$RESP"
    Extract the id: CID=$(echo "$RESP" | head -n-1 | python3 -c 'import json,sys;print(json.load(sys.stdin)["id"])')
    PASS if status 201 (or 200) AND response has "id" and "name".

3.2 List:
    curl -s -H "x-api-key: $KEY_A" "$BASE/contracts" | python3 -c 'import json,sys; d=json.load(sys.stdin); print("found" if any("qa_life" in json.dumps(x) for x in (d if isinstance(d,list) else d.get("contracts",[]))) else "missing")'
    PASS if the new contract appears in the list.

3.3 Get by id:
    curl -s -o /dev/null -w "%{http_code}\n" -H "x-api-key: $KEY_A" "$BASE/contracts/$CID"
    PASS if 200.

3.4 Patch:
    curl -s -o /dev/null -w "%{http_code}\n" -X PATCH -H "x-api-key: $KEY_A" -H "Content-Type: application/json" -d '{"description":"QA lifecycle contract - edited"}' "$BASE/contracts/$CID"
    curl -s -H "x-api-key: $KEY_A" "$BASE/contracts/$CID" | grep -o "edited" || echo "not updated"
    PASS if PATCH returns 200 AND the re-read shows "edited".

3.5 List versions:
    curl -s -w "\n%{http_code}\n" -H "x-api-key: $KEY_A" "$BASE/contracts/$CID/versions"
    PASS if 200 AND at least one version is returned.

3.6 Deploy a contract (RFC-028 atomic deploy):
    The /contracts/deploy body needs name + yaml_content (name must match the
    YAML's name field). It returns 201 on success.
    DBODY=$(python3 -c "import json;print(json.dumps({'name':'$NAME','yaml_content':open('/tmp/qa_life.yaml').read(),'source':'qa','deployed_by':'haiku-qa'}))")
    curl -s -w "\n%{http_code}\n" -X POST "$BASE/contracts/deploy" -H "x-api-key: $KEY_A" -H "Content-Type: application/json" -d "$DBODY"
    PASS if 200 or 201 AND the response has "version" and "deployed_at"
    (a stable version was deployed).

3.7 Malformed YAML rejected:
    BADBODY=$(python3 -c 'import json;print(json.dumps({"name":"qa_bad_yaml","yaml_content":"this: is: not: valid: yaml: ["}))')
    curl -s -w "\n%{http_code}\n" -X POST "$BASE/contracts" -H "x-api-key: $KEY_A" -H "Content-Type: application/json" -d "$BADBODY"
    PASS if status is 400 (a clear parse error) — NOT 500.

3.8 Soft delete:
    curl -s -o /dev/null -w "%{http_code}\n" -X DELETE -H "x-api-key: $KEY_A" "$BASE/contracts/$CID"
    curl -s -o /dev/null -w "%{http_code}\n" -H "x-api-key: $KEY_A" "$BASE/contracts/$CID"
    PASS if DELETE returns 204 (or 200) AND the contract is gone afterward
    (the follow-up GET returns 404).

Report: table ID | Test | Expected | Observed | Pass/Fail.
Then "Suite 3: N passed, M failed". Show raw response for any failure.
```

---

## Prompt 4 — Suite 4: Validation engine & ingestion

```text
You are QA-testing the ContractGate validation engine in production. Run Suite 4.
Use the bash tool with curl and python3. Run every command below in ONE bash
call — shell variables do not persist between calls.

  BASE=https://contractgate-api.fly.dev
  KEY_A=<Org A DB-backed API key>

Setup — create the test contract WITH A STABLE VERSION. Ingestion requires a
stable version, so use POST /contracts/deploy: it creates the contract and a
stable version atomically and returns "contract_id". (A plain POST /contracts
only makes a draft, and ingesting against it returns 409.)
    TS=$(date +%s); NAME="qa_val_$TS"
    cat > /tmp/qa_val.yaml <<EOF
version: "1.0"
name: "$NAME"
description: "QA validation contract"

ontology:
  entities:
    - name: user_id
      type: string
      required: true
      pattern: "^[a-zA-Z0-9_-]+$"
    - name: event_type
      type: string
      required: true
      enum: ["click", "view", "purchase", "login"]
    - name: timestamp
      type: integer
      required: true
      min: 0
    - name: amount
      type: number
      required: false
      min: 0
EOF
    DBODY=$(python3 -c "import json;print(json.dumps({'name':'$NAME','yaml_content':open('/tmp/qa_val.yaml').read()}))")
    CID=$(curl -s -X POST "$BASE/contracts/deploy" -H "x-api-key: $KEY_A" -H "Content-Type: application/json" -d "$DBODY" | python3 -c 'import json,sys;print(json.load(sys.stdin)["contract_id"])')
    echo "Contract ID: $CID"

Define a helper:
    ingest() { curl -s -X POST "$BASE/ingest/$CID" -H "x-api-key: $KEY_A" -H "Content-Type: application/json" -d "$1"; }

4.1 Valid event:
    ingest '[{"user_id":"alice_01","event_type":"click","timestamp":1712000001}]'
    PASS if response has "passed":1 and "failed":0.

4.2 Missing required user_id:
    ingest '[{"event_type":"click","timestamp":1712000002}]'
    PASS if "failed":1.

4.3 Bad enum event_type:
    ingest '[{"user_id":"bob_2","event_type":"explode","timestamp":1712000003}]'
    PASS if "failed":1.

4.4 Bad pattern user_id:
    ingest '[{"user_id":"!nope!","event_type":"view","timestamp":1712000004}]'
    PASS if "failed":1.

4.5 Negative amount:
    ingest '[{"user_id":"carol_3","event_type":"purchase","timestamp":1712000005,"amount":-5}]'
    PASS if "failed":1.

4.6 Mixed batch (2 valid, 2 invalid):
    ingest '[{"user_id":"dave_4","event_type":"login","timestamp":1712000006},{"user_id":"","event_type":"purchase","timestamp":1712000007},{"user_id":"erin_5","event_type":"view","timestamp":1712000008},{"user_id":"frank_6","event_type":"badtype","timestamp":1712000009}]'
    PASS if "passed":2 and "failed":2.

4.7 Dry run (no DB write):
    curl -s -X POST "$BASE/ingest/$CID?dry_run=true" -H "x-api-key: $KEY_A" -H "Content-Type: application/json" -d '[{"user_id":"dry_7","event_type":"view","timestamp":1712000010}]'
    PASS if response contains "dry_run":true.

4.8 Stats:
    curl -s -w "\n%{http_code}\n" -H "x-api-key: $KEY_A" "$BASE/ingest/$CID/stats"
    PASS if 200 AND response has "total_events". Note the count.

4.9 date field type (RFC-044) — also deployed so it has a stable version:
    DNAME="qa_date_$TS"
    cat > /tmp/qa_date.yaml <<EOF
version: "1.0"
name: "$DNAME"
description: "QA date-type contract"

ontology:
  entities:
    - name: user_id
      type: string
      required: true
    - name: signup_date
      type: date
      required: true
EOF
    DATEBODY=$(python3 -c "import json;print(json.dumps({'name':'$DNAME','yaml_content':open('/tmp/qa_date.yaml').read()}))")
    DCID=$(curl -s -X POST "$BASE/contracts/deploy" -H "x-api-key: $KEY_A" -H "Content-Type: application/json" -d "$DATEBODY" | python3 -c 'import json,sys;print(json.load(sys.stdin)["contract_id"])')
    echo "valid:"   ; curl -s -X POST "$BASE/ingest/$DCID" -H "x-api-key: $KEY_A" -H "Content-Type: application/json" -d '[{"user_id":"d1","signup_date":"2026-05-24"}]'
    echo "invalid:" ; curl -s -X POST "$BASE/ingest/$DCID" -H "x-api-key: $KEY_A" -H "Content-Type: application/json" -d '[{"user_id":"d2","signup_date":"not-a-date"}]'
    PASS if the valid YYYY-MM-DD value gives "passed":1 AND the invalid value
    gives "failed":1.

4.10 Latency — 30 sequential valid ingests, then check the real server p99:
    for i in $(seq 1 30); do curl -s -o /dev/null -w "%{time_total}\n" -X POST "$BASE/ingest/$CID" -H "x-api-key: $KEY_A" -H "Content-Type: application/json" -d "[{\"user_id\":\"lat_$i\",\"event_type\":\"click\",\"timestamp\":$((1712001000+i))}]"; done | sort -n | tail -3
    curl -s -H "x-api-key: $KEY_A" "$BASE/ingest/$CID/stats"
    PASS if all 30 ingests succeed, the slowest wall-clock time is under 2s,
    AND the stats field p99_validation_us is under 15000 — that is the real
    server-side validation latency (<15ms budget), measured by the engine.

Report: table ID | Test | Expected | Observed | Pass/Fail.
Then "Suite 4: N passed, M failed". Show raw response for any failure.
```

---

## Prompt 5 — Suite 5: Inference & playground

```text
You are QA-testing ContractGate inference and playground endpoints in
production. Run Suite 5. Use the bash tool with curl and python3. Run every command below in ONE bash
call — shell variables do not persist between calls.

  BASE=https://contractgate-api.fly.dev
  KEY_A=<Org A DB-backed API key>

5.1 Playground — valid event:
    cat > /tmp/qa_pg.yaml <<EOF
version: "1.0"
name: "qa_pg"
description: "playground test"

ontology:
  entities:
    - name: user_id
      type: string
      required: true
    - name: event_type
      type: string
      required: true
      enum: ["click", "view"]
EOF
    PG=$(python3 -c 'import json;print(json.dumps({"yaml_content":open("/tmp/qa_pg.yaml").read(),"event":{"user_id":"t1","event_type":"click"}}))')
    curl -s -w "\n%{http_code}\n" -X POST "$BASE/playground/validate" -H "x-api-key: $KEY_A" -H "Content-Type: application/json" -d "$PG"
    PASS if 200 AND "passed":true.

5.2 Playground — invalid event:
    PGBAD=$(python3 -c 'import json;print(json.dumps({"yaml_content":open("/tmp/qa_pg.yaml").read(),"event":{"event_type":"explode"}}))')
    curl -s -w "\n%{http_code}\n" -X POST "$BASE/playground/validate" -H "x-api-key: $KEY_A" -H "Content-Type: application/json" -d "$PGBAD"
    PASS if 200 AND "passed":false with violations.

5.3 Infer from JSON samples:
    curl -s -w "\n%{http_code}\n" -X POST "$BASE/contracts/infer" -H "x-api-key: $KEY_A" -H "Content-Type: application/json" -d '{"name":"qa_infer","samples":[{"id":"a1","count":3,"active":true},{"id":"a2","count":7,"active":false}]}'
    PASS if 200 AND response has "yaml_content", "field_count", "sample_count".

5.4 Infer from CSV:
    curl -s -w "\n%{http_code}\n" -X POST "$BASE/contracts/infer/csv" -H "x-api-key: $KEY_A" -H "Content-Type: application/json" -d '{"name":"qa_csv","csv_content":"id,price,name\n1,9.99,alpha\n2,19.50,beta"}'
    PASS if 200 AND response has "yaml_content".

5.5 Infer with empty samples (error handling):
    curl -s -w "\n%{http_code}\n" -X POST "$BASE/contracts/infer" -H "x-api-key: $KEY_A" -H "Content-Type: application/json" -d '{"name":"qa_empty","samples":[]}'
    PASS if status is 400 (clear error "at least one sample is required") — NOT 500.

Report: table ID | Test | Expected | Observed | Pass/Fail.
Then "Suite 5: N passed, M failed". Show raw response for any failure.
```

---

## Prompt 6 — Suite 6: Audit, stats & catalog

```text
You are QA-testing ContractGate audit, stats, and catalog endpoints in
production. Run Suite 6. Use the bash tool with curl and python3. Run every command below in ONE bash
call — shell variables do not persist between calls.

  BASE=https://contractgate-api.fly.dev
  KEY_A=<Org A DB-backed API key>
  KEY_B=<Org B DB-backed API key, DIFFERENT org>

Setup — deploy a contract (so it has a stable version) and ingest one valid +
one invalid event so audit rows exist:
    TS=$(date +%s); NAME="qa_aud_$TS"
    cat > /tmp/qa_aud.yaml <<EOF
version: "1.0"
name: "$NAME"
description: "QA audit contract"

ontology:
  entities:
    - name: user_id
      type: string
      required: true
    - name: event_type
      type: string
      required: true
      enum: ["click", "view"]
EOF
    DBODY=$(python3 -c "import json;print(json.dumps({'name':'$NAME','yaml_content':open('/tmp/qa_aud.yaml').read()}))")
    CID=$(curl -s -X POST "$BASE/contracts/deploy" -H "x-api-key: $KEY_A" -H "Content-Type: application/json" -d "$DBODY" | python3 -c 'import json,sys;print(json.load(sys.stdin)["contract_id"])')
    curl -s -X POST "$BASE/ingest/$CID" -H "x-api-key: $KEY_A" -H "Content-Type: application/json" -d '[{"user_id":"ok_1","event_type":"click"}]' >/dev/null
    curl -s -X POST "$BASE/ingest/$CID" -H "x-api-key: $KEY_A" -H "Content-Type: application/json" -d '[{"user_id":"bad_1","event_type":"explode"}]' >/dev/null
    echo "Contract ID: $CID"

6.1 Audit log:
    curl -s -w "\n%{http_code}\n" -H "x-api-key: $KEY_A" "$BASE/audit?contract_id=$CID&limit=20"
    PASS if 200 AND entries exist for this contract.

6.2 Audit honesty (CRITICAL): inspect the audit entries from 6.1.
    Look at the contract_version field on each entry.
    PASS if every entry's contract_version is a real version string for this
    contract — NOT null, NOT empty, NOT a placeholder/default.
    Paste the contract_version values you observe.

6.3 Global stats:
    curl -s -w "\n%{http_code}\n" -H "x-api-key: $KEY_A" "$BASE/stats"
    PASS if 200 AND org-scoped totals are returned.

6.4 Public catalog (no auth):
    curl -s -o /dev/null -w "list=%{http_code}\n" "$BASE/public-contracts"
    Then take the first id from: curl -s "$BASE/public-contracts" | python3 -c 'import json,sys;d=json.load(sys.stdin);print((d if isinstance(d,list) else d.get("contracts",[]))[0]["id"])'
    And fetch it: curl -s -o /dev/null -w "get=%{http_code}\n" "$BASE/public-contracts/<that-id>"
    PASS if the list returns 200 and fetching one returns 200. (If the catalog
    is empty, mark as PASS-with-note.)

6.5 Audit org isolation (CRITICAL):
    curl -s -H "x-api-key: $KEY_B" "$BASE/audit?contract_id=$CID&limit=20"
    PASS if Org B sees NO audit rows for Org A's contract (empty result or 404).
    FAIL HARD if Org B can read Org A's audit entries.

Report: table ID | Test | Expected | Observed | Pass/Fail.
Then "Suite 6: N passed, M failed". For 6.2 and 6.5 paste the raw responses.
```

---

## Prompt 7 — Suite 7: CLI

```text
You are QA-testing the ContractGate CLI against production. Run Suite 7.
Use the bash tool. The CLI binary is already built. Run the commands below in
ONE bash call — shell variables do not persist between calls.

  BASE=https://contractgate-api.fly.dev
  CG=<path to the contractgate binary>
  export CONTRACTGATE_API_KEY=<Org A DB-backed API key>

7.1 Version & help:
    "$CG" --version
    "$CG" --help
    PASS if a version prints AND help lists the subcommands: deploy-contract,
    push, pull, validate, scaffold, enforce, infer.

7.2 Validate a good contract:
    cat > /tmp/cli_good.yaml <<EOF
version: "1.0"
name: "qa_cli_good"
description: "good contract"

ontology:
  entities:
    - name: user_id
      type: string
      required: true
EOF
    "$CG" validate /tmp/cli_good.yaml; echo "exit=$?"
    PASS if exit code 0 and it reports the contract is valid.

7.3 Validate a bad contract:
    printf 'version: "1.0"\nname:\n  bad: [unclosed\n' > /tmp/cli_bad.yaml
    "$CG" validate /tmp/cli_bad.yaml; echo "exit=$?"
    PASS if exit code is non-zero and an error is printed.

7.4 Infer from stdin:
    echo '{"id":"x1","score":42,"active":true}' | "$CG" infer --from-stdin --name qa_cli_users; echo "exit=$?"
    PASS if exit 0 and draft contract YAML is emitted.

7.5 Scaffold a draft contract from a local sample file:
    echo '[{"id":"s1","qty":5},{"id":"s2","qty":9}]' > /tmp/cli_samples.json
    "$CG" scaffold --from-file /tmp/cli_samples.json --name qa_cli_events; echo "exit=$?"
    PASS if exit 0 and a draft contract YAML is produced.

7.6 Deploy dry-run (uses CONTRACTGATE_API_KEY):
    The CLI finds .contractgate.yml by walking up from the working directory,
    so create the config and run from the same folder in one command:
      mkdir -p /tmp/cgqa && cp /tmp/cli_good.yaml /tmp/cgqa/ && printf 'version: "1.0"\ngateway:\n  url: %s\n' "$BASE" > /tmp/cgqa/.contractgate.yml && cd /tmp/cgqa && "$CG" deploy-contract cli_good.yaml --dry-run; echo "exit=$?"
    PASS if exit 0 and the dry-run reports what would deploy without writing.

7.7 Missing key is rejected:
    cd /tmp/cgqa && env -u CONTRACTGATE_API_KEY "$CG" deploy-contract cli_good.yaml; echo "exit=$?"
    PASS if exit code 11 and the message says an API key is required.

Report: table ID | Test | Expected | Observed (exit code + key output) | Pass/Fail.
Then "Suite 7: N passed, M failed". Show output for any failure.
```

---

## Prompt 8 — Suite 8: Dashboard UI

```text
You are QA-testing the ContractGate dashboard in production. Run Suite 8.

This is a pure browser task. Do NOT invoke any skills or slash commands
(no setup-cowork, no others) — use ONLY the Claude-in-Chrome browser MCP
tools (navigate, read page, click, type, screenshot). If those browser tools
are not available, the Chrome extension is not connected — say so and stop.

  DASHBOARD=https://app.datacontractgate.com
  Login email: <paste the dashboard email here>
  Login password: <paste the dashboard password here>

8.1 Sign in:
    Navigate to https://app.datacontractgate.com/auth/login
    Enter the email and password, submit.
    PASS if you land on an authenticated page and there are no console errors.

8.2 Contracts page:
    Navigate to /contracts.
    PASS if the contract list renders and search/filter controls are present.

8.3 Playground:
    Navigate to /playground. Paste a small contract and a matching event,
    run validation.
    PASS if a pass/fail result appears in the UI.

8.4 Catalog:
    Navigate to /catalog.
    PASS if the public catalog renders and a contract can be opened.

8.5 API key issuance (RFC-056) — CRITICAL:
    Navigate to /account. Find the API Keys section. Issue a new key.
    PASS if a new key is shown exactly once. Note whether the page warns the
    key cannot be retrieved again. (Server-side issuance — the browser must not
    generate the key itself.)

8.6 Revoke the key:
    Revoke the key created in 8.5.
    PASS if it is marked revoked in the list.

8.7 Other pages load:
    Navigate to /audit and /scorecard.
    PASS if both render without errors.

8.8 Plan gating (RFC-045):
    As the current user, open a Growth-tier feature (Visual Builder, the
    From CSV tab, or GitHub sync).
    PASS if either the feature works (user is Growth+) or an upsell card is
    shown instead (user is Free) — in both cases, no crash.

8.9 CORS in practice (RFC-050):
    While signed in, open the browser dev console and network tab. Confirm the
    dashboard's calls to the API host (contractgate-api.fly.dev) return data.
    PASS if authenticated API calls succeed and the console shows no
    "blocked by CORS policy" errors.

Take a screenshot of every page you visit.

Report: table ID | Test | Expected | Observed | Pass/Fail.
Then "Suite 8: N passed, M failed". Describe any failure and attach the
screenshots.
```

---

## Prompt 9 — Final roll-up (optional)

```text
Compile the results of Suites 1-8 of the ContractGate launch test run.

Produce:
1. A summary table: Suite | Passed | Failed | Notable failures.
2. Total passed / total failed.
3. A LAUNCH VERDICT:
   - All P0 tests passed  -> "P0 clear".
   - Any P0 failed        -> "BLOCKED" and list which.
   P0 tests are: 1.1, 1.2, 2.1-2.8, 3.1-3.3, 4.1-4.6, 6.2, 6.5, 8.1, 8.5.
4. A list of every P1/P2 failure as follow-up items.

Keep it under 300 words.
```
