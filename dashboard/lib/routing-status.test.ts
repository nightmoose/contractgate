import assert from "node:assert/strict";
import test from "node:test";
import { routingStatus } from "./routing-status";

test("no contract, or no promoted version, means no stable route", () => {
  assert.equal(routingStatus(null).hasStable, false);
  assert.equal(routingStatus({ latest_stable_version: null }).hasStable, false);
});

test("a promoted version is read straight from the contract", () => {
  const status = routingStatus({
    latest_stable_version: "1.2.0",
    multi_stable_resolution: "strict",
  });
  assert.equal(status.hasStable, true);
  assert.equal(status.version, "1.2.0");
  assert.equal(status.resolution, "strict");
});

test("missing multi_stable_resolution defaults to strict", () => {
  const status = routingStatus({ latest_stable_version: "1.0.0" });
  assert.equal(status.resolution, "strict");
});

test("fallback resolution is preserved", () => {
  const status = routingStatus({
    latest_stable_version: "2.0.0",
    multi_stable_resolution: "fallback",
  });
  assert.equal(status.resolution, "fallback");
});
