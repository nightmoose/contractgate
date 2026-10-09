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
      { id: "dddddddd-dddd-dddd-dddd-dddddddddddd", name: "my_events", latest_stable_version: null },
    ]);

  const rows = await appTester(App.triggers.contract.operation.perform, {
    authData: { api_key: "cg_live_test" },
    inputData: {},
  });
  assert.deepEqual(rows, [
    { id: "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa", name: "salesforce_lead (1.0.0)" },
    { id: "dddddddd-dddd-dddd-dddd-dddddddddddd", name: "my_events (draft only: deploy a version first)" },
  ]);
});

test("connection test fails when the Gateway URL serves a web page", async () => {
  nock(BASE).get("/contracts").reply(200, "<!DOCTYPE html><html>dashboard</html>", {
    "Content-Type": "text/html",
  });

  await assert.rejects(
    () => appTester(App.authentication.test, { authData: { api_key: "cg_live_test" } }),
    /did not answer like the ContractGate API/,
  );
});

test("connection test counts contracts", async () => {
  nock(BASE).get("/contracts").reply(200, [{ id: "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa", name: "a" }]);
  const result = await appTester(App.authentication.test, { authData: { api_key: "cg_live_test" } });
  assert.deepEqual(result, { contract_count: 1 });
});

test("connection test with a bad key asks to reconnect", async () => {
  nock(BASE).get("/contracts").reply(401, { error: "unauthorized" });
  await assert.rejects(
    () => appTester(App.authentication.test, { authData: { api_key: "cg_live_bad" } }),
    (err) => err.name === "ExpiredAuthError",
  );
});
