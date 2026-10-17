import assert from "node:assert/strict";
import test from "node:test";
import { paidFeatureAccess, tierOpen } from "./paid-features";

test("paid_features true opens the gate when the stored plan is free", () => {
  assert.equal(paidFeatureAccess(true, false), true);
});

test("paid_features false keeps the gate closed when the stored plan is paid", () => {
  assert.equal(paidFeatureAccess(false, true), false);
});

test("a missing paid_features field falls back to the stored plan", () => {
  assert.equal(paidFeatureAccess(undefined, true), true);
  assert.equal(paidFeatureAccess(undefined, false), false);
  assert.equal(paidFeatureAccess(null, true), true);
  assert.equal(paidFeatureAccess(null, false), false);
});

test("self-host opens pro and enterprise gates", () => {
  assert.equal(tierOpen(true, true, false, false), true);
  assert.equal(tierOpen(true, true, false, true), true);
});

test("hosted pro stays open and hosted free stays closed", () => {
  assert.equal(tierOpen(false, true, true, false), true);
  assert.equal(tierOpen(false, false, false, false), false);
  assert.equal(tierOpen(false, undefined, true, false), true);
  assert.equal(tierOpen(false, undefined, false, false), false);
});

test("hosted pro does not open an enterprise gate", () => {
  assert.equal(tierOpen(false, true, false, true), false);
  assert.equal(tierOpen(false, true, true, true), true);
});
