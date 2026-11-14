import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import {
  decodeVisitorPath,
  liveDemoHref,
  loggedOutAuthHrefs,
  VISITOR_AUTH_HEADER,
  VISITOR_PATH_HEADER,
} from "./logged-out-cta";

const here = dirname(fileURLToPath(import.meta.url));

test("sign-up and sign-in carry next back to the page the visitor is on", () => {
  assert.deepEqual(loggedOutAuthHrefs("/catalog", null), {
    signUpHref: "/auth/signup?next=%2Fcatalog",
    signInHref: "/auth/login?next=%2Fcatalog",
  });
  assert.deepEqual(loggedOutAuthHrefs("/catalog/stripe-charges", "aischemagate"), {
    signUpHref: "/auth/signup?next=%2Fcatalog%2Fstripe-charges&product=aischemagate",
    signInHref: "/auth/login?next=%2Fcatalog%2Fstripe-charges&product=aischemagate",
  });
  assert.deepEqual(loggedOutAuthHrefs("/contracts", null), {
    signUpHref: "/auth/signup?next=%2Fcontracts",
    signInHref: "/auth/login?next=%2Fcontracts",
  });
  assert.deepEqual(loggedOutAuthHrefs("/playground?contract=user_events", null), {
    signUpHref: "/auth/signup?next=%2Fplayground%3Fcontract%3Duser_events",
    signInHref: "/auth/login?next=%2Fplayground%3Fcontract%3Duser_events",
  });
});

test("the home page omits next because login already returns there", () => {
  assert.deepEqual(loggedOutAuthHrefs("/", null), {
    signUpHref: "/auth/signup",
    signInHref: "/auth/login",
  });
  assert.deepEqual(loggedOutAuthHrefs("", null), {
    signUpHref: "/auth/signup",
    signInHref: "/auth/login",
  });
});

test("auth pages keep an existing next instead of returning to the form", () => {
  assert.deepEqual(
    loggedOutAuthHrefs("/auth/login?next=%2Fcatalog%2Fstripe-charges%2Ffork", null),
    {
      signUpHref: "/auth/signup?next=%2Fcatalog%2Fstripe-charges%2Ffork",
      signInHref: "/auth/login?next=%2Fcatalog%2Fstripe-charges%2Ffork",
    },
  );
  assert.deepEqual(loggedOutAuthHrefs("/auth/signup", "aischemagate"), {
    signUpHref: "/auth/signup?product=aischemagate",
    signInHref: "/auth/login?product=aischemagate",
  });
  assert.deepEqual(loggedOutAuthHrefs("/auth/login?next=https://evil.example", null), {
    signUpHref: "/auth/signup",
    signInHref: "/auth/login",
  });
});

test("visitor path header round-trips and rejects an off-site value", () => {
  assert.equal(VISITOR_PATH_HEADER, "x-dcg-pathname");
  assert.equal(VISITOR_AUTH_HEADER, "x-dcg-auth");
  assert.equal(decodeVisitorPath(encodeURI("/catalog/stripe-charges?ref=1")), "/catalog/stripe-charges?ref=1");
  assert.equal(decodeVisitorPath(null), "/");
  assert.equal(decodeVisitorPath("//evil.example"), "/");
  assert.equal(decodeVisitorPath("https://evil.example"), "/");
});

test("public catalog pages stay server-rendered and do not mount AuthGate", () => {
  for (const rel of ["app/catalog/page.tsx", "app/catalog/[slug]/page.tsx"]) {
    const source = readFileSync(join(here, "../", rel), "utf8");
    assert.equal(/import\s+AuthGate\b/.test(source), false, rel);
    assert.equal(/<AuthGate[\s>]/.test(source), false, rel);
    assert.equal(/^["']use client["']/m.test(source), false, rel);
  }
});

test("the live demo link points at the playground and is omitted on that page", () => {
  assert.equal(liveDemoHref("/contracts"), "/playground");
  assert.equal(liveDemoHref("/audit?range=24h"), "/playground");
  assert.equal(liveDemoHref("/playground"), null);
  assert.equal(liveDemoHref("/playground?contract=user_events"), null);
  assert.equal(liveDemoHref("/playground/"), null);
  const gate = readFileSync(join(here, "../components/AuthGate.tsx"), "utf8");
  assert.equal(gate.includes('href="/playground"'), false);
  assert.equal(gate.includes("liveDemoHref"), true);
  const playground = readFileSync(join(here, "../app/playground/page.tsx"), "utf8");
  assert.equal(/<AuthGate[\s>]/.test(playground), false);
  assert.equal(playground.includes("publicPlaygroundValidate"), true);
});

test("the root layout renders the logged-out header from the server session", () => {
  const layout = readFileSync(join(here, "../app/layout.tsx"), "utf8");
  assert.equal(layout.includes("LoggedOutHeader"), true);
  assert.equal(layout.includes("resolveVisitor"), true);
  const proxy = readFileSync(join(here, "../proxy.ts"), "utf8");
  assert.equal(proxy.includes("VISITOR_AUTH_HEADER"), true);
  assert.equal(proxy.includes("VISITOR_PATH_HEADER"), true);
  assert.equal(proxy.includes('"/catalog"'), true);
});
