/**
 * Phone-width layout: no route should scroll horizontally at 375px.
 * Inner code blocks may scroll; the document must not.
 */

import { test, expect } from "@playwright/test";

const ROUTES: { path: string; title: string }[] = [
  { path: "/", title: "Dashboard — DataContractGate" },
  { path: "/pricing", title: "Pricing — DataContractGate" },
  { path: "/docs", title: "Docs — DataContractGate" },
  { path: "/docs/kafka-connect", title: "Kafka Connect — DataContractGate" },
  { path: "/docs/python-sdk", title: "Python SDK — DataContractGate" },
  { path: "/docs/fivetran", title: "Fivetran — DataContractGate" },
  { path: "/auth/login", title: "Sign in — DataContractGate" },
  { path: "/auth/signup", title: "Sign up — DataContractGate" },
  { path: "/auth/forgot", title: "Reset password — DataContractGate" },
  { path: "/auth/reset", title: "Set a new password — DataContractGate" },
  { path: "/auth/accept-invite", title: "Sign in — DataContractGate" },
  { path: "/playground", title: "Playground — DataContractGate" },
  { path: "/contracts", title: "Contracts — DataContractGate" },
  { path: "/audit", title: "Audit log — DataContractGate" },
  // Middleware sends anonymous visitors to sign-in. The route still has its own
  // metadata for a signed-in session; this check is the anonymous phone view.
  { path: "/catalog", title: "Sign in — DataContractGate" },
  { path: "/scorecard", title: "Sign in — DataContractGate" },
  { path: "/scaffold", title: "Sign in — DataContractGate" },
  { path: "/workbench", title: "Sign in — DataContractGate" },
  { path: "/account", title: "Sign in — DataContractGate" },
  { path: "/stream-demo", title: "Stream demo — DataContractGate" },
  { path: "/privacy", title: "Privacy policy — DataContractGate" },
  { path: "/terms", title: "Terms of service — DataContractGate" },
  { path: "/billing/success", title: "Sign in — DataContractGate" },
];

test.describe("375px horizontal overflow", () => {
  test.use({ viewport: { width: 375, height: 812 } });

  for (const route of ROUTES) {
    test(`${route.path} does not overflow`, async ({ page }) => {
      const response = await page.goto(route.path, { waitUntil: "load" });
      expect(response, route.path).not.toBeNull();
      expect(response!.status(), route.path).toBeLessThan(500);
      await page
        .waitForFunction(() => !document.querySelector(".animate-spin"), null, { timeout: 8000 })
        .catch(() => {});

      if (["/", "/playground", "/contracts", "/audit"].includes(route.path)) {
        await expect(page.locator("h1").first()).toBeAttached();
      }
      await expect(page).toHaveTitle(route.title);
      const description = await page.locator('meta[name="description"]').getAttribute("content");
      expect(description, route.path).toBeTruthy();
      expect(description, route.path).not.toMatch(/\bContractGate\b/);

      const overflow = await page.evaluate(() => {
        const doc = document.documentElement;
        return doc.scrollWidth - doc.clientWidth;
      });
      expect(overflow, `${route.path} overflows by ${overflow}px`).toBeLessThanOrEqual(1);
    });
  }

  test("home Sign in is not covered by the help button", async ({ page }) => {
    await page.goto("/", { waitUntil: "load" });
    await page.waitForFunction(() => !document.querySelector(".animate-spin"));
    const signIn = page.getByRole("link", { name: "Sign in", exact: true });
    const help = page.getByRole("button", { name: "What’s this?" });
    await expect(signIn).toBeVisible();
    await expect(help).toBeVisible();

    const overlap = await page.evaluate(() => {
      const link = [...document.querySelectorAll("a")].find((a) => a.textContent?.trim() === "Sign in");
      const button = document.querySelector("[aria-label='What’s this?']");
      if (!link || !button) return null;
      const a = link.getBoundingClientRect();
      const b = button.getBoundingClientRect();
      const x = Math.max(0, Math.min(a.right, b.right) - Math.max(a.left, b.left));
      const y = Math.max(0, Math.min(a.bottom, b.bottom) - Math.max(a.top, b.top));
      return x * y;
    });
    expect(overlap).toBe(0);
  });

  test("the menu button opens the sidebar drawer", async ({ page }) => {
    await page.goto("/pricing", { waitUntil: "load" });
    const menu = page.getByRole("button", { name: "Open menu" });
    await menu.click();
    const sidebar = page.locator("#app-sidebar");
    await expect(sidebar).toBeVisible();
    await expect(sidebar.getByRole("link", { name: "Contracts" })).toBeVisible();
    await page.getByRole("button", { name: "Close menu" }).click();
    await expect(sidebar).toBeHidden();
  });
});
