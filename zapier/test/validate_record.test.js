const test = require("node:test");
const assert = require("node:assert/strict");
const nock = require("nock");
const zapier = require("zapier-platform-core");
const App = require("../index");

const appTester = zapier.createAppTester(App);
const BASE = "https://app.datacontractgate.com";
const CONTRACT = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa";

function bundle(inputData) {
  return {
    authData: { api_key: "cg_live_test" },
    inputData,
  };
}

function passBody(record) {
  return {
    total: 1,
    passed: 1,
    failed: 0,
    dry_run: false,
    resolved_version: "1.0.0",
    results: [
      {
        index: 0,
        passed: true,
        violations: [],
        contract_version: "1.0.0",
        quarantine_id: null,
        transformed_event: record,
      },
    ],
  };
}

test.afterEach(() => {
  nock.cleanAll();
});

test("a passing record returns the payload and does not halt", async () => {
  const record = { email: "ada@example.com", status: "new" };
  const scope = nock(BASE, { reqheaders: { "x-api-key": "cg_live_test" } })
    .post(`/v1/ingest/${CONTRACT}`, record)
    .reply(200, passBody(record));

  const result = await appTester(
    App.creates.validate_record.operation.perform,
    bundle({ contract_id: CONTRACT, record: JSON.stringify(record) }),
  );

  assert.equal(scope.isDone(), true);
  assert.equal(result.passed, true);
  assert.deepEqual(result.payload, record);
  assert.equal(result.contract_version, "1.0.0");
});

test("a rejected record is quarantined and halts the Zap", async () => {
  const record = { email: "" };
  nock(BASE)
    .post(`/v1/ingest/${CONTRACT}`, record)
    .reply(422, {
      total: 1,
      passed: 0,
      failed: 1,
      dry_run: false,
      resolved_version: "1.0.0",
      results: [
        {
          index: 0,
          passed: false,
          violations: [{ field: "email", message: "Field 'email' is required", kind: "missing_required_field" }],
          contract_version: "1.0.0",
          quarantine_id: "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb",
        },
      ],
    });

  await assert.rejects(
    () =>
      appTester(
        App.creates.validate_record.operation.perform,
        bundle({ contract_id: CONTRACT, record }),
      ),
    (err) => {
      assert.equal(err.name, "HaltedError");
      assert.match(err.message, /email/);
      assert.match(err.message, /bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb/);
      return true;
    },
  );
});

test("halt off returns the failure so a Path can branch, with a null payload", async () => {
  const record = { email: "" };
  nock(BASE)
    .post(`/v1/ingest/${CONTRACT}?dry_run=true&version=1.2.0`)
    .reply(422, {
      total: 1,
      passed: 0,
      failed: 1,
      dry_run: true,
      resolved_version: "1.2.0",
      results: [
        {
          index: 0,
          passed: false,
          violations: [{ field: "email", message: "required" }],
          contract_version: "1.2.0",
          quarantine_id: null,
        },
      ],
    });

  const result = await appTester(
    App.creates.validate_record.operation.perform,
    bundle({
      contract_id: CONTRACT,
      record: JSON.stringify(record),
      halt_on_failure: "false",
      dry_run: "true",
      version: "1.2.0",
    }),
  );

  assert.equal(result.passed, false);
  assert.equal(result.payload, null);
  assert.equal(result.dry_run, true);
  assert.match(result.violations_summary, /email: required/);
});

test("an expired API key asks the user to reconnect", async () => {
  nock(BASE).post(`/v1/ingest/${CONTRACT}`).reply(401, { error: "unauthorized" });

  await assert.rejects(
    () =>
      appTester(
        App.creates.validate_record.operation.perform,
        bundle({ contract_id: CONTRACT, record: { email: "a@b.c" } }),
      ),
    (err) => {
      assert.equal(err.name, "ExpiredAuthError");
      return true;
    },
  );
});

test("idempotency key is forwarded", async () => {
  const record = { id: "lead_1" };
  const scope = nock(BASE, { reqheaders: { "idempotency-key": "lead_1" } })
    .post(`/v1/ingest/${CONTRACT}`, record)
    .reply(200, passBody(record));

  await appTester(
    App.creates.validate_record.operation.perform,
    bundle({
      contract_id: CONTRACT,
      record: JSON.stringify(record),
      idempotency_key: "lead_1",
    }),
  );
  assert.equal(scope.isDone(), true);
});

test("loading a sample does not call the gateway", async () => {
  const scope = nock(BASE).post(/.*/).reply(500);
  const result = await appTester(App.creates.validate_record.operation.perform, {
    authData: { api_key: "cg_live_test" },
    inputData: {},
    meta: { isLoadingSample: true },
  });
  assert.equal(result.passed, true);
  assert.equal(scope.isDone(), false);
});
