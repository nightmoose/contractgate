import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import {
  callbackRedirectUrl,
  defaultNext,
  failurePath,
  isEmailOtpType,
  safeInternalPath,
  safeNextPath,
  sameOriginRedirect,
  SIGNUP_NEXT_METADATA_KEY,
  signupUserMetadata,
  withAuthReturn,
} from "./auth-email-link";

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
  assert.equal(safeNextPath("/catalog/stripe-charges/fork", "/"), "/catalog/stripe-charges/fork");
  assert.equal(safeNextPath("", "/auth/reset"), "/auth/reset");
  assert.equal(safeNextPath(null, "/"), "/");
  assert.equal(safeNextPath("//evil.example", "/"), "/");
  assert.equal(safeNextPath("https://evil.example", "/"), "/");
  assert.equal(safeNextPath("/\\evil.example", "/"), "/");
  assert.equal(safeNextPath("/%2F%2Fevil.example", "/"), "/");
  assert.equal(safeNextPath("/auth/reset\nSet-Cookie:x", "/"), "/");
  assert.equal(safeNextPath("/%0a/catalog/stripe-charges/fork", "/"), "/");
  // Confirmation template percent-encodes the href. One decode is the fork path.
  assert.equal(
    safeNextPath("%2fcatalog%2fstripe-charges%2ffork", "/"),
    "/catalog/stripe-charges/fork",
  );
  assert.equal(safeNextPath("https%3a%2f%2fevil.example", "/"), "/");
});

test("safeInternalPath stays on the app when the URL parser would change host", () => {
  assert.equal(safeInternalPath("/catalog/stripe-charges/fork", "/"), "/catalog/stripe-charges/fork");
  assert.equal(safeInternalPath("/\\evil.example", "/"), "/");
  assert.equal(safeInternalPath("//evil.example", "/"), "/");
  assert.equal(
    sameOriginRedirect("https://app.datacontractgate.com", "/catalog/stripe-charges/fork"),
    "https://app.datacontractgate.com/catalog/stripe-charges/fork",
  );
  assert.equal(
    sameOriginRedirect("https://app.datacontractgate.com", "//evil.example"),
    "https://app.datacontractgate.com/",
  );
  assert.equal(
    sameOriginRedirect("https://app.datacontractgate.com", "/\\evil.example"),
    "https://app.datacontractgate.com/",
  );
  assert.equal(
    sameOriginRedirect("https://app.datacontractgate.com", "https://evil.example/phish"),
    "https://app.datacontractgate.com/",
  );
});

test("auth links keep the catalog return path and the AISchemaGate product signal", () => {
  assert.equal(
    withAuthReturn("/auth/login", "/catalog/stripe-charges/fork", null),
    "/auth/login?next=%2Fcatalog%2Fstripe-charges%2Ffork",
  );
  assert.equal(
    withAuthReturn("/auth/signup", "/catalog/stripe-charges/fork", "aischemagate"),
    "/auth/signup?next=%2Fcatalog%2Fstripe-charges%2Ffork&product=aischemagate",
  );
  assert.equal(withAuthReturn("/auth/login", "https://evil.example", "nope"), "/auth/login");
  assert.equal(
    withAuthReturn("/auth/login", "//evil.example", "aischemagate"),
    "/auth/login?product=aischemagate",
  );
  assert.equal(
    callbackRedirectUrl(
      "https://app.datacontractgate.com",
      "/catalog/stripe-charges/fork",
      "aischemagate",
    ),
    "https://app.datacontractgate.com/auth/callback?next=%2Fcatalog%2Fstripe-charges%2Ffork&product=aischemagate",
  );
  assert.equal(
    callbackRedirectUrl("https://app.datacontractgate.com", "https://evil.example", "other"),
    "https://app.datacontractgate.com/auth/callback?next=%2F",
  );
});

test("signup metadata stores the fork path and drops an external next", () => {
  assert.deepEqual(
    signupUserMetadata({
      displayName: "",
      email: "ada@example.com",
      next: "/catalog/stripe-charges/fork",
      product: "aischemagate",
    }),
    {
      display_name: "ada",
      [SIGNUP_NEXT_METADATA_KEY]: "/catalog/stripe-charges/fork",
      product: "aischemagate",
    },
  );
  const external = signupUserMetadata({
    displayName: "Ada",
    email: "ada@example.com",
    next: "https://evil.example",
    product: "https://evil.example",
  });
  assert.equal(external.display_name, "Ada");
  assert.equal(external[SIGNUP_NEXT_METADATA_KEY], undefined);
  assert.equal(external.product, undefined);
  assert.deepEqual(
    signupUserMetadata({
      displayName: "Ada",
      email: "ada@example.com",
      next: "/",
      product: null,
    }),
    { display_name: "Ada" },
  );
});

test("confirmation template carries signup_next instead of a hardcoded next=/", () => {
  const templatePath = join(
    dirname(fileURLToPath(import.meta.url)),
    "../../supabase/templates/confirmation.html",
  );
  const template = readFileSync(templatePath, "utf8");
  assert.equal(template.includes("{{ .TokenHash }}"), true);
  assert.equal(template.includes(".Data.signup_next"), true);
  assert.equal(template.includes('eq .Data.product "aischemagate"'), true);
  assert.equal(template.includes("{{ .ConfirmationURL }}"), false);
  assert.equal(template.includes('&type=signup&next=/"'), false);
});
