/**
 * Unauthenticated website smoke — no credentials required.
 */
import { test, expect } from "@playwright/test";

test.describe("Public website surfaces", () => {
  test("home / dashboard shell loads", async ({ page }) => {
    const res = await page.goto("/");
    expect(res?.ok() || res?.status() === 200 || res?.status() === 307).toBeTruthy();
    // Either marketing home, app shell, or login redirect is fine
    await expect(page.locator("body")).toBeVisible();
    const title = await page.title();
    expect(title.length).toBeGreaterThan(0);
  });

  test("login page renders email + password + GitHub", async ({ page }) => {
    await page.goto("/auth/login");
    await expect(page.getByRole("heading", { name: /sign in/i })).toBeVisible();
    await expect(page.getByLabel(/email/i).or(page.locator('input[type="email"]'))).toBeVisible();
    await expect(page.locator('input[type="password"]')).toBeVisible();
    await expect(page.getByRole("button", { name: /github/i })).toBeVisible();
  });

  test("signup page loads", async ({ page }) => {
    const res = await page.goto("/auth/signup");
    expect(res?.status()).toBeLessThan(500);
    await expect(page.locator("body")).toBeVisible();
  });

  test("pricing page loads", async ({ page }) => {
    const res = await page.goto("/pricing");
    expect(res?.status()).toBeLessThan(500);
    // Free / Growth / Enterprise copy or price anchors
    const body = await page.locator("body").innerText();
    expect(body.toLowerCase()).toMatch(/free|growth|enterprise|pricing|plan/);
  });

  test("stream demo is publicly reachable", async ({ page }) => {
    const res = await page.goto("/stream-demo");
    expect(res?.status()).toBeLessThan(500);
    await expect(page.locator("body")).toBeVisible();
    // Should not hard-error
    const body = await page.locator("body").innerText();
    expect(body.toLowerCase()).not.toMatch(/internal server error|application error/);
  });

  test("docs surface loads", async ({ page }) => {
    const res = await page.goto("/docs");
    expect(res?.status()).toBeLessThan(500);
    await expect(page.locator("body")).toBeVisible();
  });

  test("privacy + terms load", async ({ page }) => {
    for (const path of ["/privacy", "/terms"]) {
      const res = await page.goto(path);
      expect(res?.status(), path).toBeLessThan(500);
    }
  });

  test("protected routes redirect unauthenticated users to login", async ({ page }) => {
    for (const path of ["/contracts", "/playground", "/audit", "/scorecard"]) {
      await page.goto(path);
      // Auth gate may redirect immediately or show a login wall
      await page.waitForTimeout(1500);
      const url = page.url();
      const body = (await page.locator("body").innerText()).toLowerCase();
      const gated =
        /\/auth\/login/.test(url) ||
        body.includes("sign in") ||
        body.includes("log in") ||
        body.includes("github");
      expect(gated, `${path} should require auth (url=${url})`).toBeTruthy();
    }
  });
});

test.describe("Marketing site", () => {
  test("datacontractgate.com marketing home", async ({ browser }) => {
    const context = await browser.newContext();
    const page = await context.newPage();
    const res = await page.goto("https://datacontractgate.com/", {
      waitUntil: "domcontentloaded",
    });
    // 200 or 308→200
    expect(res?.status()).toBeLessThan(400);
    await expect(page.locator("body")).toBeVisible();
    await context.close();
  });
});
