import assert from "node:assert/strict";
import test from "node:test";
import { createLatestRequestTracker } from "./latest-request";

test("only the most recently started request is current", () => {
  const tracker = createLatestRequestTracker();
  const first = tracker.start();
  const second = tracker.start();
  assert.notEqual(first, second);
  assert.equal(tracker.isCurrent(first), false);
  assert.equal(tracker.isCurrent(second), true);
});

test("an out-of-order resolution is recognized as stale", () => {
  // Mirrors selecting v1.0.0 then v1.0.1 before v1.0.0's response arrives:
  // the first request is started first but its response is checked last.
  const tracker = createLatestRequestTracker();
  const v1 = tracker.start();
  const v2 = tracker.start();
  // v2's response resolves first.
  assert.equal(tracker.isCurrent(v2), true);
  // v1's response resolves after v2's; it must not win.
  assert.equal(tracker.isCurrent(v1), false);
});

test("a single in-flight request stays current until replaced", () => {
  const tracker = createLatestRequestTracker();
  const only = tracker.start();
  assert.equal(tracker.isCurrent(only), true);
  assert.equal(tracker.isCurrent(only), true);
});
