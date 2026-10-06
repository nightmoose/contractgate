const test = require("node:test");
const assert = require("node:assert/strict");
const nock = require("nock");
const zapier = require("zapier-platform-core");
const App = require("../index");

const appTester = zapier.createAppTester(App);
const BASE = "https://app.datacontractgate.com";

test.afterEach(() => nock.cleanAll());

test("new quarantined record polls newest rows and keeps the id", async () => {
  const scope = nock(BASE)
    .get("/quarantine")
    .query({ limit: "100", contract_id: "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa" })
    .reply(200, [
      {
        id: "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb",
        contract_id: "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa",
        contract_version: "1.0.0",
        raw_event: { email: "" },
        violation_details: [{ field: "email", message: "required" }],
        violation_count: 1,
        quarantined_at: "2026-10-06T15:00:00Z",
      },
    ]);

  const rows = await appTester(App.triggers.new_quarantine.operation.perform, {
    authData: { api_key: "cg_live_test" },
    inputData: { contract_id: "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa" },
  });

  assert.equal(scope.isDone(), true);
  assert.equal(rows.length, 1);
  assert.equal(rows[0].id, "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb");
  assert.equal(rows[0].violations_summary, "email: required");
  assert.deepEqual(rows[0].record, { email: "" });
});

test("contract dropdown lists id and name", async () => {
  nock(BASE)
    .get("/contracts")
    .reply(200, [
      { id: "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa", name: "salesforce_lead", latest_stable_version: "1.0.0" },
    ]);

  const rows = await appTester(App.triggers.contract.operation.perform, {
    authData: { api_key: "cg_live_test" },
    inputData: {},
  });
  assert.deepEqual(rows, [
    { id: "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa", name: "salesforce_lead (1.0.0)" },
  ]);
});
