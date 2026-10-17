import assert from "node:assert/strict";
import test from "node:test";
import { paidFeatureAccess } from "./paid-features";

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
