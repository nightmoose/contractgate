import { defineConfig, devices } from "@playwright/test";

/**
 * Production / staging website dogfood.
 *
 *   cd ops/dogfood
 *   npm install
 *   npx playwright install chromium
 *   npx playwright test --config browser/playwright.config.ts
 *
 * Auth (optional for full product surfaces):
 *   CG_EMAIL=… CG_PASSWORD=… npx playwright test --config browser/playwright.config.ts
 *
 * API key (optional, for API suite):
 *   CG_API_KEY=cg_live_… npx playwright test browser/api.spec.ts
 */

const baseURL =
  process.env.CG_DASHBOARD_URL?.replace(/\/$/, "") ||
  "https://app.datacontractgate.com";

export default defineConfig({
  testDir: ".",
  testMatch: /.*\.spec\.ts/,
  fullyParallel: false,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 1 : 0,
  workers: 1,
  timeout: 90_000,
  expect: { timeout: 15_000 },
  reporter: [
    ["list"],
    ["json", { outputFile: "../findings/runs/browser-last.json" }],
    ["html", { open: "never", outputFolder: "../findings/runs/browser-report" }],
  ],
  use: {
    baseURL,
    trace: "retain-on-failure",
    screenshot: "only-on-failure",
    video: "retain-on-failure",
    actionTimeout: 20_000,
    navigationTimeout: 45_000,
  },
  projects: [
    {
      name: "chromium",
      use: { ...devices["Desktop Chrome"] },
    },
  ],
  outputDir: "../findings/runs/browser-artifacts",
});
