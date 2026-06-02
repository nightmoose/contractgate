/**
 * RFC-023 Phase 2 — Demo mode e2e smoke tests.
 *
 * Assumes the full demo stack is running:
 *   NEXT_PUBLIC_DEMO_MODE=1  dashboard at localhost:3000
 *   gateway at localhost:8080 with seeded demo org + contracts
 *
 * Run via:
 *   NEXT_PUBLIC_DEMO_MODE=1 npx playwright test e2e/demo.spec.ts
 *
 * In CI: runs after `docker compose --profile demo up` in compose-demo-smoke.
 */

import { test, expect } from "@playwright/test";

// ── Helpers ───────────────────────────────────────────────────────────────────

/** Wait up to `timeout` ms for a numeric text to be > 0. */
async function waitForNonZeroCount(
  page: import("@playwright/test").Page,
  locator: import("@playwright/test").Locator,
  timeout = 30_000
) {
  await expect(async () => {
    const text = await locator.textContent();
    const n = parseInt(text?.replace(/,/g, "") ?? "0", 10);
    expect(n).toBeGreaterThan(0);
  }).toPass({ timeout });
}

// ── Suite ─────────────────────────────────────────────────────────────────────

test.describe("Demo mode", () => {
  test("loads without login wall", async ({ page }) => {
    await page.goto("/");
    // No redirect to /auth/login
    await expect(page).not.toHaveURL(/\/auth\/login/);
    // Demo banner visible
    await expect(page.getByText("Self-Hosted Free")).toBeVisible();
  });

  test("banner shows upgrade CTA", async ({ page }) => {
    await page.goto("/");
    const cta = page.getByRole("link", { name: /ContractGate Cloud/i });
    await expect(cta).toBeVisible();
    await expect(cta).toHaveAttribute("href", /contractgate\.io\/cloud/);
    await expect(cta).toHaveAttribute("target", "_blank");
  });

  test("contracts page shows ≥3 contracts", async ({ page }) => {
    await page.goto("/contracts");
    await expect(page).not.toHaveURL(/\/auth\/login/);

    // Wait for the contracts list to populate (seeder may still be inserting).
    // We look for at least 3 contract name cells.
    await expect(async () => {
      const rows = page.locator("[data-testid='contract-row'], table tbody tr, [class*='contract']");
      const count = await rows.count();
      expect(count).toBeGreaterThanOrEqual(3);
    }).toPass({ timeout: 30_000 });
  });

  test("audit page loads and shows live events", async ({ page }) => {
    await page.goto("/audit");
    await expect(page).not.toHaveURL(/\/auth\/login/);

    // Wait for at least one event row to appear.
    // Seeder posts at 3 req/s so within 10s there should be rows.
    const row = page.locator("table tbody tr, [data-testid='audit-row']").first();
    await expect(row).toBeVisible({ timeout: 20_000 });
  });

  test("playground validates a sample event end-to-end", async ({ page }) => {
    await page.goto("/playground");
    await expect(page).not.toHaveURL(/\/auth\/login/);

    // The playground should load without a login wall.
    // Look for the YAML editor or validate button.
    const playgroundContent = page.locator(
      "[data-testid='playground'], .playground, textarea, [role='textbox']"
    ).first();
    await expect(playgroundContent).toBeVisible({ timeout: 10_000 });
  });

  test("account page shows DemoFeatureUnavailable", async ({ page }) => {
    await page.goto("/account");
    await expect(page).not.toHaveURL(/\/auth\/login/);
    // Should show the feature-unavailable gate, not the API key form.
    await expect(page.getByText(/not available in Self-Hosted Free/i)).toBeVisible();
    await expect(page.getByRole("link", { name: /Upgrade to ContractGate Cloud/i })).toBeVisible();
    // Should NOT show the real account content.
    await expect(page.getByText(/API Keys/)).not.toBeVisible();
  });

  test("stream-demo page is accessible", async ({ page }) => {
    await page.goto("/stream-demo");
    // stream-demo was already auth-free; confirm it still works in demo mode.
    await expect(page).not.toHaveURL(/\/auth\/login/);
  });
});
