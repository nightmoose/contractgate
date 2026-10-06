const test = require("node:test");
const assert = require("node:assert/strict");
const { asBool, parseRecord, summarize } = require("../lib/record");
const { baseUrl } = require("../lib/base_url");

test("parseRecord accepts an object or a JSON string", () => {
  const obj = { email: "ada@example.com" };
  assert.deepEqual(parseRecord(obj), obj);
  assert.deepEqual(parseRecord(' {"email":"ada@example.com"} '), obj);
});

test("parseRecord rejects a list, text, and [object Object]", () => {
  assert.throws(() => parseRecord("[1]"), /one JSON object/);
  assert.throws(() => parseRecord("not json"), /not valid JSON/);
  assert.throws(() => parseRecord("[object Object]"), /Code by Zapier/);
  assert.throws(() => parseRecord(""), /empty/);
});

test("asBool defaults and coerces Zapier checkbox strings", () => {
  assert.equal(asBool(undefined, true), true);
  assert.equal(asBool("false", true), false);
  assert.equal(asBool("true", false), true);
  assert.equal(asBool(false, true), false);
});

test("summarize uses field and message", () => {
  assert.equal(
    summarize([{ field: "email", message: "required" }]),
    "email: required",
  );
  assert.equal(summarize([]), "");
});

test("baseUrl defaults to cloud and rejects junk", () => {
  assert.equal(baseUrl({}), "https://app.datacontractgate.com");
  assert.equal(baseUrl({ base_url: "http://localhost:3001/" }), "http://localhost:3001");
  assert.throws(() => baseUrl({ base_url: "ftp://nope" }), /https/);
});
