/**
 * Unit tests for the pure `assertGuardedChange` guard (RFC-085).
 *
 * No test runner is wired into this project yet (no jest/vitest config),
 * so this is a plain assert-based script against `./roleGuard` (which has
 * zero imports, so it needs no path-alias setup to run), compiled and run
 * with `node` directly:
 *
 *   npx tsc lib/org/roleGuard.ts lib/org/authz.test.ts --outDir /tmp/authz-test \
 *     --module commonjs --target es2020 --esModuleInterop --skipLibCheck
 *   node /tmp/authz-test/lib/org/authz.test.js
 */

import assert from "node:assert";
import { assertGuardedChange } from "./roleGuard";

// Admin may not change an owner's role.
assert.deepStrictEqual(
  assertGuardedChange({
    callerRole: "admin",
    targetCurrentRole: "owner",
    targetNewRole: "admin",
    liveOwnerCount: 3,
  }),
  { ok: false, status: 403, error: "only an owner can manage owners" }
);

// Admin may not promote someone to owner.
assert.deepStrictEqual(
  assertGuardedChange({
    callerRole: "admin",
    targetCurrentRole: "member",
    targetNewRole: "owner",
    liveOwnerCount: 3,
  }),
  { ok: false, status: 403, error: "only an owner can manage owners" }
);

// Owner demoting the last remaining owner is blocked.
assert.deepStrictEqual(
  assertGuardedChange({
    callerRole: "owner",
    targetCurrentRole: "owner",
    targetNewRole: "admin",
    liveOwnerCount: 1,
  }),
  { ok: false, status: 409, error: "cannot_remove_last_owner" }
);

// Owner removing (DELETE, no targetNewRole) the last remaining owner is blocked.
assert.deepStrictEqual(
  assertGuardedChange({
    callerRole: "owner",
    targetCurrentRole: "owner",
    targetNewRole: undefined,
    liveOwnerCount: 1,
  }),
  { ok: false, status: 409, error: "cannot_remove_last_owner" }
);

// Owner demoting an owner is fine when another owner remains.
assert.deepStrictEqual(
  assertGuardedChange({
    callerRole: "owner",
    targetCurrentRole: "owner",
    targetNewRole: "admin",
    liveOwnerCount: 2,
  }),
  { ok: true }
);

// Owner promoting an admin to owner is always fine (never reduces the count).
assert.deepStrictEqual(
  assertGuardedChange({
    callerRole: "owner",
    targetCurrentRole: "admin",
    targetNewRole: "owner",
    liveOwnerCount: 1,
  }),
  { ok: true }
);

// Admin managing a member (no owner involved) is fine.
assert.deepStrictEqual(
  assertGuardedChange({
    callerRole: "admin",
    targetCurrentRole: "member",
    targetNewRole: "admin",
    liveOwnerCount: 2,
  }),
  { ok: true }
);

console.log("authz.test.ts: all assertions passed");
