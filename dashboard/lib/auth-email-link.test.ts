import assert from "node:assert/strict";
import test from "node:test";
import { defaultNext, failurePath, isEmailOtpType, safeNextPath } from "./auth-email-link";

test("accepts the email otp types and nothing else", () => {
  assert.equal(isEmailOtpType("recovery"), true);
  assert.equal(isEmailOtpType("signup"), true);
  assert.equal(isEmailOtpType("magiclink"), true);
  assert.equal(isEmailOtpType("email"), true);
  assert.equal(isEmailOtpType("sms"), false);
  assert.equal(isEmailOtpType("recovery "), false);
});

test("recovery failures stay on the reset page", () => {
  assert.equal(failurePath("recovery"), "/auth/reset?error=link_invalid");
  assert.equal(failurePath("signup"), "/auth/login?error=confirmation_failed");
});

test("default next matches the email that was sent", () => {
  assert.equal(defaultNext("recovery"), "/auth/reset");
  assert.equal(defaultNext("email_change"), "/account");
  assert.equal(defaultNext("signup"), "/");
});

test("safeNextPath allows app paths and rejects open redirects", () => {
  assert.equal(safeNextPath("/auth/reset", "/"), "/auth/reset");
  assert.equal(safeNextPath("/auth/reset?fresh=1", "/"), "/auth/reset?fresh=1");
  assert.equal(safeNextPath("", "/auth/reset"), "/auth/reset");
  assert.equal(safeNextPath(null, "/"), "/");
  assert.equal(safeNextPath("//evil.example", "/"), "/");
  assert.equal(safeNextPath("https://evil.example", "/"), "/");
  assert.equal(safeNextPath("/\\evil.example", "/"), "/");
  assert.equal(safeNextPath("/%2F%2Fevil.example", "/"), "/");
  assert.equal(safeNextPath("/auth/reset\nSet-Cookie:x", "/"), "/");
});
